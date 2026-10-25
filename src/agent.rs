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
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use dbus::Path;
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::message::{MatchRule, Message};
use dbus::nonblock::{MsgMatch, Proxy, SyncConnection};
use dbus_crossroads::{Crossroads, MethodErr};
use futures::StreamExt;
use futures::channel::mpsc::UnboundedReceiver as MessageReceiver;
use tokio::runtime::Handle;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

use crate::qr::RequestType;

pub const BUS_NAME: &str = "org.ucabled";

/// Another process already owns [`BUS_NAME`].
///
/// Typed so the daemon can tell a name conflict (fatal: two daemons must not
/// compete for the HID device) from a transient bus failure that may fall
/// back to the terminal UI.
#[derive(Debug)]
pub struct NameTaken;

impl std::fmt::Display for NameTaken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the bus name is owned by another process")
    }
}

impl std::error::Error for NameTaken {}
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

/// polkit check timeout when an agent registers.  Generous: registration is
/// rare, and a slow polkit must not spuriously deny a valid agent.
const REGISTER_TIMEOUT: Duration = Duration::from_secs(25);
/// polkit check timeout for the per-Prompt re-check.  Short and fail-closed,
/// so a hung polkit cannot stall a transaction for long.
const RECHECK_TIMEOUT: Duration = Duration::from_secs(5);

/// Minimum spacing between RegisterAgent attempts from one uid.  Every
/// attempt costs a polkit round trip and any local uid may call, so an
/// unauthenticated flood would otherwise spam polkitd; keying on the uid
/// (resolved once per connection, see [`SenderUids`]) keeps extra
/// connections of one uid from multiplying that budget.  Legitimate
/// agents register once per session; the agent's 2s retry loop stays well
/// clear of this interval.
const REGISTER_MIN_INTERVAL: Duration = Duration::from_secs(1);
/// Bound on remembered uids, so a crowd of them cannot grow the limiter
/// without bound; at capacity the oldest entry is evicted.
const REGISTER_LIMITER_MAX: usize = 64;
/// Bound on cached connection uids, so a uid that opens many system-bus
/// connections cannot grow the map without bound; at capacity an arbitrary
/// entry is dropped and re-resolved on its next call.
const SENDER_UIDS_MAX: usize = 64;

/// Per-uid rate limit for RegisterAgent.
#[derive(Default)]
struct RegisterLimiter {
    last: HashMap<u32, Instant>,
}

impl RegisterLimiter {
    /// Whether `uid` may attempt a registration at `now`; an allowed
    /// attempt is recorded.
    fn check(&mut self, uid: u32, now: Instant) -> bool {
        if let Some(last) = self.last.get_mut(&uid) {
            if now.duration_since(*last) < REGISTER_MIN_INTERVAL {
                return false;
            }
            *last = now;
            return true;
        }
        if self.last.len() >= REGISTER_LIMITER_MAX
            && let Some(oldest) = self
                .last
                .iter()
                .min_by_key(|(_, at)| *at)
                .map(|(uid, _)| *uid)
        {
            self.last.remove(&oldest);
        }
        self.last.insert(uid, now);
        true
    }
}

/// uids resolved for unique bus names, so one connection costs a single
/// GetConnectionUnixUser (not one per registration attempt and another after
/// polkit).  The routing task evicts an entry when its name vanishes
/// (NameOwnerChanged with an empty new owner), and a live unique name never
/// changes uid, so an entry can only go stale after the name is gone — where
/// it is no longer reachable.  The map is bounded: callers may open many
/// connections, and a cache miss only costs one more lookup.
#[derive(Default)]
struct SenderUids {
    map: HashMap<String, u32>,
}

impl SenderUids {
    fn get(&self, sender: &str) -> Option<u32> {
        self.map.get(sender).copied()
    }

    fn insert(&mut self, sender: &str, uid: u32) {
        if !self.map.contains_key(sender)
            && self.map.len() >= SENDER_UIDS_MAX
            && let Some(victim) = self.map.keys().next().cloned()
        {
            self.map.remove(&victim);
        }
        self.map.insert(sender.to_string(), uid);
    }

    fn evict(&mut self, sender: &str) {
        self.map.remove(sender);
    }
}

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
    /// Incremented on every registration; distinguishes the current agent
    /// from a departed one with which a transaction may still be pending.
    generation: u64,
}

