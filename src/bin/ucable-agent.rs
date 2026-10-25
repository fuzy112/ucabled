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
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver as MessageReceiver;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::mpsc::{self, UnboundedSender};

use ucabled::agent::{
    AGENT_INTERFACE, BUS_NAME, ERR_NOT_AUTHORIZED, MANAGER_INTERFACE, MANAGER_PATH,
};

const AGENT_PATH: &str = "/org/ucabled/Agent";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";
/// Unique sender of driver-generated signals; the same string as the
/// interface by convention, but used here as the match's sender filter.
const DBUS_DRIVER: &str = "org.freedesktop.DBus";

/// A live helper window: its pid (to tell it apart when it exits), the
/// transaction it belongs to, and a queue of stdin lines. The helper process
/// itself is owned by its window task, so killing is delegated to that task
/// through `kill` rather than signalled by pid from here.
struct LiveWindow {
    pid: u32,
    tid: u64,
    lines: UnboundedSender<String>,
    kill: UnboundedSender<()>,
}

type WindowSlot = Arc<Mutex<Option<LiveWindow>>>;

#[tokio::main]
async fn main() -> Result<()> {
    // The agent takes no options; every argument is handled or rejected.
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!(
                    "ucable-agent {} — per-user session agent for ucabled\n\
                     \n\
                     Registers with the system daemon over D-Bus and shows the\n\
                     QR window on request. Normally started by systemd as a user\n\
                     service (ucable-agent.service); takes no options.\n\
                     \n\
                     Environment:\n\
                     \x20 UCABLED_HELPER  UI helper to spawn (absolute path or a name\n\
                     \x20                 resolved via PATH); defaults to the bundled\n\
                     \x20                 ucable-agent-helper\n\
                     \n\
                     Options:\n\
                     \x20 -h, --help     print this help\n\
                     \x20 -V, --version  print the version",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "-V" | "--version" => {
                println!("ucable-agent {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other => {
                eprintln!("unknown argument: {other} (try --help)");
                std::process::exit(2);
            }
        }
    }

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
    // The daemon (re)appeared on the bus; registration must be renewed.
    let (daemon_tx, mut daemon_rx) = mpsc::unbounded_channel::<()>();

    let qr_window: WindowSlot = Arc::new(Mutex::new(None));
    // Unique name of the process that currently owns org.ucabled.  The bus
    // policy already denies unprivileged sends to the agent's unique name, so
    // this is defence in depth: it keeps a future policy change (or a bus
    // without the shipped configuration) from handing the daemon's authority
    // to another local process.  It is filled in after the NameOwnerChanged
    // match below is installed, so a restart cannot fall between the query
    // and the match.
    let daemon_owner: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    let crossroads = build_crossroads(&qr_window, cancel_tx, &daemon_owner);

    let method = conn
        .add_match(MatchRule::new_method_call())
        .await
        .context("match method calls")?;
    let signal = conn
        // Only the bus driver may report name changes: the stock system-bus
        // policy lets any local user emit signals, so without this sender
        // restriction a local process could forge a NameOwnerChanged for
        // org.ucabled, clear (or replace) the cached owner, and durably
        // silence the UI.
        .add_match(
            MatchRule::new_signal(DBUS_INTERFACE, "NameOwnerChanged").with_sender(DBUS_DRIVER),
        )
        .await
        .context("match NameOwnerChanged")?;
    let (_method_match, methods) = method.msg_stream();
    let (_signal_match, signals) = signal.msg_stream();
    // Query the owner only after the match exists: a daemon restart between
    // an earlier query and `add_match` would otherwise be missed, leaving the
    // agent with a dead unique name as its sender filter and with every
    // Prompt/Found/Close call from the new daemon rejected.
    let owner = current_daemon_owner(&conn).await;
    *daemon_owner.lock().unwrap() = owner;
    tokio::spawn(route_messages(
        conn.clone(),
        crossroads,
        methods,
        signals,
        daemon_tx,
        daemon_owner,
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
            Some(()) = daemon_rx.recv() => {
                // A daemon restart clears its in-memory agent slot. Any
                // failure (refused or transient) drops our windows, like
                // before; the next appearance signal starts a new attempt.
                if !matches!(register_once(&conn).await, Ok(true)) {
                    close_window(&qr_window);
                }
            }
        }
    }
}

