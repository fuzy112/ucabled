// SPDX-License-Identifier: GPL-3.0-or-later

//! D-Bus bridge between the system daemon and the per-user session agent.
//!
//! The daemon owns the system-bus name `org.ucabled` and exposes
//! `org.ucabled.Manager1`; a session agent registers an `org.ucabled.Agent1`
//! object and is then called back to show the QR window.
//!
//! Registration is authorized through polkit (action
//! `org.ucabled.register-agent`, `allow_active=yes`), so only the user of
//! the current active local session can register. The daemon passes the
//! caller's unique bus name as a `system-bus-name` subject and re-checks on
//! every prompt, so an agent left over from a previous session is ignored.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::Connection;
use dbus::message::{MatchRule, Message};
use dbus::nonblock::{MsgMatch, Proxy, SyncConnection};
use dbus::Path;
use dbus_crossroads::{Crossroads, MethodErr};
use futures::channel::mpsc::UnboundedReceiver as MessageReceiver;
use futures::StreamExt;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub const BUS_NAME: &str = "org.ucabled";
pub const MANAGER_PATH: &str = "/org/ucabled/Manager";
pub const MANAGER_INTERFACE: &str = "org.ucabled.Manager1";
pub const AGENT_INTERFACE: &str = "org.ucabled.Agent1";
pub const POLKIT_ACTION: &str = "org.ucabled.register-agent";
/// Returned to an agent whose user is not the active local session user; the
/// agent treats this as final and exits instead of retrying.
pub const ERR_NOT_AUTHORIZED: &str = "org.ucabled.NotAuthorized";

const POLKIT_BUS: &str = "org.freedesktop.PolicyKit1";
const POLKIT_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const POLKIT_INTERFACE: &str = "org.freedesktop.PolicyKit1.Authority";
const DBUS_BUS: &str = "org.freedesktop.DBus";
const DBUS_PATH: &str = "/org/freedesktop/DBus";
const DBUS_INTERFACE: &str = "org.freedesktop.DBus";

/// A registered UI agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// Unique bus name of the agent (`:1.x`), used as call destination.
    pub destination: String,
    /// Object path of its `org.ucabled.Agent1` implementation.
    pub path: String,
    pub uid: u32,
}

/// The single agent the daemon currently talks to.
#[derive(Default)]
pub struct AgentSlot {
    current: Option<Agent>,
}

impl AgentSlot {
    pub fn current(&self) -> Option<&Agent> {
        self.current.as_ref()
    }
}

#[derive(Debug, Clone)]
pub enum AgentCommand {
    Prompt {
        tid: u64,
        url: String,
        rp: Option<String>,
        timeout_secs: u64,
    },
    /// Ask the user to pick the phone when another authenticator is present.
    Select {
        tid: u64,
    },
    Found {
        tid: u64,
    },
    Close {
        tid: u64,
    },
}

/// Daemon-side handle to the UI bridge. Cheap to clone and safe to use from
/// any task; commands are handed to a dedicated D-Bus dispatch task.
#[derive(Clone)]
pub struct AgentClient {
    tx: UnboundedSender<AgentCommand>,
    slot: Arc<Mutex<AgentSlot>>,
    active_tid: Arc<AtomicU64>,
}

impl AgentClient {
    /// Whether an authorized agent is currently registered.
    pub fn available(&self) -> bool {
        self.slot.lock().unwrap().current.is_some()
    }

    pub fn prompt(&self, tid: u64, url: &str, rp: Option<String>, timeout_secs: u64) {
        self.active_tid.store(tid, Ordering::SeqCst);
        let _ = self.tx.send(AgentCommand::Prompt {
            tid,
            url: url.to_string(),
            rp,
            timeout_secs,
        });
    }

    /// Ask the agent to show the "use the phone instead?" prompt. The answer
    /// comes back through the `select` channel passed to [`start`].
    pub fn select(&self, tid: u64) {
        let _ = self.tx.send(AgentCommand::Select { tid });
    }

