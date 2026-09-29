//! Per-user session agent: registers a prompter with the system daemon and
//! shows the QR window on request.
//!
//! It only registers when it has a graphical session; whether it is *allowed*
//! to register is decided by the daemon through polkit, so an agent belonging
//! to a background (non-active) session is refused and exits.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::blocking::Connection;
use dbus_crossroads::Crossroads;

use ucabled::prompter::{BUS_NAME, ERR_NOT_AUTHORIZED, PROMPTER_INTERFACE, UI_INTERFACE, UI_PATH};

const PROMPTER_PATH: &str = "/org/ucabled/Prompter";
const REREGISTER_INTERVAL: Duration = Duration::from_secs(10);

type ChildSlot = Arc<Mutex<Option<Child>>>;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        tracing::info!("no graphical session, not registering a prompter");
        return Ok(());
    }

    let child: ChildSlot = Arc::new(Mutex::new(None));
    let conn = Connection::new_system().context("connect system bus")?;

    let mut crossroads = Crossroads::new();
    let iface = crossroads.register(PROMPTER_INTERFACE, {
        let child = child.clone();
        move |builder| {
            let prompt_child = child.clone();
            builder.method(
                "Prompt",
                ("tid", "url", "rp", "timeout"),
                (),
                move |_ctx, _: &mut (), (tid, url, rp, timeout): (u64, String, String, u64)| {
                    let rp = (!rp.is_empty()).then_some(rp);
                    show_window(&prompt_child, tid, &url, rp, timeout);
                    Ok(())
                },
            );

            let found_child = child.clone();
            builder.method(
                "Found",
                ("tid",),
                (),
                move |_ctx, _: &mut (), (_tid,): (u64,)| {
                    if let Some(child) = found_child.lock().unwrap().as_mut() {
                        if let Some(stdin) = child.stdin.as_mut() {
                            let _ = stdin.write_all(b"found\n");
                            let _ = stdin.flush();
                        }
                    }
                    Ok(())
                },
            );

            let close_child = child.clone();
            builder.method(
                "Close",
                ("tid",),
                (),
                move |_ctx, _: &mut (), (_tid,): (u64,)| {
                    kill_child(&close_child);
                    Ok(())
                },
            );
        }
    });
    crossroads.insert(PROMPTER_PATH, &[iface], ());

    // Register with the daemon, retrying while it is not up yet.
    if !register_blocking() {
        tracing::info!("registration refused; this session is not the active one");
        return Ok(());
    }

    // The daemon can be restarted; re-register periodically (idempotent).
    let keepalive_child = child.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(REREGISTER_INTERVAL);
        if !register_once().unwrap_or(false) {
            keepalive_child.lock().unwrap().take();
        }
    });

    tracing::info!("prompter registered at {PROMPTER_PATH}");
    crossroads.serve(&conn)?;
    Ok(())
}

fn helper_path() -> Option<std::path::PathBuf> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("ucabled-qr")))?;
    sibling.exists().then_some(sibling)
}

fn show_window(state: &ChildSlot, tid: u64, url: &str, rp: Option<String>, timeout_secs: u64) {
    kill_child(state);

    let Some(helper) = helper_path() else {
        tracing::warn!("ucabled-qr not found next to the agent; cannot show the QR code");
        return;
    };
    let mut command = Command::new(helper);
    if let Some(rp) = &rp {
        command.arg(rp);
    }
    command.arg("--timeout").arg(timeout_secs.to_string());
    command.stdin(Stdio::piped());
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!("failed to spawn ucabled-qr: {e}");
            return;
        }
    };
    // The URL carries the transaction secret, so it goes over the pipe.
    if let Some(stdin) = child.stdin.as_mut() {
        let sent = stdin
            .write_all(url.as_bytes())
            .and_then(|_| stdin.write_all(b"\n"))
            .and_then(|_| stdin.flush());
        if let Err(e) = sent {
            tracing::warn!("failed to send QR code to helper: {e}");
        }
    }
    *state.lock().unwrap() = Some(child);

    let state = state.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(150));
        let exited = {
            let mut guard = state.lock().unwrap();
            match guard.as_mut() {
                None => return,
                Some(child) => match child.try_wait() {
                    Ok(Some(_)) => {
                        *guard = None;
                        true
                    }
                    Ok(None) => false,
                    Err(_) => {
                        *guard = None;
                        return;
                    }
                },
            }
        };
        if exited {
            notify_cancelled(tid);
            return;
        }
    });
}

fn kill_child(state: &ChildSlot) {
    if let Some(mut child) = state.lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// One registration attempt. `Ok(true)` = registered, `Ok(false)` = refused
/// (not the active session), `Err` = transient.
fn register_once() -> Result<bool> {
    let conn = Connection::new_system().context("connect system bus")?;
    let proxy = conn.with_proxy(BUS_NAME, UI_PATH, Duration::from_secs(5));
    let result: std::result::Result<(), dbus::Error> = proxy.method_call(
        UI_INTERFACE,
        "RegisterPrompter",
        (dbus::Path::from(PROMPTER_PATH),),
    );
    match result {
        Ok(()) => Ok(true),
        Err(e) if e.name().map(|n| n.to_string()) == Some(ERR_NOT_AUTHORIZED.to_string()) => {
            Ok(false)
        }
        Err(e) => Err(e).context("RegisterPrompter"),
    }
}

/// Retry registration until it succeeds or is definitively refused.
fn register_blocking() -> bool {
    loop {
        match register_once() {
            Ok(true) => return true,
            Ok(false) => return false,
            Err(e) => {
                tracing::info!("daemon not ready ({e:#}), retrying");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

fn notify_cancelled(tid: u64) {
    let result = (|| -> Result<()> {
        let conn = Connection::new_system()?;
        let proxy = conn.with_proxy(BUS_NAME, UI_PATH, Duration::from_secs(5));
        let _: () = proxy.method_call(UI_INTERFACE, "TransactionCancelled", (tid,))?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!("failed to report cancellation: {e:#}");
    }
}
