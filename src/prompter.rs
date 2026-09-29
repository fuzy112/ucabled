//! D-Bus bridge between the system daemon and the per-user session agent.
//!
//! The daemon owns the system-bus name `org.ucabled` and exposes
//! `org.ucabled.Ui1`; a session agent registers an `org.ucabled.Prompter1`
//! object and is then called back to show the QR window.
//!
//! Registration is authorized through polkit (action
//! `org.ucabled.register-prompter`, `allow_active=yes`), so only the user of
//! the current active local session can register. The daemon passes the
//! caller's unique bus name as a `system-bus-name` subject and re-checks on
//! every prompt, so a prompter left over from a previous session is ignored.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use dbus::arg::{PropMap, RefArg, Variant};
use dbus::blocking::{Connection, Proxy};
use dbus::Path;
use dbus_crossroads::{Crossroads, MethodErr};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub const BUS_NAME: &str = "org.ucabled";
pub const UI_PATH: &str = "/org/ucabled/Ui";
pub const UI_INTERFACE: &str = "org.ucabled.Ui1";
pub const PROMPTER_INTERFACE: &str = "org.ucabled.Prompter1";
pub const POLKIT_ACTION: &str = "org.ucabled.register-prompter";
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
pub struct Prompter {
    /// Unique bus name of the agent (`:1.x`), used as call destination.
    pub destination: String,
    /// Object path of its `org.ucabled.Prompter1` implementation.
    pub path: String,
    pub uid: u32,
}

/// The single prompter the daemon currently talks to.
#[derive(Default)]
pub struct PrompterSlot {
    current: Option<Prompter>,
}

impl PrompterSlot {
    pub fn current(&self) -> Option<&Prompter> {
        self.current.as_ref()
    }
}

#[derive(Debug, Clone)]
pub enum UiCommand {
    Show {
        tid: u64,
        url: String,
        rp: Option<String>,
        timeout_secs: u64,
    },
    Found {
        tid: u64,
    },
    Close {
        tid: u64,
    },
}

/// Daemon-side handle to the UI bridge. Cheap to clone and safe to use from
/// any task; commands are handed to a dedicated blocking D-Bus thread.
#[derive(Clone)]
pub struct UiClient {
    tx: UnboundedSender<UiCommand>,
    slot: Arc<Mutex<PrompterSlot>>,
    active_tid: Arc<AtomicU64>,
}

impl UiClient {
    /// Whether an authorized prompter is currently registered.
    pub fn available(&self) -> bool {
        self.slot.lock().unwrap().current.is_some()
    }

    pub fn show(&self, tid: u64, url: &str, rp: Option<String>, timeout_secs: u64) {
        self.active_tid.store(tid, Ordering::SeqCst);
        let _ = self.tx.send(UiCommand::Show {
            tid,
            url: url.to_string(),
            rp,
            timeout_secs,
        });
    }

    /// The phone's BLE advert was seen; the user now confirms on it.
    pub fn found(&self) {
        let tid = self.active_tid.load(Ordering::SeqCst);
        let _ = self.tx.send(UiCommand::Found { tid });
    }

    /// The transaction ended (or was cancelled); tear the window down.
    pub fn close(&self) {
        let tid = self.active_tid.load(Ordering::SeqCst);
        let _ = self.tx.send(UiCommand::Close { tid });
    }
}

/// Start the D-Bus service and dispatch threads. `cancel_tx` receives a `()`
/// when the agent reports that the user cancelled the dialog.
pub fn start(cancel_tx: UnboundedSender<()>) -> Result<UiClient> {
    let slot = Arc::new(Mutex::new(PrompterSlot::default()));
    let (tx, rx) = mpsc::unbounded_channel();

    let service_slot = slot.clone();
    std::thread::Builder::new()
        .name("ucabled-ui-service".into())
        .spawn(move || {
            if let Err(e) = service_thread(cancel_tx, service_slot) {
                tracing::error!("UI D-Bus service stopped: {e:#}");
            }
        })
        .context("spawn UI service thread")?;

    let dispatch_slot = slot.clone();
    std::thread::Builder::new()
        .name("ucabled-ui-dispatch".into())
        .spawn(move || {
            if let Err(e) = dispatch_thread(rx, dispatch_slot) {
                tracing::error!("UI dispatch stopped: {e:#}");
            }
        })
        .context("spawn UI dispatch thread")?;

    Ok(UiClient {
        tx,
        slot,
        active_tid: Arc::new(AtomicU64::new(0)),
    })
}