    /// The phone's BLE advert was seen; the user now confirms on it.
    pub fn found(&self) {
        let tid = self.active_tid.load(Ordering::SeqCst);
        let _ = self.tx.send(AgentCommand::Found { tid });
    }

    /// The transaction ended (or was cancelled); tear the window down.
    pub fn close(&self) {
        let tid = self.active_tid.load(Ordering::SeqCst);
        let _ = self.tx.send(AgentCommand::Close { tid });
    }
}

/// Start the D-Bus service and dispatch tasks on `handle`. `cancel_tx`
/// receives the transaction id when the agent reports that the user cancelled
/// the dialog; `select_tx` receives the user's answer (`true` = use the
/// phone) to a [`AgentClient::select`] request.
///
/// The setup (connect, request the bus name, install matches) runs inline so
/// a failure still falls back at startup; message routing and outbound calls
/// then live in spawned tasks on a single async connection.
pub fn start(
    handle: &Handle,
    cancel_tx: mpsc::Sender<u64>,
    select_tx: mpsc::Sender<(u64, bool)>,
) -> Result<AgentClient> {
    let slot = Arc::new(Mutex::new(AgentSlot::default()));
    let (tx, rx) = mpsc::unbounded_channel();

    let (resource, conn) =
        dbus_tokio::connection::new_system_sync().context("connect system bus")?;
    handle.spawn(async move {
        let err = resource.await;
        tracing::error!("system bus connection lost: {err}");
    });

    let crossroads = build_crossroads(&slot, cancel_tx, select_tx);
    let registered = handle.block_on(register_on_bus(&conn))?;
    let ((method_match, methods), (signal_match, signals)) = registered;

    handle.spawn(route_messages(
        conn.clone(),
        crossroads,
        method_match,
        methods,
        signal_match,
        signals,
        slot.clone(),
    ));
    handle.spawn(dispatch_commands(conn, rx, slot.clone()));

    Ok(AgentClient {
        tx,
        slot,
        active_tid: Arc::new(AtomicU64::new(0)),
    })
}

/// Check whether `sender` may register an agent, returning its uid.
///
/// Opens its own blocking bus connection: this runs inside the message
/// routing task and only on registration (a rare, locally answered call),
/// so briefly parking the task there is acceptable.
fn authorize(sender: &str) -> Result<Option<u32>> {
    let conn = Connection::new_system().context("connect system bus")?;

    let mut details: PropMap = PropMap::new();
    let value: Box<dyn RefArg + 'static> = Box::new(sender.to_string());
    details.insert("name".to_string(), Variant(value));
    let subject = ("system-bus-name".to_string(), details);

    let proxy = conn.with_proxy(POLKIT_BUS, POLKIT_PATH, Duration::from_secs(25));
    // The result is a single struct argument `(b b a{ss})`, so it must be
    // read as a one-element tuple wrapping the struct.
    let ((authorized, _challenge, _details),): ((bool, bool, HashMap<String, String>),) = proxy
        .method_call(
            POLKIT_INTERFACE,
            "CheckAuthorization",
            (
                subject,
                POLKIT_ACTION,
                HashMap::<String, String>::new(),
                0u32,
                "",
            ),
        )
        .context("polkit CheckAuthorization")?;

    if !authorized {
        return Ok(None);
    }

    let dbus = conn.with_proxy(DBUS_BUS, DBUS_PATH, Duration::from_secs(5));
    let (uid,): (u32,) = dbus
        .method_call(DBUS_INTERFACE, "GetConnectionUnixUser", (sender,))
        .unwrap_or((u32::MAX,));
    Ok(Some(uid))
}

type MatchStream = (MsgMatch, MessageReceiver<Message>);

/// Request the bus name and install the method-call and NameOwnerChanged
/// matches, returning the guards (dropping them stops matching) and streams.
async fn register_on_bus(conn: &Arc<SyncConnection>) -> Result<(MatchStream, MatchStream)> {
    conn.request_name(BUS_NAME, false, false, false)
        .await
        .with_context(|| format!("request bus name {BUS_NAME}"))?;
    let method = conn
        .add_match(MatchRule::new_method_call())
        .await
        .context("match method calls")?;
    let signal = conn
        .add_match(MatchRule::new_signal(DBUS_INTERFACE, "NameOwnerChanged").with_sender(DBUS_BUS))
        .await
        .context("match NameOwnerChanged")?;
    tracing::info!("listening on {BUS_NAME} ({MANAGER_INTERFACE})");
    Ok((method.msg_stream(), signal.msg_stream()))
}

