// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use ucabled::agent::CancelRequest;
use ucabled::ctap::{
    CMD_GET_NEXT_ASSERTION, CMD_MAKE_CREDENTIAL, CTAP1_ERR_CHANNEL_BUSY, CTAP1_ERR_TIMEOUT,
    CTAP2_ERR_INVALID_OPTION, CTAP2_ERR_NO_CREDENTIALS, CTAP2_ERR_NOT_ALLOWED,
    CTAP2_ERR_PIN_NOT_SET,
};
use ucabled::ctaphid::{CtapAction, Transport};
use ucabled::qr::RequestType;
use ucabled::uhid_dev::{FIDO_REPORT_DESCRIPTOR, UhidDevice, UhidEvent};
use ucabled::ui::Notifier;

const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(150);

/// Delay before retrying a transient uhid read error.
const UHID_READ_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Log every Nth consecutive write failure (the first is always logged).
const WRITE_FAILURE_LOG_EVERY: u32 = 100;

/// Budget for one HID input report write, and how many times it is tried
/// before the rest of the response is abandoned.  A host that stops reading
/// must not block the daemon's event loop: `write_input` waits for
/// writability, and without a bound a wedged reader would hang it forever.
const WRITE_REPORT_TIMEOUT: Duration = Duration::from_millis(250);
const WRITE_REPORT_ATTEMPTS: u32 = 2;

/// Whole-response budget.  The per-report bound keeps one wedged write from
/// blocking the loop forever; this bounds the sum, so a host that accepts
/// every report just under its budget cannot hold the daemon for
/// `reports * attempts * WRITE_REPORT_TIMEOUT` (tens of seconds for a
/// maximum-size CTAPHID response).
const WRITE_RESPONSE_TIMEOUT: Duration = Duration::from_secs(2);

/// Bytes of an input report needed to log its CID and command (cid + cmd).
const HID_REPORT_LOG_PREFIX: usize = 5;

/// Consecutive uhid read failures tolerated before giving up (and letting
/// systemd restart the service and recreate the device).
const MAX_UHID_READ_FAILURES: u32 = 10;

/// How long a blink probe may stay unanswered (the QR window is only open
/// while it is pending, so this bounds the whole "scan the code, or touch
/// another key" decision).  The helper gets the same value as its own window
/// timeout.
const BLINK_TIMEOUT: Duration = Duration::from_secs(70);

/// How long a multi-credential session waits for the host's next
/// `authenticatorGetNextAssertion` before the phone's assertion state is
/// dropped.  One CTAP reply budget: the browser's account picker may stay
/// open as long as a normal ceremony needs for confirmation.
const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(300);

/// Absolute lifetime of a multi-credential session, counted from the first
/// reply.  The idle timeout restarts on every follow-up, so without an
/// absolute bound a cooperative host and phone could keep the tunnel (and the
/// phone's assertion state) alive for as long as they keep asking.
const SESSION_MAX_LIFETIME: Duration = Duration::from_secs(600);

/// Upper bound on follow-up `getNextAssertion` calls accepted from one
/// `numberOfCredentials` value.  The count is phone-controlled CBOR; a real
/// account picker is far below this, and the bound keeps the loop counter and
/// the number of relayed replies finite.
const MAX_NEXT_ASSERTIONS: usize = 64;

/// User-facing notice when a tunnel redirect is refused.  Static text only:
/// nothing the tunnel server sent may cross into the user's session, where
/// the notification service may render markup.
const REDIRECT_REFUSED_SUMMARY: &str = "Passkey sign-in stopped";
const REDIRECT_REFUSED_BODY: &str = "The phone tunnel server sent an unexpected \
redirect, so the transaction was refused.  If this repeats, please report it; \
details are in the system journal (journalctl -u ucabled).";

/// Bound on queued UI cancellations.  Cancellations are only acted on when
/// they match the in-flight transaction, so a full queue can only ever mean
/// a misbehaving agent; excess ones are dropped in the D-Bus layer.
const CANCEL_QUEUE_SIZE: usize = 16;

/// The caBLE session behind a pending operation.
struct Transaction {
    cid: u32,
    /// Per-transaction token, so a reply or a cancellation that was already
    /// queued when the host moved on cannot touch a newer transaction.
    generation: u64,
    /// Aborts the session task, its tunnel, and its window.
    abort: AbortHandle,
    /// Drives the session task: its first command, then follow-ups.
    next: mpsc::UnboundedSender<SessionCmd>,
    /// Registration generation of the agent the window was opened from,
    /// used to recognise a stale agent-gone signal.
    agent: Option<u64>,
}

/// What the daemon is waiting for on a pending transaction.
enum Phase {
    /// Firefox's blink probe ("Multiple devices found") holds the channel and
    /// the QR window is up.  `answered` flips once the phone has connected and
    /// the probe got its Pin* status; while it is still `false` another
    /// authenticator (a physical security key) can win the selection by being
    /// touched, which makes Firefox cancel the probe.
    Blink { answered: bool },
    /// A CTAP command is being relayed.
    Relay,
    /// The first reply reported more than one credential, so the caBLE
    /// session stays open and `getNextAssertion` follow-ups are forwarded
    /// through `next` instead of starting a new transaction.
    Multi,
}

/// An in-flight operation that the daemon is waiting on.
struct Pending {
    transaction: Transaction,
    phase: Phase,
    /// When to give up on a blink probe (and on the host's first command
    /// after it).  `None` once a real command is being relayed: those are
    /// bounded by the phone and CTAP reply timeouts instead.
    deadline: Option<tokio::time::Instant>,
}

/// A command from the daemon to a live caBLE session.
enum SessionCmd {
    /// The first (or only) CTAP command of the transaction.
    Command(Vec<u8>),
    /// Relay one `authenticatorGetNextAssertion`.
    Next,
}