impl AgentSlot {
    pub fn current(&self) -> Option<&Agent> {
        self.current.as_ref()
    }
}

/// Why the daemon should drop its in-flight transaction.
#[derive(Debug, Clone, Copy)]
pub enum CancelRequest {
    /// The user cancelled the dialog for this transaction id.
    Transaction(u64),
    /// The registered agent went away, so nothing remains to confirm or
    /// cancel the pending transaction with; abort it.  Tagged with the
    /// agent's registration generation so a signal from a departed agent
    /// cannot abort a transaction owned by its already-registered
    /// successor.
    AgentGone(u64),
}

#[derive(Debug, Clone)]
pub enum AgentCommand {
    Prompt {
        tid: u64,
        url: String,
        rp: Option<String>,
        timeout_secs: u64,
        /// Which CTAP command is being relayed.  Not secret, unlike `url`,
        /// so it may cross to the agent and on to the helper, which phrases
        /// the QR page with it.
        request_type: RequestType,
    },
    /// The phone's BLE advert was seen; the user now confirms on it.
    Found {
        tid: u64,
    },
    Close {
        tid: u64,
    },
    /// Show a user-facing notice (desktop notification).  Content is
    /// static daemon text — nothing attacker-controlled crosses over.
    Notify {
        summary: String,
        body: String,
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

    /// Registration generation of the current agent, if any.  Callers tag
    /// a pending transaction with it so a stale `AgentGone` from the
    /// agent's predecessor is ignored.
    pub fn generation(&self) -> Option<u64> {
        let guard = self.slot.lock().unwrap();
        guard.current.as_ref().map(|_| guard.generation)
    }

    pub fn prompt(
        &self,
        tid: u64,
        url: &str,
        rp: Option<String>,
        timeout_secs: u64,
        request_type: RequestType,
    ) {
        self.active_tid.store(tid, Ordering::SeqCst);
        let _ = self.tx.send(AgentCommand::Prompt {
            tid,
            url: url.to_string(),
            rp,
            timeout_secs,
            request_type,
        });
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

    /// Show a user-facing notice.  Only static daemon text may be passed:
    /// it crosses into the user's session and is rendered by the desktop's
    /// notification service.
    pub fn notify(&self, summary: &str, body: &str) {
        let _ = self.tx.send(AgentCommand::Notify {
            summary: summary.to_string(),
            body: body.to_string(),
        });
    }
}

/// Start the D-Bus service and dispatch tasks on `handle`. `cancel_tx`
/// receives a [`CancelRequest`] when the agent reports that the user
/// cancelled the dialog or when the agent itself goes away.
///
/// The setup (connect, request the bus name, install matches) runs inline so
/// a failure still falls back at startup; message routing and outbound calls
/// then live in spawned tasks on a single async connection.
pub fn start(handle: &Handle, cancel_tx: mpsc::Sender<CancelRequest>) -> Result<AgentClient> {
    let slot = Arc::new(Mutex::new(AgentSlot::default()));
    let sender_uids = Arc::new(Mutex::new(SenderUids::default()));
    let (tx, rx) = mpsc::unbounded_channel();

    let (resource, conn) =
        dbus_tokio::connection::new_system_sync().context("connect system bus")?;
    handle.spawn(async move {
        let err = resource.await;
        tracing::error!("system bus connection lost: {err}");
    });

    let crossroads = build_crossroads(&slot, cancel_tx.clone(), conn.clone(), &sender_uids);
    let streams = handle.block_on(register_on_bus(&conn))?;

    handle.spawn(route_messages(
        conn.clone(),
        crossroads,
        streams,
        slot.clone(),
        cancel_tx.clone(),
        sender_uids,
    ));
    handle.spawn(dispatch_commands(conn, rx, slot.clone(), cancel_tx.clone()));

    Ok(AgentClient {
        tx,
        slot,
        active_tid: Arc::new(AtomicU64::new(0)),
    })
}

/// Check whether `sender` may act as the UI agent.
///
/// Runs on the daemon's existing bus connection over the async API, so it
/// neither opens a new connection nor blocks a worker thread; the caller
/// awaits it.  Only the machine-local polkit round trip is on the path.  The
/// uid is resolved separately (and cached), because polkit identifies the
/// caller by unique name, not by uid.
async fn authorize(conn: &Arc<SyncConnection>, sender: &str, timeout: Duration) -> Result<bool> {
    let mut details: PropMap = PropMap::new();
    let value: Box<dyn RefArg + 'static> = Box::new(sender.to_string());
    details.insert("name".to_string(), Variant(value));
    let subject = ("system-bus-name".to_string(), details);

    let proxy = Proxy::new(POLKIT_BUS, POLKIT_PATH, timeout, conn.clone());
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
        .await
        .context("polkit CheckAuthorization")?;

    Ok(authorized)
}

/// Resolve the unix uid of the connection holding `sender`.
async fn connection_uid(conn: &Arc<SyncConnection>, sender: &str) -> Result<u32> {
    let dbus = Proxy::new(DBUS_BUS, DBUS_PATH, Duration::from_secs(5), conn.clone());
    let (uid,): (u32,) = dbus
        .method_call(DBUS_INTERFACE, "GetConnectionUnixUser", (sender,))
        .await
        .context("GetConnectionUnixUser")?;
    Ok(uid)
}

type MatchStream = (MsgMatch, MessageReceiver<Message>);

/// Guards and streams for the two installed matches; dropping a guard stops
/// matching, so both are owned by the routing task.
struct MatchStreams {
    methods: MatchStream,
    signals: MatchStream,
}

/// Request the bus name and install the method-call and NameOwnerChanged
/// matches, returning the guards (dropping them stops matching) and streams.
async fn register_on_bus(conn: &Arc<SyncConnection>) -> Result<MatchStreams> {
    // `do_not_queue`: a second daemon must fail immediately with a clear
    // error instead of sitting in the name queue until systemd's start
    // timeout kills it.  Single-instance exclusion itself is enforced by the
    // state-directory lock (`instance::acquire`), because `--no-ui` and the
    // terminal fallback never get this far.
    let reply = conn
        .request_name(BUS_NAME, false, false, true)
        .await
        .with_context(|| format!("request bus name {BUS_NAME}"))?;
    // With do_not_queue a taken name is an Ok(Exists) reply, not an error.
    use dbus::nonblock::stdintf::org_freedesktop_dbus::RequestNameReply;
    if !matches!(
        reply,
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner
    ) {
        return Err(anyhow::Error::new(NameTaken))
            .with_context(|| format!("request name {BUS_NAME} reply {reply:?}"));
    }
    let method = conn
        .add_match(MatchRule::new_method_call())
        .await
        .context("match method calls")?;
    let signal = conn
        .add_match(MatchRule::new_signal(DBUS_INTERFACE, "NameOwnerChanged").with_sender(DBUS_BUS))
        .await
        .context("match NameOwnerChanged")?;
    tracing::info!("listening on {BUS_NAME} ({MANAGER_INTERFACE})");
    Ok(MatchStreams {
        methods: method.msg_stream(),
        signals: signal.msg_stream(),
    })
}

fn build_crossroads(
    slot: &Arc<Mutex<AgentSlot>>,
    cancel_tx: mpsc::Sender<CancelRequest>,
    conn: Arc<SyncConnection>,
    sender_uids: &Arc<Mutex<SenderUids>>,
) -> Crossroads {
    let limiter = Arc::new(Mutex::new(RegisterLimiter::default()));
    let sender_uids = sender_uids.clone();
    let mut crossroads = Crossroads::new();
    // Registration authorizes through polkit over the async bus API, so the
    // crossroads must be able to spawn the method's future.
    crossroads.set_async_support(Some((
        conn.clone(),
        Box::new(|fut| {
            tokio::spawn(fut);
        }),
    )));
    let iface = crossroads.register(MANAGER_INTERFACE, {
        let slot = slot.clone();
        move |builder| {
            let register_slot = slot.clone();
            let register_conn = conn.clone();
            let register_limiter = limiter.clone();
            let register_uids = sender_uids.clone();
            let register_cancel = cancel_tx.clone();
            builder.method_with_cr_async(
                "RegisterAgent",
                ("path",),
                (),
                move |mut ctx, _cr, (path,): (Path<'static>,)| {
                    let slot = register_slot.clone();
                    let conn = register_conn.clone();
                    let limiter = register_limiter.clone();
                    let sender_uids = register_uids.clone();
                    let cancel_tx = register_cancel.clone();
                    async move {
                        let Some(sender) = ctx.message().sender().map(|s| s.to_string()) else {
                            return ctx.reply(Err(MethodErr::failed(&"missing sender")));
                        };
                        // Resolve the caller's uid before any polkit work: the
                        // limit is per uid, so extra connections from one
                        // account share a single budget instead of
                        // multiplying the polkit round trips.  A uid cannot
                        // change for a live unique name, so caching it is
                        // safe; route_messages evicts the entry when the name
                        // disappears.
                        let cached_uid = { sender_uids.lock().unwrap().get(&sender) };
                        let uid = match cached_uid {
                            Some(uid) => uid,
                            None => match connection_uid(&conn, &sender).await {
                                Ok(uid) => {
                                    sender_uids.lock().unwrap().insert(&sender, uid);
                                    uid
                                }
                                Err(e) => {
                                    tracing::warn!(%sender, "cannot resolve caller uid: {e:#}");
                                    return ctx.reply(Err(MethodErr::failed(
                                        &"cannot resolve caller uid",
                                    )));
                                }
                            },
                        };
                        if !limiter.lock().unwrap().check(uid, Instant::now()) {
                            tracing::warn!(%sender, uid, "agent registration rate-limited");
                            return ctx.reply(Err(MethodErr::from((
                                "org.freedesktop.DBus.Error.LimitsExceeded",
                                "too many registration attempts",
                            ))));
                        }
                        let authorized = match authorize(&conn, &sender, REGISTER_TIMEOUT).await {
                            Ok(authorized) => authorized,
                            Err(e) => {
                                tracing::warn!("authorization check failed: {e:#}");
                                return ctx
                                    .reply(Err(MethodErr::failed(&"authorization check failed")));
                            }
                        };
                        if !authorized {
                            tracing::warn!(%sender, "agent registration denied by polkit");
                            return ctx.reply(Err(MethodErr::from((
                                ERR_NOT_AUTHORIZED,
                                "only the active local session may register an agent",
                            ))));
                        }
                        let authorized_uid = uid;
                        let agent = Agent {
                            destination: sender.clone(),
                            path: path.to_string(),
                            uid: authorized_uid,
                        };
                        let (generation, replaced) = install_agent(&slot, agent);
                        tracing::info!(
                            %sender,
                            uid = authorized_uid,
                            %path,
                            generation,
                            "agent registered"
                        );
                        if let Some(previous) = replaced {
                            // Registration is last-wins, but the replaced
                            // agent's window (and the QR secret in it) and the
                            // transaction bound to it would otherwise keep the
                            // shared HID channel locked until completion or
                            // timeout.  Abort it through its registration
                            // generation, which no successor can match.
                            let _ = cancel_tx.send(CancelRequest::AgentGone(previous)).await;
                        }
                        ctx.reply(Ok(()))
                    }
                },
            );

            let unregister_slot = slot.clone();
            let unregister_cancel = cancel_tx.clone();
            builder.method("UnregisterAgent", (), (), move |ctx, _: &mut (), ()| {
                let sender = ctx.message().sender().map(|s| s.to_string());
                if let Some(generation) = clear_if_matches(&unregister_slot, sender.as_deref()) {
                    // Sync context: no guaranteed delivery here.  The queue
                    // drains promptly, so a full queue means a flood; the
                    // async departure paths below use blocking delivery.
                    if unregister_cancel
                        .try_send(CancelRequest::AgentGone(generation))
                        .is_err()
                    {
                        tracing::warn!("cancel queue full, agent-gone signal dropped");
                    }
                }
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
                        let _ = cancel_tx.try_send(CancelRequest::Transaction(tid));
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
    streams: MatchStreams,
    slot: Arc<Mutex<AgentSlot>>,
    cancel_tx: mpsc::Sender<CancelRequest>,
    sender_uids: Arc<Mutex<SenderUids>>,
) {
    let (_method_match, mut methods) = streams.methods;
    let (_signal_match, mut signals) = streams.signals;
    loop {
        tokio::select! {
            msg = methods.next() => {
                let Some(msg) = msg else { break };
                let _ = crossroads.handle_message(msg, &*conn);
            }
            sig = signals.next() => {
                let Some(sig) = sig else { break };
                if let Ok((name, _old_owner, new_owner)) = sig.read3::<String, String, String>()
                    && new_owner.is_empty() {
                        // The name is gone for good, so its cached uid can
                        // never be used again; drop it so a long-lived bus
                        // does not accumulate one entry per vanished peer.
                        sender_uids.lock().unwrap().evict(&name);
                        if let Some(generation) = clear_if_matches(&slot, Some(&name)) {
                            // Awaited delivery: this is the backstop that
                            // frees the shared HID channel and must not be
                            // lost to a momentarily full queue.
                            let _ = cancel_tx
                                .send(CancelRequest::AgentGone(generation))
                                .await;
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
    cancel_tx: mpsc::Sender<CancelRequest>,
) {
    while let Some(command) = rx.recv().await {
        // The lock is released before the await: only the cloned Agent
        // crosses it.
        let Some(agent) = slot.lock().unwrap().current().cloned() else {
            tracing::debug!("no agent registered, dropping UI command");
            continue;
        };
        // A Prompt carries the QR transaction secret, and allow_active is a
        // property of the session, not of the agent: after a fast user switch
        // a previous session's agent can linger with its registration intact.
        // Re-check with polkit before delivering the secret, as the design
        // doc requires.  Registration was already authorized, so this only
        // guards the case where the session's activity changed since.
        if let AgentCommand::Prompt { tid, .. } = &command {
            let tid = *tid;
            let outcome = recheck(&authorize(&conn, &agent.destination, RECHECK_TIMEOUT).await);
            match outcome {
                Recheck::Authorized => {}
                outcome @ (Recheck::Denied | Recheck::Inconclusive) => {
                    if outcome == Recheck::Denied {
                        tracing::warn!(
                            destination = %agent.destination,
                            "agent's session is no longer active, dropping the prompt"
                        );
                    } else {
                        tracing::warn!("polkit re-check failed, cancelling the transaction");
                    }
                    if let Some(cancel) = recheck_cancel(outcome, tid, &agent.destination, &slot) {
                        let _ = cancel_tx.send(cancel).await;
                    }
                    continue;
                }
            }
        }
        let proxy = Proxy::new(
            agent.destination.as_str(),
            agent.path.as_str(),
            Duration::from_secs(15),
            conn.clone(),
        );
        let prompt_tid = match &command {
            AgentCommand::Prompt { tid, .. } => Some(*tid),
            _ => None,
        };
        let result: std::result::Result<(), dbus::Error> = match command {
            AgentCommand::Prompt {
                tid,
                url,
                rp,
                timeout_secs,
                request_type,
            } => {
                proxy
                    .method_call(
                        AGENT_INTERFACE,
                        "Prompt",
                        (
                            tid,
                            url,
                            rp.unwrap_or_default(),
                            timeout_secs,
                            request_type.as_str(),
                        ),
                    )
                    .await
            }
            AgentCommand::Found { tid } => {
                proxy.method_call(AGENT_INTERFACE, "Found", (tid,)).await
            }
            AgentCommand::Close { tid } => {
                proxy.method_call(AGENT_INTERFACE, "Close", (tid,)).await
            }
            AgentCommand::Notify { summary, body } => {
                proxy
                    .method_call(AGENT_INTERFACE, "Notify", (summary, body))
                    .await
            }
        };
        if let Err(e) = result {
            tracing::warn!("agent call failed: {e}");
            match failed_call(agent_gone(&e), prompt_tid) {
                // The NameOwnerChanged watch normally drops a dead agent
                // first; this is a fallback in case that signal was missed.
                // Clearing the slot aborts the transaction through its
                // generation tag, so it needs no separate tid cancel.
                FailedCall::Gone => {
                    if let Some(generation) = clear_if_matches(&slot, Some(&agent.destination)) {
                        let _ = cancel_tx.send(CancelRequest::AgentGone(generation)).await;
                    }
                }
                // A Prompt that could not be delivered (registered but
                // unresponsive agent, call timed out) must not leave the
                // transaction running invisibly with the HID channel held:
                // cancel it like a user cancellation.
                FailedCall::CancelTransaction(tid) => {
                    let _ = cancel_tx.send(CancelRequest::Transaction(tid)).await;
                }
                FailedCall::Nothing => {}
            }
        }
    }
}

/// Install `agent` as the current agent, returning its registration
/// generation and, when an agent was already registered, the generation it
/// replaced.  Callers signal the daemon for a replaced generation so a
/// transaction still bound to the previous agent is aborted instead of
/// holding the HID channel until its own timeout.
fn install_agent(slot: &Arc<Mutex<AgentSlot>>, agent: Agent) -> (u64, Option<u64>) {
    let mut guard = slot.lock().unwrap();
    guard.generation += 1;
    let generation = guard.generation;
    let replaced = guard.current.replace(agent).is_some();
    (generation, replaced.then(|| generation - 1))
}

/// Drop the slot's agent if it belongs to `sender`, returning the dropped
/// agent's registration generation.  Callers signal the daemon so an
/// in-flight transaction is aborted rather than leaving the shared HID
/// channel locked until the BLE timeout.
fn clear_if_matches(slot: &Arc<Mutex<AgentSlot>>, sender: Option<&str>) -> Option<u64> {
    let mut guard = slot.lock().unwrap();
    if guard
        .current()
        .map(|p| Some(p.destination.as_str()) == sender)
        .unwrap_or(false)
    {
        tracing::info!("agent unregistered");
        guard.current = None;
        return Some(guard.generation);
    }
    None
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

/// How a failed agent call should be reported to the daemon.  A gone agent
/// is handled by clearing its slot, which aborts the transaction through
/// the registration generation; an undeliverable Prompt to a still-live
/// agent has no such departure to key on and is cancelled by transaction
/// id instead.  Other commands own no transaction, so a transient failure
/// on them cancels nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailedCall {
    Gone,
    CancelTransaction(u64),
    Nothing,
}

fn failed_call(is_gone: bool, prompt_tid: Option<u64>) -> FailedCall {
    if is_gone {
        FailedCall::Gone
    } else if let Some(tid) = prompt_tid {
        FailedCall::CancelTransaction(tid)
    } else {
        FailedCall::Nothing
    }
}

/// Outcome of the polkit re-check run before a Prompt is delivered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Recheck {
    /// Still authorized: deliver the prompt.
    Authorized,
    /// The agent's session is no longer the active one: drop the agent and
    /// abort the transaction.
    Denied,
    /// The check could not be completed (bus or polkit failure): abort this
    /// transaction but keep the agent, which may still be valid.
    Inconclusive,
}

fn recheck(result: &Result<bool>) -> Recheck {
    match result {
        Ok(true) => Recheck::Authorized,
        Ok(false) => Recheck::Denied,
        Err(_) => Recheck::Inconclusive,
    }
}

/// Apply a re-check outcome to the agent slot and decide what the daemon
/// should be told.  A denial drops the agent and aborts the transaction
/// through its registration generation; an inconclusive check keeps the
/// agent and aborts only the transaction, so a transient bus or polkit
/// failure does not deregister a healthy agent.  `None` means the prompt
/// may be delivered.
fn recheck_cancel(
    outcome: Recheck,
    tid: u64,
    destination: &str,
    slot: &Arc<Mutex<AgentSlot>>,
) -> Option<CancelRequest> {
    match outcome {
        Recheck::Authorized => None,
        Recheck::Denied => clear_if_matches(slot, Some(destination)).map(CancelRequest::AgentGone),
        Recheck::Inconclusive => Some(CancelRequest::Transaction(tid)),
    }
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

        assert!(clear_if_matches(&slot, Some(":1.8")).is_none());
        assert!(client.available());
        assert!(clear_if_matches(&slot, Some(":1.7")).is_some());
        assert!(!client.available());
    }

    #[test]
    fn replacing_an_agent_reports_the_previous_generation() {
        let slot = Arc::new(Mutex::new(AgentSlot::default()));
        let agent = |name: &str| Agent {
            destination: name.into(),
            path: "/org/ucabled/Agent".into(),
            uid: 1000,
        };

        // The first registration replaces nothing.
        assert_eq!(install_agent(&slot, agent(":1.7")), (1, None));

        // A new registration is last-wins and reports the agent it replaced,
        // so the daemon can abort that agent's in-flight transaction.
        assert_eq!(install_agent(&slot, agent(":1.8")), (2, Some(1)));
        assert_eq!(
            slot.lock()
                .unwrap()
                .current()
                .map(|a| a.destination.as_str()),
            Some(":1.8")
        );

        // After the agent leaves there is nothing to abort on re-register.
        assert!(clear_if_matches(&slot, Some(":1.8")).is_some());
        assert_eq!(install_agent(&slot, agent(":1.9")), (3, None));
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

    #[test]
    fn a_gone_agent_is_not_also_cancelled_by_tid() {
        use FailedCall::*;
        // A departure aborts through the slot's generation, so no tid
        // cancel is emitted alongside it.
        assert_eq!(failed_call(true, Some(7)), Gone);
        assert_eq!(failed_call(true, None), Gone);
        // An undelivered Prompt to a still-registered agent is cancelled
        // by its transaction id.
        assert_eq!(failed_call(false, Some(7)), CancelTransaction(7));
        // A transient failure on a command with no transaction is ignored.
        assert_eq!(failed_call(false, None), Nothing);
    }

    #[test]
    fn sender_uid_cache_is_bounded_and_keeps_existing_entries() {
        let mut uids = SenderUids::default();
        uids.insert(":1.1", 1000);
        assert_eq!(uids.get(":1.1"), Some(1000));

        for i in 0..SENDER_UIDS_MAX + 10 {
            uids.insert(&format!(":1.{}", i + 2), 1001);
        }
        assert_eq!(uids.map.len(), SENDER_UIDS_MAX);

        // Updating a cached sender must not evict another entry.
        let existing = uids.map.keys().next().unwrap().clone();
        uids.insert(&existing, 7);
        assert_eq!(uids.map.len(), SENDER_UIDS_MAX);
        assert_eq!(uids.get(&existing), Some(7));

        uids.evict(&existing);
        assert_eq!(uids.get(&existing), None);
    }

    #[test]
    fn recheck_distinguishes_denial_from_failure() {
        use Recheck::*;
        assert_eq!(recheck(&Ok(true)), Authorized);
        // A definite denial means the session is no longer active.
        assert_eq!(recheck(&Ok(false)), Denied);
        // An inconclusive check (bus/polkit failure) must not be treated as
        // a denial, or a hiccup would deregister a healthy agent.
        assert_eq!(
            recheck(&Err(anyhow::anyhow!("polkit unreachable"))),
            Inconclusive
        );
    }

    #[test]
    fn recheck_effects_keep_or_drop_the_agent() {
        let slot = Arc::new(Mutex::new(AgentSlot::default()));
        slot.lock().unwrap().current = Some(Agent {
            destination: ":1.7".into(),
            path: "/org/ucabled/Agent".into(),
            uid: 1000,
        });
        slot.lock().unwrap().generation = 42;

        // Authorized: deliver the prompt and leave the agent in place.
        assert!(recheck_cancel(Recheck::Authorized, 7, ":1.7", &slot).is_none());
        assert!(slot.lock().unwrap().current.is_some());

        // Inconclusive: keep the agent, abort only this transaction, so a
        // transient failure does not deregister a healthy agent.
        assert!(matches!(
            recheck_cancel(Recheck::Inconclusive, 7, ":1.7", &slot),
            Some(CancelRequest::Transaction(7))
        ));
        assert!(slot.lock().unwrap().current.is_some());

        // Denied: drop the agent and abort through its generation.
        assert!(matches!(
            recheck_cancel(Recheck::Denied, 7, ":1.7", &slot),
            Some(CancelRequest::AgentGone(42))
        ));
        assert!(slot.lock().unwrap().current.is_none());
    }

    #[test]
    fn register_limiter_throttles_per_uid() {
        let mut limiter = RegisterLimiter::default();
        let t0 = Instant::now();
        assert!(limiter.check(1000, t0));
        assert!(!limiter.check(1000, t0 + Duration::from_millis(100)));
        // Extra connections from the same uid share the bucket, so a second
        // connection cannot multiply the polkit round trips.
        assert!(!limiter.check(1000, t0 + Duration::from_millis(100)));
        // Another uid is unaffected.
        assert!(limiter.check(1001, t0 + Duration::from_millis(100)));
        // After the interval the same uid may try again, and a denied
        // attempt does not push the window out.
        assert!(limiter.check(1000, t0 + REGISTER_MIN_INTERVAL));
    }

    #[test]
    fn register_limiter_bounds_remembered_uids() {
        let mut limiter = RegisterLimiter::default();
        let t0 = Instant::now();
        for uid in 0..REGISTER_LIMITER_MAX as u32 {
            assert!(limiter.check(uid, t0));
        }
        assert_eq!(limiter.last.len(), REGISTER_LIMITER_MAX);
        // A new uid evicts the oldest entry instead of growing the map.
        assert!(limiter.check(9999, t0));
        assert_eq!(limiter.last.len(), REGISTER_LIMITER_MAX);
    }
}
