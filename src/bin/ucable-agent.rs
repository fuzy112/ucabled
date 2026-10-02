// SPDX-License-Identifier: GPL-3.0-or-later

//! Per-user session agent: registers an agent with the system daemon and
//! shows the QR window on request.
//!
//! It only registers when it has a graphical session; whether it is *allowed*
//! to register is decided by the daemon through polkit, so an agent belonging
//! to a background (non-active) session is refused and exits.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::arg::{RefArg, Variant};
use dbus::message::{MatchRule, Message};
use dbus::nonblock::{Proxy, SyncConnection};
use dbus_crossroads::Crossroads;
use futures::channel::mpsc::UnboundedReceiver as MessageReceiver;
use futures::StreamExt;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc::{self, UnboundedSender};

use ucabled::agent::{
    AGENT_INTERFACE, BUS_NAME, ERR_NOT_AUTHORIZED, MANAGER_INTERFACE, MANAGER_PATH,
};

const AGENT_PATH: &str = "/org/ucabled/Agent";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";
/// How long the device-selection window stays up before declining.
const SELECT_TIMEOUT_SECS: u64 = 60;

/// A live helper window: its pid (for killing) and a queue of stdin lines.
/// The helper process itself is owned by its window task.
struct LiveWindow {
    pid: u32,
    lines: UnboundedSender<String>,
}

type WindowSlot = Arc<Mutex<Option<LiveWindow>>>;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    if std::env::var_os("WAYLAND_DISPLAY").is_none() && std::env::var_os("DISPLAY").is_none() {
        tracing::info!("no graphical session, not registering an agent");
        return Ok(());
    }

    let (resource, conn) =
        dbus_tokio::connection::new_system_sync().context("connect system bus")?;
    tokio::spawn(async move {
        let err = resource.await;
        tracing::error!("system bus connection lost: {err}");
    });

    // Helper exits are reported to the main loop, which is the only task
    // calling back into the daemon.
    let (cancel_tx, mut cancel_rx) = mpsc::unbounded_channel::<u64>();
    // (tid, use_phone) from the device-selection window.
    let (select_tx, mut select_rx) = mpsc::unbounded_channel::<(u64, bool)>();
    // The daemon (re)appeared on the bus; registration must be renewed.
    let (daemon_tx, mut daemon_rx) = mpsc::unbounded_channel::<()>();

    let qr_window: WindowSlot = Arc::new(Mutex::new(None));
    let select_window: WindowSlot = Arc::new(Mutex::new(None));

    let crossroads = build_crossroads(&qr_window, &select_window, cancel_tx, select_tx);

    let method = conn
        .add_match(MatchRule::new_method_call())
        .await
        .context("match method calls")?;
    let signal = conn
        .add_match(MatchRule::new_signal(DBUS_INTERFACE, "NameOwnerChanged"))
        .await
        .context("match NameOwnerChanged")?;
    let (_method_match, methods) = method.msg_stream();
    let (_signal_match, signals) = signal.msg_stream();
    tokio::spawn(route_messages(
        conn.clone(),
        crossroads,
        methods,
        signals,
        daemon_tx,
    ));

    // Register with the daemon, retrying while it is not up yet.
    loop {
        match register_once(&conn).await {
            Ok(true) => break,
            Ok(false) => {
                tracing::info!("registration refused; this session is not the active one");
                return Ok(());
            }
            Err(e) => {
                tracing::info!("daemon not ready ({e:#}), retrying");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
    tracing::info!("agent registered at {AGENT_PATH}");

    loop {
        tokio::select! {
            Some(tid) = cancel_rx.recv() => {
                notify_cancelled(&conn, tid).await;
            }
            Some((tid, use_phone)) = select_rx.recv() => {
                notify_selection(&conn, tid, use_phone).await;
            }
            Some(()) = daemon_rx.recv() => {
                // A daemon restart clears its in-memory agent slot. Any
                // failure (refused or transient) drops our windows, like
                // before; the next appearance signal starts a new attempt.
                if !matches!(register_once(&conn).await, Ok(true)) {
                    close_window(&qr_window);
                    close_window(&select_window);
                }
            }
        }
    }
}

fn build_crossroads(
    qr_window: &WindowSlot,
    select_window: &WindowSlot,
    cancel_tx: UnboundedSender<u64>,
    select_tx: UnboundedSender<(u64, bool)>,
) -> Crossroads {
    let mut crossroads = Crossroads::new();
    let iface = crossroads.register(AGENT_INTERFACE, {
        let qr_window = qr_window.clone();
        let select_window = select_window.clone();
        move |builder| {
            let prompt_window = qr_window.clone();
            builder.method(
                "Prompt",
                ("tid", "url", "rp", "timeout"),
                (),
                move |_ctx, _: &mut (), (tid, url, rp, timeout): (u64, String, String, u64)| {
                    let rp = (!rp.is_empty()).then_some(rp);
                    show_window(&prompt_window, &cancel_tx, tid, &url, rp, timeout);
                    Ok(())
                },
            );

            let found_window = qr_window.clone();
            builder.method(
                "Found",
                ("tid",),
                (),
                move |_ctx, _: &mut (), (_tid,): (u64,)| {
                    if let Some(window) = found_window.lock().unwrap().as_ref() {
                        let _ = window.lines.send("found\n".to_string());
                    }
                    Ok(())
                },
            );

            let ask_window = select_window.clone();
            let select_tx_slot = select_tx.clone();
            builder.method(
                "Select",
                ("tid",),
                (),
                move |_ctx, _: &mut (), (tid,): (u64,)| {
                    show_select_window(&ask_window, &select_tx_slot, tid);
                    Ok(())
                },
            );

            let close_qr = qr_window.clone();
            let close_select = select_window.clone();
            builder.method(
                "Close",
                ("tid",),
                (),
                move |_ctx, _: &mut (), (_tid,): (u64,)| {
                    close_window(&close_qr);
                    close_window(&close_select);
                    Ok(())
                },
            );

            builder.method(
                "Notify",
                ("summary", "body"),
                (),
                move |_ctx, _: &mut (), (summary, body): (String, String)| {
                    tokio::spawn(desktop_notify(summary, body));
                    Ok(())
                },
            );
        }
    });
    crossroads.insert(AGENT_PATH, &[iface], ());
    crossroads
}

/// The single task owning the crossroads: serves agent method calls and
/// notices when the daemon (re)appears on the bus.
async fn route_messages(
    conn: Arc<SyncConnection>,
    mut crossroads: Crossroads,
    mut methods: MessageReceiver<Message>,
    mut signals: MessageReceiver<Message>,
    daemon_tx: UnboundedSender<()>,
) {
    loop {
        tokio::select! {
            msg = methods.next() => {
                let Some(msg) = msg else { break };
                let _ = crossroads.handle_message(msg, &*conn);
            }
            sig = signals.next() => {
                let Some(sig) = sig else { break };
                if let Ok((name, _old_owner, new_owner)) = sig.read3::<String, String, String>() {
                    if name == BUS_NAME && !new_owner.is_empty() {
                        tracing::info!("daemon appeared on the bus, re-registering");
                        let _ = daemon_tx.send(());
                    }
                }
            }
        }
    }
}

fn helper_path() -> Option<std::path::PathBuf> {
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("ucable-agent-helper")))?;
    sibling.exists().then_some(sibling)
}