fn build_crossroads(
    qr_window: &WindowSlot,
    cancel_tx: UnboundedSender<u64>,
    daemon_owner: &Arc<Mutex<Option<String>>>,
) -> Crossroads {
    let mut crossroads = Crossroads::new();
    let iface = crossroads.register(AGENT_INTERFACE, {
        let qr_window = qr_window.clone();
        let prompt_owner = daemon_owner.clone();
        let found_owner = daemon_owner.clone();
        let close_owner = daemon_owner.clone();
        let notify_owner = daemon_owner.clone();
        move |builder| {
            let prompt_window = qr_window.clone();
            builder.method(
                "Prompt",
                ("tid", "url", "rp", "timeout", "request_type"),
                (),
                move |ctx,
                      _: &mut (),
                      (tid, url, rp, timeout, request_type): (
                    u64,
                    String,
                    String,
                    u64,
                    String,
                )| {
                    if !from_daemon(ctx, &prompt_owner) {
                        return Ok(());
                    }
                    let rp = (!rp.is_empty()).then_some(rp);
                    show_window(
                        &prompt_window,
                        &cancel_tx,
                        tid,
                        &url,
                        rp,
                        timeout,
                        &request_type,
                    );
                    Ok(())
                },
            );

            let found_window = qr_window.clone();
            builder.method(
                "Found",
                ("tid",),
                (),
                move |ctx, _: &mut (), (tid,): (u64,)| {
                    if !from_daemon(ctx, &found_owner) {
                        return Ok(());
                    }
                    forward_line_for_tid(&found_window, tid, "found\n");
                    Ok(())
                },
            );

            let close_qr = qr_window.clone();
            builder.method(
                "Close",
                ("tid",),
                (),
                move |ctx, _: &mut (), (tid,): (u64,)| {
                    if !from_daemon(ctx, &close_owner) {
                        return Ok(());
                    }
                    close_window_for_tid(&close_qr, tid);
                    Ok(())
                },
            );

            builder.method(
                "Notify",
                ("summary", "body"),
                (),
                move |ctx, _: &mut (), (summary, body): (String, String)| {
                    if !from_daemon(ctx, &notify_owner) {
                        return Ok(());
                    }
                    tokio::spawn(desktop_notify(summary, body));
                    Ok(())
                },
            );
        }
    });
    crossroads.insert(AGENT_PATH, &[iface], ());
    crossroads
}

/// Whether `ctx`'s caller is the current owner of `org.ucabled`.
///
/// A missing caller or an unknown owner is rejected: an agent that has not
/// seen the daemon's name yet must not act on anybody's request.
fn from_daemon(ctx: &dbus_crossroads::Context, owner: &Mutex<Option<String>>) -> bool {
    let owner = owner.lock().unwrap();
    match (ctx.message().sender(), owner.as_deref()) {
        (Some(sender), Some(owner)) => sender == owner,
        _ => {
            tracing::warn!(
                sender = ?ctx.message().sender(),
                "ignoring agent call that does not come from the daemon"
            );
            false
        }
    }
}

/// Ask the bus who currently owns `org.ucabled`; `None` when the name is not
/// owned (yet) or the bus cannot tell us.
async fn current_daemon_owner(conn: &Arc<SyncConnection>) -> Option<String> {
    let proxy = Proxy::new(
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        Duration::from_secs(5),
        conn.clone(),
    );
    let result: std::result::Result<(String,), dbus::Error> = proxy
        .method_call("org.freedesktop.DBus", "GetNameOwner", (BUS_NAME,))
        .await;
    match result {
        Ok((owner,)) => Some(owner),
        Err(e) => {
            tracing::debug!("org.ucabled has no owner yet: {e}");
            None
        }
    }
}