fn build_crossroads(
    slot: &Arc<Mutex<AgentSlot>>,
    cancel_tx: mpsc::Sender<u64>,
    select_tx: mpsc::Sender<(u64, bool)>,
) -> Crossroads {
    let mut crossroads = Crossroads::new();
    let iface = crossroads.register(MANAGER_INTERFACE, {
        let slot = slot.clone();
        move |builder| {
            let register_slot = slot.clone();
            builder.method(
                "RegisterAgent",
                ("path",),
                (),
                move |ctx, _: &mut (), (path,): (Path<'static>,)| {
                    let sender = ctx
                        .message()
                        .sender()
                        .ok_or_else(|| MethodErr::failed(&"missing sender"))?
                        .to_string();
                    let authorized = authorize(&sender).map_err(|e| {
                        tracing::warn!("polkit check failed: {e:#}");
                        MethodErr::failed(&"authorization check failed")
                    })?;
                    let Some(uid) = authorized else {
                        tracing::warn!(%sender, "agent registration denied by polkit");
                        return Err(MethodErr::from((
                            ERR_NOT_AUTHORIZED,
                            "only the active local session may register an agent",
                        )));
                    };
                    tracing::info!(%sender, %path, uid, "agent registered");
                    register_slot.lock().unwrap().current = Some(Agent {
                        destination: sender,
                        path: path.to_string(),
                        uid,
                    });
                    Ok(())
                },
            );

            let unregister_slot = slot.clone();
            builder.method("UnregisterAgent", (), (), move |ctx, _: &mut (), ()| {
                let sender = ctx.message().sender().map(|s| s.to_string());
                clear_if_matches(&unregister_slot, sender.as_deref());
                Ok(())
            });

            let cancel_slot = slot.clone();
            builder.method(
                "TransactionCancelled",
                ("tid",),
                (),
                move |ctx, _: &mut (), (tid,): (u64,)| {
                    let sender = ctx.message().sender().map(|s| s.to_string());
                    let is_current = {
                        let guard = cancel_slot.lock().unwrap();
                        guard
                            .current()
                            .map(|p| Some(p.destination.as_str()) == sender.as_deref())
                            .unwrap_or(false)
                    };
                    if is_current {
                        tracing::info!(tid, "agent reported cancellation");
                        // Bounded queue: a flooded or stale cancellation is
                        // dropped, never retained.
                        let _ = cancel_tx.try_send(tid);
                    }
                    Ok(())
                },
            );

            let select_slot = slot.clone();
            builder.method(
                "SelectionResult",
                ("tid", "use_phone"),
                (),
                move |ctx, _: &mut (), (tid, use_phone): (u64, bool)| {
                    let sender = ctx.message().sender().map(|s| s.to_string());
                    let is_current = {
                        let guard = select_slot.lock().unwrap();
                        guard
                            .current()
                            .map(|p| Some(p.destination.as_str()) == sender.as_deref())
                            .unwrap_or(false)
                    };
                    if is_current {
                        tracing::info!(tid, use_phone, "agent reported device selection");
                        // Bounded queue, as for cancellations.
                        let _ = select_tx.try_send((tid, use_phone));
                    }
                    Ok(())
                },
            );
        }
    });
    crossroads.insert(MANAGER_PATH, &[iface], ());
    crossroads
}

/// The single task that owns the crossroads: feeds it incoming method calls
/// and drops a stale agent the moment its connection goes away (the bus emits
/// NameOwnerChanged(name=":1.x", new_owner="") on disconnect).
async fn route_messages(
    conn: Arc<SyncConnection>,
    mut crossroads: Crossroads,
    _method_match: MsgMatch,
    mut methods: MessageReceiver<Message>,
    _signal_match: MsgMatch,
    mut signals: MessageReceiver<Message>,
    slot: Arc<Mutex<AgentSlot>>,
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
                    if new_owner.is_empty() {
                        clear_if_matches(&slot, Some(&name));
                    }
                }
            }
        }
    }
}