/// Kill the live window's helper, if any. The window task reaps it through
/// its pending `wait()` and stays silent because the slot no longer holds
/// its pid.
fn close_window(slot: &WindowSlot) {
    if let Some(window) = slot.lock().unwrap().take() {
        drop(window.lines);
        // Safe: the pid belongs to our own not-yet-reaped child (the window
        // task reaps it), so it cannot have been recycled.
        unsafe { libc::kill(window.pid as i32, libc::SIGKILL) };
    }
}

/// Spawn `command`, queue `initial` as its first stdin line, and run
/// `on_exit` with the exit status — but only if the window is still the
/// live one; a Close or a newer window kills the helper and suppresses the
/// report. Returns false when the helper could not be spawned.
fn open_window(
    slot: &WindowSlot,
    command: &mut Command,
    initial: Option<String>,
    on_exit: impl FnOnce(std::process::ExitStatus) + Send + 'static,
) -> bool {
    close_window(slot);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            tracing::warn!("failed to spawn ucable-agent-helper: {e}");
            return false;
        }
    };
    let Some(pid) = child.id() else {
        tracing::warn!("helper has no pid");
        return false;
    };

    let (lines, mut lines_rx) = mpsc::unbounded_channel::<String>();
    if let Some(initial) = initial {
        let _ = lines.send(initial);
    }
    *slot.lock().unwrap() = Some(LiveWindow { pid, lines });

    let slot = slot.clone();
    tokio::spawn(async move {
        let mut stdin = child.stdin.take();
        let mut lines_open = true;
        let status = loop {
            tokio::select! {
                status = child.wait() => break status,
                line = lines_rx.recv(), if lines_open => {
                    match line {
                        Some(line) => {
                            if let Some(stdin) = stdin.as_mut() {
                                let _ = stdin.write_all(line.as_bytes()).await;
                                let _ = stdin.flush().await;
                            }
                        }
                        None => lines_open = false,
                    }
                }
            }
        };
        let still_live = {
            let mut guard = slot.lock().unwrap();
            match guard.as_ref() {
                Some(window) if window.pid == pid => {
                    *guard = None;
                    true
                }
                _ => false,
            }
        };
        if still_live {
            if let Ok(status) = status {
                on_exit(status);
            }
        }
    });
    true
}