/// The single task owning the crossroads: serves agent method calls and
/// notices when the daemon (re)appears on the bus.
async fn route_messages(
    conn: Arc<SyncConnection>,
    mut crossroads: Crossroads,
    mut methods: MessageReceiver<Message>,
    mut signals: MessageReceiver<Message>,
    daemon_tx: UnboundedSender<()>,
    daemon_owner: Arc<Mutex<Option<String>>>,
) {
    loop {
        tokio::select! {
            msg = methods.next() => {
                let Some(msg) = msg else { break };
                let _ = crossroads.handle_message(msg, &*conn);
            }
            sig = signals.next() => {
                let Some(sig) = sig else { break };
                if let Ok((name, _old_owner, new_owner)) = sig.read3::<String, String, String>()
                    && name == BUS_NAME {
                        let appeared = !new_owner.is_empty();
                        *daemon_owner.lock().unwrap() =
                            appeared.then(|| new_owner.clone());
                        if appeared {
                            tracing::info!("daemon appeared on the bus, re-registering");
                            let _ = daemon_tx.send(());
                        }
                    }
            }
        }
    }
}

/// Locate the UI helper executable.
///
/// `UCABLED_HELPER` overrides the bundled helper. If it is an absolute
/// path, it is used only when it exists; a missing path logs a warning and
/// falls through. A bare name is returned as-is so `Command` resolves it via
/// `PATH`. Without the variable, the default is the `ucable-agent-helper`
/// sibling of the agent binary.
fn helper_path() -> Option<std::path::PathBuf> {
    if let Some(value) = std::env::var_os("UCABLED_HELPER") {
        let path = std::path::PathBuf::from(value);
        if path.as_os_str().is_empty() {
            tracing::warn!("UCABLED_HELPER is empty; using the bundled helper");
        } else if path.is_absolute() {
            if path.exists() {
                return Some(path);
            }
            tracing::warn!(
                "UCABLED_HELPER={} does not exist; using the bundled helper",
                path.display()
            );
        } else {
            return Some(path);
        }
    }
    let sibling = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("ucable-agent-helper")))?;
    sibling.exists().then_some(sibling)
}

/// Kill the live window's helper, if any, by asking its window task to. That
/// task owns the `Child`, so it kills a process it has not reaped yet and the
/// signal can never hit a recycled pid; it then stays silent because the slot
/// no longer holds its pid.
fn close_window(slot: &WindowSlot) {
    if let Some(window) = slot.lock().unwrap().take() {
        drop(window.lines);
        let _ = window.kill.send(());
    }
}

/// Like `close_window`, but only when the live window belongs to `tid`. A
/// Close (or any tid-carrying call) for a finished transaction must not kill
/// the window of a newer one.
fn close_window_for_tid(slot: &WindowSlot, tid: u64) {
    let window = {
        let mut guard = slot.lock().unwrap();
        match guard.as_ref() {
            Some(window) if window.tid == tid => guard.take(),
            _ => None,
        }
    };
    if let Some(window) = window {
        drop(window.lines);
        let _ = window.kill.send(());
    }
}

/// Forward a stdin line to the live window, but only when it belongs to
/// `tid`: a Found for a finished transaction must not leak into the window
/// of a newer one. Returns whether the line was forwarded.
fn forward_line_for_tid(slot: &WindowSlot, tid: u64, line: &str) -> bool {
    let guard = slot.lock().unwrap();
    match guard.as_ref() {
        Some(window) if window.tid == tid => {
            let _ = window.lines.send(line.to_string());
            true
        }
        _ => false,
    }
}