async fn dispatch_commands(
    conn: Arc<SyncConnection>,
    mut rx: UnboundedReceiver<AgentCommand>,
    slot: Arc<Mutex<AgentSlot>>,
) {
    while let Some(command) = rx.recv().await {
        // The lock is released before the await: only the cloned Agent
        // crosses it.
        let Some(agent) = slot.lock().unwrap().current().cloned() else {
            tracing::debug!("no agent registered, dropping UI command");
            continue;
        };
        let proxy = Proxy::new(
            agent.destination.as_str(),
            agent.path.as_str(),
            Duration::from_secs(15),
            conn.clone(),
        );
        let result: std::result::Result<(), dbus::Error> = match command {
            AgentCommand::Prompt {
                tid,
                url,
                rp,
                timeout_secs,
            } => {
                proxy
                    .method_call(
                        AGENT_INTERFACE,
                        "Prompt",
                        (tid, url, rp.unwrap_or_default(), timeout_secs),
                    )
                    .await
            }
            AgentCommand::Select { tid } => {
                proxy.method_call(AGENT_INTERFACE, "Select", (tid,)).await
            }
            AgentCommand::Found { tid } => {
                proxy.method_call(AGENT_INTERFACE, "Found", (tid,)).await
            }
            AgentCommand::Close { tid } => {
                proxy.method_call(AGENT_INTERFACE, "Close", (tid,)).await
            }
        };
        if let Err(e) = result {
            tracing::warn!("agent call failed: {e}");
            // The NameOwnerChanged watch normally drops a dead agent
            // first; this is a fallback in case that signal was missed. Do
            // not clear on timeouts or other transient failures.
            if agent_gone(&e) {
                clear_if_matches(&slot, Some(&agent.destination));
            }
        }
    }
}

fn clear_if_matches(slot: &Arc<Mutex<AgentSlot>>, sender: Option<&str>) {
    let mut guard = slot.lock().unwrap();
    if guard
        .current()
        .map(|p| Some(p.destination.as_str()) == sender)
        .unwrap_or(false)
    {
        tracing::info!("agent unregistered");
        guard.current = None;
    }
}

/// Whether a call error means the agent's connection is gone for good.
fn agent_gone(e: &dbus::Error) -> bool {
    matches!(
        e.name(),
        Some(
            "org.freedesktop.DBus.Error.NameHasNoOwner"
                | "org.freedesktop.DBus.Error.ServiceUnknown"
                | "org.freedesktop.DBus.Error.Disconnected"
        )
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_tracks_the_slot() {
        let slot = Arc::new(Mutex::new(AgentSlot::default()));
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = AgentClient {
            tx,
            slot: slot.clone(),
            active_tid: Arc::new(AtomicU64::new(0)),
        };
        assert!(!client.available());

        slot.lock().unwrap().current = Some(Agent {
            destination: ":1.7".into(),
            path: "/org/ucabled/Agent".into(),
            uid: 1000,
        });
        assert!(client.available());

        clear_if_matches(&slot, Some(":1.8"));
        assert!(client.available());
        clear_if_matches(&slot, Some(":1.7"));
        assert!(!client.available());
    }

    #[test]
    fn only_peer_gone_errors_clear_the_agent() {
        for name in [
            "org.freedesktop.DBus.Error.NameHasNoOwner",
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.Disconnected",
        ] {
            assert!(agent_gone(&dbus::Error::new_custom(name, "gone")));
        }
        for name in [
            "org.freedesktop.DBus.Error.NoReply",
            "org.freedesktop.DBus.Error.TimedOut",
            "org.freedesktop.DBus.Error.UnknownMethod",
        ] {
            assert!(!agent_gone(&dbus::Error::new_custom(name, "transient")));
        }
    }
}