/// One reply from a relay task: the first reply, or a follow-up.
struct RelayReply {
    cid: u32,
    /// Per-transaction token, so a reply that was already queued when the
    /// host cancelled cannot complete the next transaction on this channel.
    generation: u64,
    payload: Result<Vec<u8>, ucabled::error::TransactionError>,
    /// Credentials still reachable via getNextAssertion (0 = none left).
    remaining: usize,
}

impl Pending {
    fn cid(&self) -> u32 {
        self.transaction.cid
    }

    fn generation(&self) -> u64 {
        self.transaction.generation
    }

    fn agent(&self) -> Option<u64> {
        self.transaction.agent
    }
}

/// Whether a cancel request applies to the pending operation.  A user
/// cancellation is tagged with the transaction id the agent was prompted
/// for, and an agent-gone signal with the agent's registration generation,
/// so a stale signal (a window closed after its transaction ended, an agent
/// whose successor already started a new transaction) cannot abort a later,
/// unrelated transaction.
fn cancel_aborts(request: &CancelRequest, pending: Option<&Pending>) -> bool {
    match (request, pending) {
        (CancelRequest::Transaction(tid), Some(p)) => p.cid() as u64 == *tid,
        (CancelRequest::AgentGone(generation), Some(p)) => p.agent() == Some(*generation),
        _ => false,
    }
}

/// Whether a relay reply belongs to the pending transaction.  The channel id
/// alone is not enough: after a cancellation the host can reuse it for a new
/// transaction, and a reply already queued by the old one must not answer it.
fn reply_matches(pending: Option<&Pending>, reply: &RelayReply) -> bool {
    pending.is_some_and(|p| p.cid() == reply.cid && p.generation() == reply.generation)
}