/// Spawn `command`, queue `initial` as its first stdin line, and run
/// `on_exit` with the exit status — but only if the window is still the
/// live one; a Close or a newer window kills the helper and suppresses the
/// report. Returns false when the helper could not be spawned.
fn open_window(
    slot: &WindowSlot,
    tid: u64,
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
        // A child that has no pid is already reaped, but make sure it is not
        // left running before giving up on the window.
        tracing::warn!("helper has no pid");
        let _ = child.start_kill();
        return false;
    };

    let (lines, mut lines_rx) = mpsc::unbounded_channel::<String>();
    if let Some(initial) = initial {
        let _ = lines.send(initial);
    }
    let (kill, mut kill_rx) = mpsc::unbounded_channel::<()>();
    *slot.lock().unwrap() = Some(LiveWindow {
        pid,
        tid,
        lines,
        kill,
    });

    let slot = slot.clone();
    tokio::spawn(async move {
        let mut stdin = child.stdin.take();
        let mut lines_open = true;
        let mut kill_open = true;
        let mut kill_requested = false;
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
                _ = kill_rx.recv(), if kill_open => {
                    kill_open = false;
                    kill_requested = true;
                }
            }
            // Run the kill after `select!` has released its borrow of `child`,
            // so it is this task — which still owns the unreaped process — that
            // signals it.  The slot is already empty, so no exit is reported.
            if kill_requested {
                kill_requested = false;
                let _ = child.start_kill();
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
        if still_live && let Ok(status) = status {
            on_exit(status);
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
    request_type: &str,
) {
    let Some(helper) = helper_path() else {
        tracing::warn!("ucable-agent-helper not found next to the agent; cannot show the QR code");
        return;
    };
    let mut command = Command::new(helper);
    if let Some(rp) = &rp {
        // A valued option, never a bare positional: the RP comes from the
        // CTAP payload and may begin with '-', which a positional slot would
        // hand to the helper's flag parser.
        command.arg("--rp").arg(rp);
    }
    command
        .arg("--timeout")
        .arg(timeout_secs.to_string())
        .stdin(Stdio::piped());
    if !request_type.is_empty() {
        // Says which command is being relayed so the helper can phrase the QR
        // page; a helper that does not know the flag shows a plain caption.
        command.arg("--request-type").arg(request_type);
    }
    // The URL carries the transaction secret, so it goes over the pipe.
    let cancel_tx = cancel_tx.clone();
    open_window(
        state,
        tid,
        &mut command,
        Some(format!("{url}\n")),
        move |_status| {
            let _ = cancel_tx.send(tid);
        },
    );
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
            (
                "ucabled",
                0u32,
                "",
                summary,
                body,
                Vec::<String>::new(),
                hints,
                -1i32,
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn closing_a_window_kills_the_helper_without_reporting_it() {
        let slot: WindowSlot = Arc::new(Mutex::new(None));
        let reported = Arc::new(AtomicBool::new(false));
        let reported_in_cb = reported.clone();
        let mut command = Command::new("sleep");
        command.arg("30").stdin(Stdio::piped());
        assert!(open_window(&slot, 1, &mut command, None, move |_| {
            reported_in_cb.store(true, Ordering::SeqCst);
        }));

        let pid = slot
            .lock()
            .unwrap()
            .as_ref()
            .expect("the window is live")
            .pid;
        assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());

        close_window(&slot);

        // The window task reaps the helper, so its /proc entry disappears.
        let mut gone = false;
        for _ in 0..100 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(gone, "helper {pid} was not killed");
        assert!(slot.lock().unwrap().is_none());
        assert!(
            !reported.load(Ordering::SeqCst),
            "a window killed by close_window must not report an exit"
        );
    }

    async fn wait_for_death(pid: u32) -> bool {
        for _ in 0..100 {
            if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn a_close_for_another_tid_leaves_the_window_alone() {
        let slot: WindowSlot = Arc::new(Mutex::new(None));
        let mut command = Command::new("sleep");
        command.arg("30").stdin(Stdio::piped());
        // The live window belongs to transaction 2.
        assert!(open_window(&slot, 2, &mut command, None, |_| {}));
        let pid = slot
            .lock()
            .unwrap()
            .as_ref()
            .expect("the window is live")
            .pid;

        // A Close for the older transaction 1 must not kill it.
        close_window_for_tid(&slot, 1);
        assert!(slot.lock().unwrap().is_some());
        assert!(std::path::Path::new(&format!("/proc/{pid}")).exists());

        // A Close carrying the window's own tid does.
        close_window_for_tid(&slot, 2);
        assert!(slot.lock().unwrap().is_none());
        assert!(wait_for_death(pid).await, "helper {pid} was not killed");
    }

    #[tokio::test]
    async fn lines_are_forwarded_only_to_the_owning_transaction() {
        let slot: WindowSlot = Arc::new(Mutex::new(None));
        let mut command = Command::new("sleep");
        command.arg("30").stdin(Stdio::piped());
        assert!(open_window(&slot, 2, &mut command, None, |_| {}));
        let pid = slot.lock().unwrap().as_ref().unwrap().pid;

        assert!(forward_line_for_tid(&slot, 2, "found\n"));
        assert!(!forward_line_for_tid(&slot, 1, "found\n"));

        // No window at all: nothing to forward to.
        close_window(&slot);
        assert!(!forward_line_for_tid(&slot, 2, "found\n"));
        assert!(wait_for_death(pid).await, "helper {pid} was not killed");
    }
}
