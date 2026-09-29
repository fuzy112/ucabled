// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-user session agent: registers an agent with the system daemon and
//! shows the QR window on request.
//!
//! It only registers when it has a graphical session; whether it is *allowed*
//! to register is decided by the daemon through polkit, so an agent belonging
//! to a background (non-active) session is refused and exits.

use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::blocking::Connection;
use dbus::channel::MatchingReceiver;
use dbus::message::MatchRule;
use dbus_crossroads::Crossroads;

use ucabled::agent::{AGENT_INTERFACE, BUS_NAME, ERR_NOT_AUTHORIZED, UI_INTERFACE, UI_PATH};

const AGENT_PATH: &str = "/org/ucabled/Agent";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";

type ChildSlot = Arc<Mutex<Option<Child>>>;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        tracing::info!("no graphical session, not registering an agent");
        return Ok(());
    }

    let child: ChildSlot = Arc::new(Mutex::new(None));
    let conn = Connection::new_system().context("connect system bus")?;

    // Helper exits are reported to the serving loop, which owns `conn` and is
    // therefore the only thread allowed to send over it.
    let (cancel_tx, cancel_rx) = mpsc::channel::<u64>();

    let mut crossroads = Crossroads::new();
    let iface = crossroads.register(AGENT_INTERFACE, {
        let child = child.clone();
        let cancel_tx = cancel_tx.clone();
        move |builder| {
            let prompt_child = child.clone();
            let prompt_cancel = cancel_tx.clone();
            builder.method(
                "Prompt",
                ("tid", "url", "rp", "timeout"),
                (),
                move |_ctx, _: &mut (), (tid, url, rp, timeout): (u64, String, String, u64)| {
                    let rp = (!rp.is_empty()).then_some(rp);
                    show_window(&prompt_child, &prompt_cancel, tid, &url, rp, timeout);
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
    crossroads.insert(AGENT_PATH, &[iface], ());

    // Serve the agent and report cancellations on the very same
    // connection, so the daemon always sees the sender's unique name.
    conn.start_receive(
        MatchRule::new_method_call(),
        Box::new(move |msg, conn| {
            crossroads.handle_message(msg, conn).unwrap();
            true
        }),
    );

    // Re-register whenever the daemon (re)appears: a restart clears its
    // in-memory agent slot, and only this signal tells us about it.
    let reregister = Arc::new(AtomicBool::new(false));
    {
        let reregister = reregister.clone();
        conn.add_match(
            MatchRule::new_signal(DBUS_INTERFACE, "NameOwnerChanged"),
            move |(name, _old_owner, new_owner): (String, String, String), _conn, _msg| {
                if name == BUS_NAME && !new_owner.is_empty() {
                    tracing::info!("daemon appeared on the bus, re-registering");
                    reregister.store(true, Ordering::SeqCst);
                }
                true
            },
        )?;
    }

    // Register with the daemon, retrying while it is not up yet.
    if !register_blocking(&conn) {
        tracing::info!("registration refused; this session is not the active one");
        return Ok(());
    }

    tracing::info!("agent registered at {AGENT_PATH}");

    loop {
        conn.process(Duration::from_millis(200))?;
        while let Ok(tid) = cancel_rx.try_recv() {
            notify_cancelled(&conn, tid);
        }
        if reregister.swap(false, Ordering::SeqCst) && !register_once(&conn).unwrap_or(false) {
            child.lock().unwrap().take();
        }
    }
}

fn helper_path() -> Option<std::path::PathBuf> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("ucable-agent-helper")))?;
    sibling.exists().then_some(sibling)
}

fn show_window(
    state: &ChildSlot,
    cancel_tx: &mpsc::Sender<u64>,
    tid: u64,
    url: &str,
    rp: Option<String>,
    timeout_secs: u64,
) {
    kill_child(state);

    let Some(helper) = helper_path() else {
        tracing::warn!("ucable-agent-helper not found next to the agent; cannot show the QR code");
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
            tracing::warn!("failed to spawn ucable-agent-helper: {e}");
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
    let cancel_tx = cancel_tx.clone();
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
            let _ = cancel_tx.send(tid);
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

/// One registration attempt over `conn` (which must be the connection serving
/// the agent object). `Ok(true)` = registered, `Ok(false)` = refused
/// (not the active session), `Err` = transient.
fn register_once(conn: &Connection) -> Result<bool> {
    let proxy = conn.with_proxy(BUS_NAME, UI_PATH, Duration::from_secs(5));
    let result: std::result::Result<(), dbus::Error> = proxy.method_call(
        UI_INTERFACE,
        "RegisterAgent",
        (dbus::Path::from(AGENT_PATH),),
    );
    match result {
        Ok(()) => Ok(true),
        Err(e) if e.name().map(|n| n.to_string()) == Some(ERR_NOT_AUTHORIZED.to_string()) => {
            Ok(false)
        }
        Err(e) => Err(e).context("RegisterAgent"),
    }
}

/// Retry registration until it succeeds or is definitively refused.
fn register_blocking(conn: &Connection) -> bool {
    loop {
        match register_once(conn) {
            Ok(true) => return true,
            Ok(false) => return false,
            Err(e) => {
                tracing::info!("daemon not ready ({e:#}), retrying");
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// Report the cancellation over `conn`: the daemon only accepts it from the
/// currently registered agent's unique name.
fn notify_cancelled(conn: &Connection, tid: u64) {
    let result = (|| -> Result<()> {
        let proxy = conn.with_proxy(BUS_NAME, UI_PATH, Duration::from_secs(5));
        let _: () = proxy.method_call(UI_INTERFACE, "TransactionCancelled", (tid,))?;
        Ok(())
    })();
    if let Err(e) = result {
        tracing::warn!("failed to report cancellation: {e:#}");
    }
}