fn main() -> Result<()> {
    let mut no_ui = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--no-ui" => no_ui = true,
            "-h" | "--help" => {
                println!(
                    "ucabled {} — virtual FIDO2 device relaying WebAuthn ceremonies\n\
                     to a phone via caBLE v2 (hybrid transport)\n\
                     \n\
                     Usage: ucabled [--no-ui]\n\
                     \n\
                     Options:\n\
                     \x20 --no-ui        print the QR code on the terminal instead of\n\
                     \x20                 using the UI agent\n\
                     \x20 -h, --help     print this help\n\
                     \x20 -V, --version  print the version",
                    env!("CARGO_PKG_VERSION")
                );
                return Ok(());
            }
            "-V" | "--version" => {
                println!("ucabled {}", env!("CARGO_PKG_VERSION"));
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
    // One daemon per state directory, before anything can create the identity
    // key or register a virtual HID device.  The bus name is not enough:
    // `--no-ui` and the terminal fallback never claim it.
    let state_dir = ucabled::identity::state_dir();
    let _instance = ucabled::instance::acquire(&state_dir)
        .context("failed to acquire the single-instance lock")?;
    tracing::debug!(lock = %_instance.path().display(), "acquired the instance lock");
    let (cancel_tx, cancel_rx) = mpsc::channel::<CancelRequest>(CANCEL_QUEUE_SIZE);
    let rt = tokio::runtime::Runtime::new()?;
    let notifier = if no_ui {
        Notifier::Terminal
    } else {
        match ucabled::agent::start(rt.handle(), cancel_tx.clone()) {
            Ok(ui) => Notifier::Agent(ui),
            // A name conflict means another daemon (with a different state
            // directory, or a foreign implementation) would compete for the
            // same HID device: fall back to the terminal would keep both
            // alive, so refuse to start.
            Err(e) if e.chain().any(|c| c.is::<ucabled::agent::NameTaken>()) => {
                return Err(e.context("another process owns org.ucabled"));
            }
            Err(e) => {
                tracing::warn!("UI bridge unavailable ({e:#}), falling back to terminal QR");
                Notifier::Terminal
            }
        }
    };

    rt.block_on(daemon_loop(notifier, cancel_rx))
}

/// Build the keepalive timer.  The missed-tick policy is `Delay`, not the
/// default `Burst`: a keepalive's job is a steady cadence, and bursting the
/// backlog of an idle interval would flood the host with thousands of
/// reports the moment a transaction starts.
fn keepalive_interval() -> tokio::time::Interval {
    let mut interval = tokio::time::interval(KEEPALIVE_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    interval
}

async fn daemon_loop(
    notifier: Notifier,
    mut cancel_rx: mpsc::Receiver<CancelRequest>,
) -> Result<()> {
    // Warn (do not fix) when /dev/uhid is writable by anyone but the ucabled
    // account: a loose node lets a local process create arbitrary virtual HID
    // devices.  We deliberately do not tighten permissions ourselves.  The
    // check is repeated whenever a host opens the device, because udev can
    // re-grant a stale `uaccess` ACL after startup (a seat change re-applies
    // it) or a fixed rule can tighten it again.
    let mut uhid_loose = match ucabled::uhid_perm::loose_reason(std::path::Path::new("/dev/uhid")) {
        Ok(reason) => reason,
        Err(e) => {
            // Unknown is not "tight": keep the previous state (none yet) and
            // let the next successful check decide.
            tracing::debug!("cannot inspect /dev/uhid permissions: {e}");
            None
        }
    };
    if let Some(reason) = &uhid_loose {
        tracing::warn!(
            "/dev/uhid is writable by more than the ucabled account ({reason}); a local \
             process could create arbitrary virtual HID devices (e.g. a keyboard). A stale \
             `uaccess` udev tag makes logind re-grant the active seat user; rebooting, or \
             `udevadm trigger /sys/class/misc/uhid`, with the current rule removes it."
        );
    }

    let device = UhidDevice::create("Phone Passkey Bridge", &FIDO_REPORT_DESCRIPTOR)
        .context("failed to create uhid device (is /dev/uhid accessible?)")?;
    tracing::info!("virtual FIDO2 device registered as 'Phone Passkey Bridge'");
    // Persistent caBLE identity. QR transactions do not use it (each one
    // generates a fresh ephemeral identity, like Chromium); it is groundwork
    // for caBLE linking (docs/linking.md), where it becomes the desktop's
    // long-term identity towards linked phones.
    let _identity = ucabled::identity::load_or_create(&ucabled::identity::state_dir())
        .context("failed to load the persistent identity key")?;
    let mut transport = Transport::new(ucabled::ctap::AAGUID);

    // systemd readiness, plus a keepalive when the unit sets WatchdogSec=: a
    // hung daemon is then restarted instead of silently wedging the HID
    // channel.
    ucabled::notify::ready();
    if let Some(interval) = ucabled::notify::watchdog_interval() {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                ucabled::notify::watchdog();
            }
        });
    }
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<RelayReply>();
    // A session task reports here when the phone has connected and the
    // tunnel is open; only then is a blink probe answered with Pin*, so a
    // physical key touched before that still wins the selection.
    let (ready_tx, mut ready_rx) = mpsc::unbounded_channel::<(u32, u64)>();
    let mut pending: Option<Pending> = None;
    let mut next_generation: u64 = 1;
    // Credentials still to serve via `authenticatorGetNextAssertion` after a
    // faked preflight response that reported more than one credential. Kept as
    // (rpId, remaining credential ids).
    let mut fake_assertions: Option<(String, std::collections::VecDeque<Vec<u8>>)> = None;
    let mut keepalive = keepalive_interval();
    let mut read_failures = 0u32;
    let mut write_failures = 0u32;
    // Number of processes holding the hidraw node open (several clients may
    // share it); the transport is reset when the last one closes.
    let mut open_count = 0u32;

    loop {
        tokio::select! {
            event = device.next_event() => {
                let event = match event {
                    Ok(e) => {
                        read_failures = 0;
                        e
                    }
                    Err(e) => {
                        // Transient errors (EINTR, a closed hidraw fd, ...)
                        // should not tear down the whole daemon, but a
                        // persistent one means the device is gone.
                        read_failures += 1;
                        if read_failures >= MAX_UHID_READ_FAILURES {
                            tracing::error!("uhid read failing repeatedly, giving up: {e}");
                            return Err(e.into());
                        }
                        tracing::warn!("uhid read error ({read_failures}/{MAX_UHID_READ_FAILURES}): {e}");
                        tokio::time::sleep(UHID_READ_RETRY_DELAY).await;
                        continue;
                    }
                };
                match event {
                    UhidEvent::Output(report) => {
                        if report.len() >= HID_REPORT_LOG_PREFIX {
                            tracing::debug!(
                                cid = %hex::encode(&report[0..4]),
                                cmd = %format_args!("{:#04x}", report[4]),
                                "hid report in"
                            );
                        }
                        let (responses, action) = transport.handle_report(&report);
                        write_all(&device, responses, &mut write_failures).await;
                        match action {
                            Some(CtapAction::Relay(payload)) => {
                                let Some(cid) = transport.busy_channel() else { continue };
                                // A getNextAssertion addressed to the live
                                // multi-credential session on this channel
                                // continues that session instead of opening
                                // a new qr transaction.
                                if payload == [CMD_GET_NEXT_ASSERTION]
                                    && let Some(p) = &pending
                                    && matches!(p.phase, Phase::Multi)
                                    && p.cid() == cid
                                {
                                    if p.transaction.next.send(SessionCmd::Next).is_err() {
                                        tracing::warn!(
                                            "multi-credential session is gone; answering the follow-up"
                                        );
                                        let reports = transport.cancel_relay(cid);
                                        write_all(&device, reports, &mut write_failures).await;
                                        pending = None;
                                    }
                                    continue;
                                }
                                // An idle multi-credential session does not
                                // own the CTAPHID channel: the host has moved
                                // on, so abandon it and serve the new command
                                // instead of refusing a whole ceremony for
                                // state nobody will ask for again.
                                let idle_multi =
                                    pending.as_ref().is_some_and(|p| matches!(p.phase, Phase::Multi));
                                if idle_multi
                                    && let Some(p) = pending.take()
                                {
                                    p.transaction.abort.abort();
                                    notifier.hide();
                                    tracing::info!(
                                        "abandoned an idle multi-credential session \
                                         for a new request"
                                    );
                                }
                                // Once a blink probe has been answered the host
                                // keeps this channel and sends its preflight
                                // and then the real command on it, so those
                                // commands must not be refused as "busy".
                                let blink_answered = pending.as_ref().is_some_and(|p| {
                                    matches!(p.phase, Phase::Blink { answered: true })
                                        && p.cid() == cid
                                });
                                if pending.is_some() && !blink_answered {
                                    // A second single-frame CBOR command is
                                    // already rejected by the CTAPHID layer,
                                    // but a host can restart a multi-frame
                                    // assembly on the same channel: that
                                    // overwrites the pending one and, on
                                    // completion, clears `busy` first, so the
                                    // command reaches us while a transaction is
                                    // still pending. Refuse it and free the
                                    // slot, rather than dropping it and wedging
                                    // the channel until the host times out.
                                    tracing::warn!(
                                        "relay requested while a transaction is pending; refusing"
                                    );
                                    let reports = transport
                                        .complete_relay(cid, &[CTAP1_ERR_CHANNEL_BUSY]);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                // Follow-up to a faked preflight response that
                                // reported several credentials: serve the next
                                // one so Firefox keeps the whole allowList.
                                if payload == [CMD_GET_NEXT_ASSERTION] {
                                    let response = match fake_assertions.as_mut() {
                                        Some((rp, rest)) => match rest.pop_front() {
                                            Some(cred) => {
                                                ucabled::ctap::fake_next_assertion(rp, &cred)
                                            }
                                            None => vec![CTAP2_ERR_NOT_ALLOWED],
                                        },
                                        None => vec![CTAP2_ERR_NOT_ALLOWED],
                                    };
                                    let reports = transport.complete_relay(cid, &response);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                // Firefox probes with getAssertion up=false to
                                // filter the allowList to credentials present on
                                // this device. For a normal RP ID, answer locally
                                // with a fake success echoing the credential ID,
                                // so Firefox proceeds to the real interactive
                                // request without a pointless QR scan; an error
                                // answer here would make Firefox drop the device
                                // from the transaction entirely. The AppID probe
                                // below is the exception, where declining is
                                // instead the expected signal.
                                // Report *all* credentials and stash the rest for
                                // `getNextAssertion`: claiming only the first
                                // would narrow the real allowList to it.
                                if let Some((rp, response, rest)) =
                                    ucabled::ctap::answer_silent_probe(&payload)
                                {
                                    fake_assertions = None;
                                    // Firefox also sends this probe with the
                                    // legacy U2F AppID as the RP ID to discover
                                    // U2F credentials for the `appid` extension.
                                    // Claiming one would make Firefox address the
                                    // AppID for the real request, where the phone
                                    // (which stores passkeys by WebAuthn RP ID)
                                    // finds nothing. Decline so Firefox keeps the
                                    // WebAuthn RP ID.
                                    let response = if ucabled::ctap::is_u2f_appid(&rp) {
                                        tracing::info!("U2F AppID probe (up=false), declining");
                                        vec![CTAP2_ERR_NO_CREDENTIALS]
                                    } else {
                                        tracing::info!(
                                            credentials = rest.len() + 1,
                                            "silent probe (up=false), answering locally"
                                        );
                                        if !rest.is_empty() {
                                            fake_assertions = Some((rp, rest.into()));
                                        }
                                        response
                                    };
                                    let reports = transport.complete_relay(cid, &response);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                // The real request that follows an answered
                                // blink probe: the caBLE session is already
                                // open (and the phone connected), so hand the
                                // command to it instead of starting a second QR
                                // transaction.
                                if blink_answered {
                                    let Some(p) = pending.take() else { continue };
                                    // Any preflight state is done with once the
                                    // real request starts.
                                    fake_assertions = None;
                                    let cmd = payload.first().copied().unwrap_or(0);
                                    let command = prepare_command(payload);
                                    let sent =
                                        p.transaction.next.send(SessionCmd::Command(command));
                                    tracing::info!(
                                        cmd = %format_args!("{cmd:#04x}"),
                                        "relaying the request that followed the blink probe"
                                    );
                                    pending = Some(Pending {
                                        transaction: p.transaction,
                                        phase: Phase::Relay,
                                        deadline: None,
                                    });
                                    if sent.is_err() {
                                        tracing::warn!(
                                            "caBLE session is gone; refusing the request \
                                             that followed the blink probe"
                                        );
                                        pending = None;
                                        notifier.hide();
                                        let reports =
                                            transport.complete_relay(cid, &[CTAP1_ERR_TIMEOUT]);
                                        write_all(&device, reports, &mut write_failures).await;
                                    }
                                    continue;
                                }
                                // Firefox's dummy makeCredential is how it
                                // asks the user to pick among several
                                // authenticators ("Multiple devices found").
                                // It only counts as "selected" if the device
                                // answers with a Pin* status, which a physical
                                // key only does once touched.  Our device has
                                // nothing to touch, so show the QR code right
                                // away and answer Pin* only once the phone is
                                // connected: until then the user can still
                                // touch the other key, which makes Firefox
                                // cancel this probe.
                                if ucabled::ctap::is_blink_probe(&payload) {
                                    if blink_answered {
                                        // We are already selected; the host
                                        // has no reason to blink us again.
                                        tracing::debug!("ignoring a repeated blink probe");
                                        let reports = transport
                                            .complete_relay(cid, &[CTAP1_ERR_CHANNEL_BUSY]);
                                        write_all(&device, reports, &mut write_failures).await;
                                        continue;
                                    }
                                    // One atomic read: availability and the
                                    // generation tag share it (see below).
                                    let Some(ui) = notifier.ui_for_transaction() else {
                                        tracing::info!("blink probe, no UI available, declining");
                                        let reports = transport
                                            .complete_relay(cid, &[CTAP2_ERR_INVALID_OPTION]);
                                        write_all(&device, reports, &mut write_failures).await;
                                        continue;
                                    };
                                    let generation = next_generation;
                                    next_generation += 1;
                                    // The QR contents need a request type and a
                                    // command length, neither of which the probe
                                    // reveals about the real command: the probe
                                    // is itself a makeCredential, and its size
                                    // is the best available estimate for the
                                    // L2CAP MTU check.
                                    tracing::info!(
                                        "showing the QR code while Firefox asks which key to use"
                                    );
                                    let (abort, next) = spawn_session(
                                        &notifier,
                                        &result_tx,
                                        &ready_tx,
                                        cid,
                                        generation,
                                        payload.len(),
                                        RequestType::MakeCredential,
                                        None,
                                        BLINK_TIMEOUT.as_secs(),
                                    );
                                    pending = Some(Pending {
                                        transaction: Transaction {
                                            cid,
                                            generation,
                                            abort,
                                            next,
                                            agent: ui.generation(),
                                        },
                                        phase: Phase::Blink { answered: false },
                                        deadline: Some(
                                            tokio::time::Instant::now() + BLINK_TIMEOUT,
                                        ),
                                    });
                                    continue;
                                }
                                // Any preflight state is done with once the real
                                // request starts.
                                fake_assertions = None;
                                // One atomic read: availability and the
                                // generation tag share it, so an agent
                                // departing between the two cannot leave
                                // the transaction untagged.
                                let Some(ui) = notifier.ui_for_transaction() else {
                                    tracing::warn!(
                                        "no UI agent registered; refusing the transaction"
                                    );
                                    let reports = transport
                                        .complete_relay(cid, &[CTAP1_ERR_TIMEOUT]);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                };
                                let rp = ucabled::ctap::extract_rp_id(&payload);
                                tracing::info!(
                                    ?rp,
                                    cmd = %format_args!("{:#04x}", payload.first().copied().unwrap_or(0)),
                                    len = payload.len(),
                                    "starting caBLE transaction"
                                );
                                let payload = prepare_command(payload);
                                let request_type = if payload.first() == Some(&CMD_MAKE_CREDENTIAL) {
                                    RequestType::MakeCredential
                                } else {
                                    RequestType::GetAssertion
                                };
                                let generation = next_generation;
                                next_generation += 1;
                                let (abort, next) = spawn_session(
                                    &notifier,
                                    &result_tx,
                                    &ready_tx,
                                    cid,
                                    generation,
                                    payload.len(),
                                    request_type,
                                    rp,
                                    ucabled::relay::BLE_ADVERT_TIMEOUT.as_secs(),
                                );
                                if next.send(SessionCmd::Command(payload)).is_err() {
                                    tracing::warn!("caBLE session task is gone; refusing the request");
                                    abort.abort();
                                    let reports = transport
                                        .complete_relay(cid, &[CTAP1_ERR_TIMEOUT]);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                pending = Some(Pending {
                                    transaction: Transaction {
                                        cid,
                                        generation,
                                        abort,
                                        next,
                                        agent: ui.generation(),
                                    },
                                    phase: Phase::Relay,
                                    deadline: None,
                                });
                            }
                            Some(CtapAction::CancelRelay { cid: cancel_cid }) => {
                                // Only the channel the cancel arrived on may
                                // lose its transaction.
                                if pending.as_ref().is_none_or(|p| p.cid() != cancel_cid) {
                                    continue;
                                }
                                let Some(p) = pending.take() else { continue };
                                // Nothing is answered here: the CTAPHID layer
                                // already built the cancel response, and the
                                // phone learns when the tunnel drops.
                                p.transaction.abort.abort();
                                notifier.hide();
                                match p.phase {
                                    // Another authenticator (the user touched
                                    // the physical key) won the selection.
                                    Phase::Blink { .. } => {
                                        tracing::info!("device selection cancelled by the host")
                                    }
                                    Phase::Relay => {
                                        tracing::info!("cancelled caBLE transaction")
                                    }
                                    Phase::Multi => {
                                        tracing::info!("cancelled multi-credential session")
                                    }
                                }
                            }
                            None => {}
                        }
                    }
                    UhidEvent::Open => {
                        open_count += 1;
                        tracing::info!("hidraw opened by a process");
                        // Host activity is also the cheapest udev-event
                        // surrogate: re-audit now so a permission change
                        // since startup (or a fix) shows up in the journal.
                        let reason =
                            ucabled::uhid_perm::loose_reason(std::path::Path::new("/dev/uhid"));
                        match reason {
                            Ok(reason) if reason != uhid_loose => {
                                match &reason {
                                    Some(reason) => tracing::warn!(
                                        "/dev/uhid became writable by more than the ucabled \
                                         account ({reason}); check for a stale `uaccess` udev tag"
                                    ),
                                    None => tracing::info!(
                                        "/dev/uhid is no longer broadly writable; permissions \
                                         are tight"
                                    ),
                                }
                                uhid_loose = reason;
                            }
                            Ok(_) => {}
                            // A failed stat says nothing about the permissions;
                            // do not turn it into a "tight" transition.
                            Err(e) => {
                                tracing::debug!("cannot re-inspect /dev/uhid permissions: {e}");
                            }
                        }
                    }
                    UhidEvent::Close => {
                        if open_count == 0 {
                            // Unbalanced close: the count no longer
                            // reflects reality (an Open was missed), so do
                            // not reset — that could cut off a live client
                            // whose open was never counted.
                            tracing::warn!("hidraw close without a matching open");
                            continue;
                        }
                        open_count -= 1;
                        tracing::info!(open_count, "hidraw closed");
                        if open_count == 0 {
                            // The last host handle is gone, possibly
                            // mid-message or mid-transaction: drop the
                            // partial state so a fresh opener does not
                            // inherit a wedged channel, and stop any work
                            // nobody is left to read the answer of.  This
                            // includes the silent-probe preflight cache,
                            // which must not leak across host clients.
                            transport.reset();
                            fake_assertions = None;
                            if let Some(p) = pending.take() {
                                p.transaction.abort.abort();
                                notifier.hide();
                                tracing::info!(
                                    "last hidraw handle closed, dropped pending transaction"
                                );
                            }
                        }
                    }
                    UhidEvent::Start | UhidEvent::Stop => {}
                    UhidEvent::Other(t) => tracing::debug!(t, "unhandled uhid event"),
                }
            }
            Some(reply) = result_rx.recv() => {
                // A reply is only an answer to the transaction it was
                // started for: after a host cancel, the freed channel can
                // already carry a newer command with the same cid.
                if !reply_matches(pending.as_ref(), &reply) {
                    tracing::debug!(
                        cid = reply.cid,
                        generation = reply.generation,
                        "dropping a stale caBLE reply"
                    );
                    continue;
                }
                let cid = reply.cid;
                // While the blink probe is still unanswered the host is not
                // waiting for a CBOR answer: the only reply that can arrive is
                // a session that failed to open.  Decline the probe so another
                // authenticator can still be selected.
                let blink_unanswered = pending
                    .as_ref()
                    .is_some_and(|p| matches!(p.phase, Phase::Blink { answered: false }));
                if blink_unanswered {
                    if let Err(e) = &reply.payload {
                        tracing::warn!(
                            "caBLE session failed before the blink probe was answered: {e}"
                        );
                    }
                    pending = None;
                    notifier.hide();
                    let reports = transport.complete_relay(cid, &[CTAP2_ERR_INVALID_OPTION]);
                    write_all(&device, reports, &mut write_failures).await;
                    continue;
                }
                notifier.hide();
                let payload = match reply.payload {
                    Ok(p) => {
                        tracing::info!(
                            status = %format_args!("{:#04x}", p.first().copied().unwrap_or(0)),
                            len = p.len(),
                            "caBLE transaction completed"
                        );
                        p
                    }
                    Err(e) => {
                        tracing::warn!("caBLE transaction failed: {e}");
                        // A refused redirect means the tunnel server
                        // misbehaved — not a routine failure — so
                        // surface it to the user.
                        if e.is_redirect_refused() {
                            notifier.notify(
                                REDIRECT_REFUSED_SUMMARY,
                                REDIRECT_REFUSED_BODY,
                            );
                        }
                        vec![e.ctap_status()]
                    }
                };
                let reports = transport.complete_relay(cid, &payload);
                if !write_all(&device, reports, &mut write_failures).await {
                    // The host is not reading, so it will never assemble this
                    // reply.  Keeping the phone's assertion state (and a
                    // multi-credential session) alive for a reply nobody can
                    // use would only extend the tunnel's lifetime; drop it.
                    tracing::warn!("reply not delivered; dropping the transaction state");
                    if let Some(p) = pending.take() {
                        p.transaction.abort.abort();
                    }
                    continue;
                }
                // The phone still holds assertion state when more
                // credentials remain: keep the session open for
                // getNextAssertion follow-ups.
                pending = if reply.remaining > 0 {
                    pending.take().map(|p| Pending {
                        transaction: p.transaction,
                        phase: Phase::Multi,
                        deadline: None,
                    })
                } else {
                    None
                };
                if reply.remaining > 0 {
                    tracing::info!(remaining = reply.remaining, "multi-credential session open");
                }
            }
            // The phone has connected and the caBLE session is open: only now
            // can this device be selected, by answering the blink probe with a
            // Pin* status (CTAP2_ERR_PIN_NOT_SET).  Until this point a touch on
            // another security key makes Firefox cancel the probe instead.
            Some((ready_cid, ready_generation)) = ready_rx.recv() => {
                let ours = pending.as_ref().is_some_and(|p| {
                    matches!(p.phase, Phase::Blink { answered: false })
                        && p.cid() == ready_cid
                        && p.generation() == ready_generation
                });
                if !ours {
                    tracing::debug!(
                        cid = ready_cid,
                        generation = ready_generation,
                        "ignoring a stale session-ready signal"
                    );
                    continue;
                }
                let Some(p) = pending.as_mut() else { continue };
                p.phase = Phase::Blink { answered: true };
                let reports = transport.complete_relay(ready_cid, &[CTAP2_ERR_PIN_NOT_SET]);
                tracing::info!("phone connected, selecting this device");
                if !write_all(&device, reports, &mut write_failures).await {
                    tracing::warn!("selection answer not delivered; dropping the transaction");
                    if let Some(p) = pending.take() {
                        p.transaction.abort.abort();
                    }
                    notifier.hide();
                }
            }
            Some(request) = cancel_rx.recv() => {
                if !cancel_aborts(&request, pending.as_ref()) {
                    tracing::debug!(?request, "ignoring stale cancellation");
                    continue;
                }
                let Some(p) = pending.take() else {
                    continue;
                };
                let answered = matches!(p.phase, Phase::Blink { answered: true });
                p.transaction.abort.abort();
                notifier.hide();
                let reports = give_up_reports(&mut transport, &p);
                write_all(&device, reports, &mut write_failures).await;
                tracing::info!(?request, answered, "pending transaction cancelled");
            }
            // The keepalive cadence is only needed while a transaction is
            // pending or an unfinished assembly needs its idle-expiry check;
            // gating the branch keeps an idle daemon from waking ~7 times a
            // second for its whole lifetime.
            _ = keepalive.tick(), if pending.is_some() || transport.has_assembly() => {
                // A truncated multi-frame request must not wedge the busy
                // slot forever; hosts send continuation frames back to
                // back, so idle silence means the sender is gone.
                if transport.expire_assembly(std::time::Instant::now()) {
                    tracing::warn!("dropped an unfinished multi-frame request (idle timeout)");
                }
                // A multi-credential session whose task has exited (idle
                // timeout, tunnel failure) owns nothing any more; drop the
                // stale slot instead of letting it refuse the next command.
                let multi_gone = pending.as_ref().is_some_and(|p| {
                    matches!(p.phase, Phase::Multi) && p.transaction.next.is_closed()
                });
                if multi_gone {
                    tracing::info!("multi-credential session task is gone; dropping its slot");
                    pending = None;
                }
                let Some(p) = pending.as_ref() else { continue };
                let cid = p.cid();
                let expired = p
                    .deadline
                    .is_some_and(|deadline| tokio::time::Instant::now() >= deadline);
                if expired {
                    let Some(p) = pending.take() else { continue };
                    let answered = matches!(p.phase, Phase::Blink { answered: true });
                    p.transaction.abort.abort();
                    notifier.hide();
                    let reports = give_up_reports(&mut transport, &p);
                    write_all(&device, reports, &mut write_failures).await;
                    tracing::warn!(answered, "blink probe timed out, giving up");
                } else if transport.busy_channel() == Some(cid) {
                    // Keepalives tell the host its command is still being
                    // worked on.  An open multi-credential session between
                    // getNextAssertion calls has no command outstanding, so
                    // it must stay silent.
                    let reports = transport.keepalive(cid);
                    write_all(&device, reports, &mut write_failures).await;
                }
            }
        }
    }
}

/// Answer the host when the daemon gives up on a pending transaction: decline
/// a blink probe that is still unanswered (so Firefox keeps another
/// authenticator in the selection), cancel a command that is outstanding, or
/// write nothing when the host is not waiting for an answer on that channel.
fn give_up_reports(transport: &mut Transport, pending: &Pending) -> Vec<Vec<u8>> {
    let cid = pending.cid();
    if transport.busy_channel() != Some(cid) {
        return Vec::new();
    }
    match pending.phase {
        Phase::Blink { answered: false } => {
            transport.complete_relay(cid, &[CTAP2_ERR_INVALID_OPTION])
        }
        // A multi-credential session is usually idle between
        // getNextAssertion calls, in which case there is nothing to answer;
        // either way the task is aborted and the phone learns when the tunnel
        // drops.
        _ => transport.cancel_relay(cid),
    }
}

/// Make a CTAP command from the host ready to relay: iOS requires
/// `rp.name`/`user.displayName`, OpenSSH reuses one fixed user handle for every
/// sk key, and Firefox tags USB credentials with a transports hint some phones
/// reject.  The patchers borrow the input when nothing changes; the session
/// task needs the result across awaits, so it is owned.
fn prepare_command(payload: Vec<u8>) -> Vec<u8> {
    let payload = ucabled::ctap::patch_makecredential(&payload);
    let payload = ucabled::ctap::dissociate_ssh_user(&payload);
    let payload = ucabled::ctap::strip_transport_hints(&payload);
    payload.into_owned()
}

/// Open a caBLE session in its own task and drive it with CTAP commands.
///
/// The task shows the QR window, waits for the phone and the handshake, then
/// reports on `ready_tx` and relays the commands that arrive on the returned
/// channel.  Replies go back through `result_tx`.  The session is opened ahead
/// of the first command so that a blink probe can be answered only once the
/// phone is really there (see [`Phase::Blink`]).
#[allow(clippy::too_many_arguments)]
fn spawn_session(
    notifier: &Notifier,
    result_tx: &mpsc::UnboundedSender<RelayReply>,
    ready_tx: &mpsc::UnboundedSender<(u32, u64)>,
    cid: u32,
    generation: u64,
    command_len: usize,
    request_type: RequestType,
    rp: Option<String>,
    timeout_secs: u64,
) -> (AbortHandle, mpsc::UnboundedSender<SessionCmd>) {
    let (next_tx, mut next_rx) = mpsc::unbounded_channel::<SessionCmd>();
    let tx = result_tx.clone();
    let ready = ready_tx.clone();
    let notifier2 = notifier.clone();
    let notifier3 = notifier.clone();
    let task = tokio::spawn(async move {
        let opened = ucabled::relay::open_qr_session(
            command_len,
            request_type,
            move |url| notifier2.show(cid as u64, url, rp, timeout_secs, request_type),
            move || notifier3.phone_found(),
        )
        .await;
        let mut session = match opened {
            Ok(session) => session,
            Err(e) => {
                let _ = tx.send(RelayReply {
                    cid,
                    generation,
                    payload: Err(e),
                    remaining: 0,
                });
                return;
            }
        };
        // The tunnel is up and the phone is connected.  A blink probe is
        // answered from here on; a normal transaction has its command queued
        // already.
        let _ = ready.send((cid, generation));
        let first_command = match next_rx.recv().await {
            Some(SessionCmd::Command(command)) => command,
            // Aborted, or the daemon dropped the sender without a command.
            _ => return,
        };
        let first = session.ctap(&first_command).await;
        // A getAssertion reply that reports more than one credential leaves
        // the rest reachable only through getNextAssertion on this same
        // session.  Only a getAssertion can start one: key 5 means something
        // else in other replies.
        let mut remaining = match (&first, request_type) {
            (Ok(reply), RequestType::GetAssertion) => ucabled::ctap::number_of_credentials(reply)
                .unwrap_or(1)
                .saturating_sub(1)
                .min(MAX_NEXT_ASSERTIONS),
            _ => 0,
        };
        let sent = tx.send(RelayReply {
            cid,
            generation,
            payload: first,
            remaining,
        });
        if sent.is_err() {
            session.shutdown().await;
            return;
        }
        // Wait for the host to pick another account.  The account picker may
        // stay open for a while, so use the same budget as one CTAP reply;
        // after that the phone's assertion state is dropped.  The absolute
        // deadline keeps the idle window from being extended indefinitely.
        let session_deadline = tokio::time::Instant::now() + SESSION_MAX_LIFETIME;
        while remaining > 0 {
            let idle_deadline = tokio::time::Instant::now() + SESSION_IDLE_TIMEOUT;
            let cmd =
                tokio::time::timeout_at(idle_deadline.min(session_deadline), next_rx.recv()).await;
            match cmd {
                Ok(Some(SessionCmd::Next)) => {}
                _ => break,
            }
            let reply = session.ctap(&[CMD_GET_NEXT_ASSERTION]).await;
            // Only a successful assertion leaves another credential reachable;
            // a CTAP error status (e.g. 0x2e) means the phone's assertion state
            // is gone, so close the session instead of forwarding further
            // follow-ups.
            remaining = match &reply {
                Ok(p) if p.first() == Some(&ucabled::ctap::CTAP2_OK) => remaining.saturating_sub(1),
                _ => 0,
            };
            if tx
                .send(RelayReply {
                    cid,
                    generation,
                    payload: reply,
                    remaining,
                })
                .is_err()
            {
                break;
            }
        }
        session.shutdown().await;
        if remaining > 0 {
            tracing::info!("multi-credential session closed with credentials left");
        }
    });
    (task.abort_handle(), next_tx)
}

/// Write one response's reports to the host.
///
/// Each report is retried within [`WRITE_REPORT_TIMEOUT`], and the whole
/// response is bounded by [`WRITE_RESPONSE_TIMEOUT`].  When one still cannot
/// be written, the rest of the response is abandoned: writing later reports
/// after a gap would leave the host reassembling a message with a missing
/// frame.  The host learns about the truncated reply only from its own
/// transaction timeout (CTAPHID has no end marker); abandoning the rest
/// mainly frees the daemon and the HID channel for the next request.
async fn write_all(device: &UhidDevice, reports: Vec<Vec<u8>>, failures: &mut u32) -> bool {
    let total = reports.len();
    let response_deadline = tokio::time::Instant::now() + WRITE_RESPONSE_TIMEOUT;
    for (index, r) in reports.iter().enumerate() {
        let mut last_error = None;
        for _ in 0..WRITE_REPORT_ATTEMPTS {
            let attempt_deadline =
                (tokio::time::Instant::now() + WRITE_REPORT_TIMEOUT).min(response_deadline);
            match tokio::time::timeout_at(attempt_deadline, device.write_input(r)).await {
                Ok(Ok(())) => {
                    *failures = 0;
                    last_error = None;
                    break;
                }
                Ok(Err(e)) => last_error = Some(e),
                Err(_) => {
                    last_error = Some(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "uhid write timed out waiting for the host to read",
                    ));
                }
            }
        }
        if let Some(e) = last_error {
            *failures += 1;
            if *failures == 1 || failures.is_multiple_of(WRITE_FAILURE_LOG_EVERY) {
                tracing::warn!("uhid write failed ({failures} times): {e}");
            }
            if index + 1 < total {
                tracing::warn!(
                    written = index,
                    total,
                    "abandoning the rest of a response the host is not reading"
                );
            }
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blink_pending(cid: u32, agent: Option<u64>) -> Pending {
        Pending {
            transaction: Transaction {
                cid,
                generation: 2,
                abort: tokio::spawn(async {}).abort_handle(),
                next: mpsc::unbounded_channel().0,
                agent,
            },
            phase: Phase::Blink { answered: false },
            deadline: Some(tokio::time::Instant::now() + BLINK_TIMEOUT),
        }
    }

    #[tokio::test]
    async fn user_cancellation_matches_only_the_current_transaction() {
        let p = blink_pending(7, Some(1));
        assert!(cancel_aborts(&CancelRequest::Transaction(7), Some(&p)));
        // Stale tid from an earlier, already-finished transaction.
        assert!(!cancel_aborts(&CancelRequest::Transaction(6), Some(&p)));
        // Nothing pending at all.
        assert!(!cancel_aborts(&CancelRequest::Transaction(7), None));
    }

    #[tokio::test]
    async fn agent_gone_matches_only_the_owning_generation() {
        let p = blink_pending(7, Some(3));
        assert!(cancel_aborts(&CancelRequest::AgentGone(3), Some(&p)));
        // A departed predecessor or a not-yet-seen successor must not
        // abort this transaction.
        assert!(!cancel_aborts(&CancelRequest::AgentGone(2), Some(&p)));
        assert!(!cancel_aborts(&CancelRequest::AgentGone(4), Some(&p)));
        assert!(!cancel_aborts(&CancelRequest::AgentGone(3), None));
    }

    #[tokio::test]
    async fn agent_gone_does_not_touch_agentless_transactions() {
        // Terminal QR: no agent was involved, so no agent departure
        // applies.
        let p = blink_pending(7, None);
        assert!(!cancel_aborts(&CancelRequest::AgentGone(1), Some(&p)));
    }

    // The timer needs a Tokio time context to be constructed.
    #[tokio::test]
    async fn keepalives_use_delay_not_burst() {
        // Burst would replay every tick missed during an idle interval at
        // once, flooding the host with keepalive reports.
        assert_eq!(
            keepalive_interval().missed_tick_behavior(),
            tokio::time::MissedTickBehavior::Delay
        );
    }

    #[tokio::test]
    async fn a_stale_reply_does_not_match_the_current_transaction() {
        let (next, _rx) = mpsc::unbounded_channel();
        let abort = tokio::spawn(async {}).abort_handle();
        let pending = Pending {
            transaction: Transaction {
                cid: 7,
                generation: 2,
                abort,
                next,
                agent: None,
            },
            phase: Phase::Relay,
            deadline: None,
        };
        let reply = |cid, generation| RelayReply {
            cid,
            generation,
            payload: Ok(Vec::new()),
            remaining: 0,
        };

        assert!(reply_matches(Some(&pending), &reply(7, 2)));
        // The host cancelled and a new transaction reused channel 7: the
        // queued reply from generation 1 must not answer generation 2.
        assert!(!reply_matches(Some(&pending), &reply(7, 1)));
        assert!(!reply_matches(Some(&pending), &reply(8, 2)));
        assert!(!reply_matches(None, &reply(7, 2)));
    }
}