/// Check whether `sender` may register a prompter, returning its uid.
///
/// Opens its own bus connection: this runs on the service thread and only on
/// registration, so the cost is irrelevant.
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

fn service_thread(cancel_tx: UnboundedSender<()>, slot: Arc<Mutex<PrompterSlot>>) -> Result<()> {
    let conn = Connection::new_system().context("connect system bus")?;
    conn.request_name(BUS_NAME, false, false, false)
        .with_context(|| format!("request bus name {BUS_NAME}"))?;

    let mut crossroads = Crossroads::new();
    let iface = crossroads.register(UI_INTERFACE, {
        let slot = slot.clone();
        move |builder| {
            let register_slot = slot.clone();
            builder.method(
                "RegisterPrompter",
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
                        tracing::warn!(%sender, "prompter registration denied by polkit");
                        return Err(MethodErr::from((
                            ERR_NOT_AUTHORIZED,
                            "only the active local session may register a prompter",
                        )));
                    };
                    tracing::info!(%sender, %path, uid, "prompter registered");
                    register_slot.lock().unwrap().current = Some(Prompter {
                        destination: sender,
                        path: path.to_string(),
                        uid,
                    });
                    Ok(())
                },
            );

            let unregister_slot = slot.clone();
            builder.method("UnregisterPrompter", (), (), move |ctx, _: &mut (), ()| {
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
                        tracing::info!(tid, "prompter reported cancellation");
                        let _ = cancel_tx.send(());
                    }
                    Ok(())
                },
            );
        }
    });
    crossroads.insert(UI_PATH, &[iface], ());

    tracing::info!("listening on {BUS_NAME} ({UI_INTERFACE})");
    crossroads.serve(&conn)?;
    Ok(())
}

fn clear_if_matches(slot: &Arc<Mutex<PrompterSlot>>, sender: Option<&str>) {
    let mut guard = slot.lock().unwrap();
    if guard
        .current()
        .map(|p| Some(p.destination.as_str()) == sender)
        .unwrap_or(false)
    {
        tracing::info!("prompter unregistered");
        guard.current = None;
    }
}

fn dispatch_thread(
    mut rx: UnboundedReceiver<UiCommand>,
    slot: Arc<Mutex<PrompterSlot>>,
) -> Result<()> {
    let conn = Connection::new_system().context("connect system bus")?;

    while let Some(command) = rx.blocking_recv() {
        let Some(prompter) = slot.lock().unwrap().current().cloned() else {
            tracing::debug!("no prompter registered, dropping UI command");
            continue;
        };
        let proxy = Proxy::new(
            prompter.destination.as_str(),
            prompter.path.as_str(),
            Duration::from_secs(15),
            &conn,
        );
        let result: std::result::Result<(), dbus::Error> = match command {
            UiCommand::Show {
                tid,
                url,
                rp,
                timeout_secs,
            } => proxy.method_call(
                PROMPTER_INTERFACE,
                "Prompt",
                (tid, url, rp.unwrap_or_default(), timeout_secs),
            ),
            UiCommand::Found { tid } => proxy.method_call(PROMPTER_INTERFACE, "Found", (tid,)),
            UiCommand::Close { tid } => proxy.method_call(PROMPTER_INTERFACE, "Close", (tid,)),
        };
        if let Err(e) = result {
            tracing::warn!("prompter call failed: {e}");
            clear_if_matches(&slot, Some(&prompter.destination));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn availability_tracks_the_slot() {
        let slot = Arc::new(Mutex::new(PrompterSlot::default()));
        let (tx, _rx) = mpsc::unbounded_channel();
        let client = UiClient {
            tx,
            slot: slot.clone(),
            active_tid: Arc::new(AtomicU64::new(0)),
        };
        assert!(!client.available());

        slot.lock().unwrap().current = Some(Prompter {
            destination: ":1.7".into(),
            path: "/org/ucabled/Prompter".into(),
            uid: 1000,
        });
        assert!(client.available());

        clear_if_matches(&slot, Some(":1.8"));
        assert!(client.available());
        clear_if_matches(&slot, Some(":1.7"));
        assert!(!client.available());
    }
}