fn show_window(
    state: &WindowSlot,
    cancel_tx: &UnboundedSender<u64>,
    tid: u64,
    url: &str,
    rp: Option<String>,
    timeout_secs: u64,
) {
    let Some(helper) = helper_path() else {
        tracing::warn!("ucable-agent-helper not found next to the agent; cannot show the QR code");
        return;
    };
    let mut command = Command::new(helper);
    if let Some(rp) = &rp {
        command.arg(rp);
    }
    command
        .arg("--timeout")
        .arg(timeout_secs.to_string())
        .stdin(Stdio::piped());
    // The URL carries the transaction secret, so it goes over the pipe.
    let cancel_tx = cancel_tx.clone();
    open_window(
        state,
        &mut command,
        Some(format!("{url}\n")),
        move |_status| {
            let _ = cancel_tx.send(tid);
        },
    );
}

/// Show the "another security key is present, use the phone instead?" window
/// and report the user's answer. The helper exits 0 for "Use phone" and
/// non-zero for cancel/close/timeout.
fn show_select_window(state: &WindowSlot, result_tx: &UnboundedSender<(u64, bool)>, tid: u64) {
    let Some(helper) = helper_path() else {
        tracing::warn!("ucable-agent-helper not found next to the agent; cannot ask the user");
        let _ = result_tx.send((tid, false));
        return;
    };
    let mut command = Command::new(helper);
    command
        .arg("--select")
        .arg("--timeout")
        .arg(SELECT_TIMEOUT_SECS.to_string())
        .stdin(Stdio::null());
    let result_tx = result_tx.clone();
    let decline_tx = result_tx.clone();
    let opened = open_window(state, &mut command, None, move |status| {
        // Exit code 0 means "Use phone"; anything else (cancel, close,
        // timeout, signal) declines and lets the other key win.
        let _ = result_tx.send((tid, status.code() == Some(0)));
    });
    if !opened {
        let _ = decline_tx.send((tid, false));
    }
}

/// Show a desktop notification through the session bus.  Best-effort: a
/// missing session bus or notification service only costs a log line.
/// A fresh connection per call — notifications are rare (currently only a
/// refused tunnel redirect).
async fn desktop_notify(summary: String, body: String) {
    let (resource, conn) = match dbus_tokio::connection::new_session_sync() {
        Ok(pair) => pair,
        Err(e) => {
            tracing::warn!("cannot reach the session bus for a notification: {e}");
            return;
        }
    };
    tokio::spawn(async move {
        let err = resource.await;
        tracing::warn!("session bus connection lost: {err}");
    });
    let proxy = Proxy::new(
        "org.freedesktop.Notifications",
        "/org/freedesktop/Notifications",
        Duration::from_secs(5),
        conn,
    );
    let hints: HashMap<String, Variant<Box<dyn RefArg>>> = HashMap::new();
    let result: std::result::Result<(u32,), dbus::Error> = proxy
        .method_call(
            "org.freedesktop.Notifications",
            "Notify",
            ("ucabled", 0u32, "", summary, body, Vec::<String>::new(), hints, -1i32),
        )
        .await;
    if let Err(e) = result {
        tracing::warn!("desktop notification failed: {e}");
    }
}

/// One registration attempt over `conn` (which must be the connection serving
/// the agent object). `Ok(true)` = registered, `Ok(false)` = refused
/// (not the active session), `Err` = transient.
async fn register_once(conn: &Arc<SyncConnection>) -> Result<bool> {
    let proxy = Proxy::new(BUS_NAME, MANAGER_PATH, Duration::from_secs(5), conn.clone());
    let result: std::result::Result<(), dbus::Error> = proxy
        .method_call(
            MANAGER_INTERFACE,
            "RegisterAgent",
            (dbus::Path::from(AGENT_PATH),),
        )
        .await;
    match result {
        Ok(()) => Ok(true),
        Err(e) if e.name() == Some(ERR_NOT_AUTHORIZED) => Ok(false),
        Err(e) => Err(e).context("RegisterAgent"),
    }
}

/// Report the cancellation over `conn`: the daemon only accepts it from the
/// currently registered agent's unique name.
async fn notify_cancelled(conn: &Arc<SyncConnection>, tid: u64) {
    let result = async {
        let proxy = Proxy::new(BUS_NAME, MANAGER_PATH, Duration::from_secs(5), conn.clone());
        let _: () = proxy
            .method_call(MANAGER_INTERFACE, "TransactionCancelled", (tid,))
            .await?;
        Ok::<(), dbus::Error>(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("failed to report cancellation: {e:#}");
    }
}

/// Report the user's device-selection answer over `conn`.
async fn notify_selection(conn: &Arc<SyncConnection>, tid: u64, use_phone: bool) {
    let result = async {
        let proxy = Proxy::new(BUS_NAME, MANAGER_PATH, Duration::from_secs(5), conn.clone());
        let _: () = proxy
            .method_call(MANAGER_INTERFACE, "SelectionResult", (tid, use_phone))
            .await?;
        Ok::<(), dbus::Error>(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!("failed to report device selection: {e:#}");
    }
}
