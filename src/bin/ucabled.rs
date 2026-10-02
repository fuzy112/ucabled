// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use ucabled::ctap::{
    CMD_GET_NEXT_ASSERTION, CMD_MAKE_CREDENTIAL, CTAP1_ERR_TIMEOUT, CTAP2_ERR_INVALID_OPTION,
    CTAP2_ERR_NOT_ALLOWED, CTAP2_ERR_NO_CREDENTIALS, CTAP2_ERR_PIN_NOT_SET,
};
use ucabled::ctaphid::{CtapAction, Transport};
use ucabled::qr::RequestType;
use ucabled::uhid_dev::{UhidDevice, UhidEvent, FIDO_REPORT_DESCRIPTOR};
use ucabled::ui::Notifier;

const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(150);

/// Delay before retrying a transient uhid read error.
const UHID_READ_RETRY_DELAY: Duration = Duration::from_millis(100);

/// Log every Nth consecutive write failure (the first is always logged).
const WRITE_FAILURE_LOG_EVERY: u32 = 100;

/// Bytes of an input report needed to log its CID and command (cid + cmd).
const HID_REPORT_LOG_PREFIX: usize = 5;

/// Consecutive uhid read failures tolerated before giving up (and letting
/// systemd restart the service and recreate the device).
const MAX_UHID_READ_FAILURES: u32 = 10;

/// How long to wait for the user to answer the device-selection prompt before
/// declining. The helper has its own, slightly shorter, timeout.
const SELECT_TIMEOUT: Duration = Duration::from_secs(70);

/// Bound on queued UI cancellations. Cancellations carry the transaction id
/// and are only acted on when they match the in-flight transaction, so a
/// full queue can only ever mean a misbehaving agent; excess ones are
/// dropped in the D-Bus layer.
const CANCEL_QUEUE_SIZE: usize = 16;

/// Bound on queued device-selection answers, same rationale as
/// [`CANCEL_QUEUE_SIZE`].
const SELECT_QUEUE_SIZE: usize = 16;

/// An in-flight operation that the daemon is waiting on.
enum Pending {
    /// A caBLE transaction is running for `cid`; `abort` cancels it.
    Relay { cid: u32, abort: AbortHandle },
    /// The user is being asked to choose the phone over another
    /// authenticator (e.g. a physical security key). Firefox's own prompt can
    /// only be answered by touching a physical key, so we ask through the
    /// agent.
    Select {
        cid: u32,
        deadline: tokio::time::Instant,
    },
}

impl Pending {
    fn cid(&self) -> u32 {
        match self {
            Pending::Relay { cid, .. } | Pending::Select { cid, .. } => *cid,
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let no_ui = std::env::args().any(|a| a == "--no-ui");

    let (cancel_tx, cancel_rx) = mpsc::channel::<u64>(CANCEL_QUEUE_SIZE);
    let (select_tx, select_rx) = mpsc::channel::<(u64, bool)>(SELECT_QUEUE_SIZE);
    let rt = tokio::runtime::Runtime::new()?;
    let notifier = if no_ui {
        Notifier::Terminal
    } else {
        match ucabled::agent::start(rt.handle(), cancel_tx.clone(), select_tx.clone()) {
            Ok(ui) => Notifier::Agent(ui),
            Err(e) => {
                tracing::warn!("UI bridge unavailable ({e:#}), falling back to terminal QR");
                Notifier::Terminal
            }
        }
    };

    rt.block_on(daemon_loop(notifier, cancel_rx, select_rx))
}

async fn daemon_loop(
    notifier: Notifier,
    mut cancel_rx: mpsc::Receiver<u64>,
    mut select_rx: mpsc::Receiver<(u64, bool)>,
) -> Result<()> {
    let device = UhidDevice::create("Phone Passkey Bridge", &FIDO_REPORT_DESCRIPTOR)
        .context("failed to create uhid device (is /dev/uhid accessible?)")?;
    tracing::info!("virtual FIDO2 device registered as 'Phone Passkey Bridge'");

    let mut transport = Transport::new(ucabled::ctap::AAGUID);
    let (result_tx, mut result_rx) =
        mpsc::unbounded_channel::<(u32, Result<Vec<u8>, ucabled::error::TransactionError>)>();
    let mut pending: Option<Pending> = None;
    // Credentials still to serve via `authenticatorGetNextAssertion` after a
    // faked preflight response that reported more than one credential. Kept as
    // (rpId, remaining credential ids).
    let mut fake_assertions: Option<(String, std::collections::VecDeque<Vec<u8>>)> = None;
    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);
    let mut read_failures = 0u32;
    let mut write_failures = 0u32;

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
                                if pending.is_some() {
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
                                // this device. Answer locally with a fake success
                                // echoing the credential ID, so Firefox proceeds
                                // to the real interactive request without a
                                // pointless QR scan. (Error answers make Firefox
                                // drop the device from the transaction entirely.)
                                // Report *all* credentials and stash the rest for
                                // `getNextAssertion`: claiming only the first
                                // would narrow the real allowList to it.
                                if ucabled::ctap::is_silent_probe(&payload) {
                                    fake_assertions = None;
                                    let (response, rest) =
                                        match ucabled::ctap::fake_silent_assertion(&payload) {
                                            Some((response, rest)) => (response, rest),
                                            None => (vec![CTAP2_ERR_NO_CREDENTIALS], Vec::new()),
                                        };
                                    tracing::info!(
                                        credentials = rest.len() + 1,
                                        "silent probe (up=false), answering locally"
                                    );
                                    if !rest.is_empty() {
                                        let rp = ucabled::ctap::extract_rp_id(&payload)
                                            .unwrap_or_default();
                                        fake_assertions = Some((rp, rest.into()));
                                    }
                                    let reports = transport.complete_relay(cid, &response);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                // Firefox's dummy makeCredential is how it
                                // asks the user to pick among several
                                // authenticators ("Multiple devices found").
                                // It only counts as "selected" if the device
                                // answers with a Pin* status, which a physical
                                // key only does once touched. Put the choice in
                                // the user's hands with our own window.
                                if ucabled::ctap::is_blink_probe(&payload) {
                                    if notifier.select(cid as u64) {
                                        tracing::info!("asking the user to choose the phone");
                                        pending = Some(Pending::Select {
                                            cid,
                                            deadline: tokio::time::Instant::now() + SELECT_TIMEOUT,
                                        });
                                    } else {
                                        tracing::info!("blink probe, no UI available, declining");
                                        let reports = transport.complete_relay(cid, &[CTAP2_ERR_INVALID_OPTION]);
                                        write_all(&device, reports, &mut write_failures).await;
                                    }
                                    continue;
                                }
                                // Any preflight state is done with once the real
                                // request starts.
                                fake_assertions = None;
                                if !notifier.available() {
                                    tracing::warn!(
                                        "no UI agent registered; refusing the transaction"
                                    );
                                    let reports = transport
                                        .complete_relay(cid, &[CTAP1_ERR_TIMEOUT]);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                let rp = ucabled::ctap::extract_rp_id(&payload);
                                let transports_hint =
                                    ucabled::ctap::request_has_transport_hints(&payload);
                                tracing::info!(
                                    ?rp,
                                    cmd = %format_args!("{:#04x}", payload.first().copied().unwrap_or(0)),
                                    len = payload.len(),
                                    transports_hint,
                                    "starting caBLE transaction"
                                );
                                // iOS requires rp.name / user.displayName;
                                // inject them when Firefox omitted them.
                                let payload = ucabled::ctap::patch_makecredential(&payload);
                                // Firefox tags USB credentials with a
                                // transports hint that some phones reject.
                                let payload = ucabled::ctap::strip_transport_hints(&payload);
                                let request_type = if payload.first() == Some(&CMD_MAKE_CREDENTIAL) {
                                    RequestType::MakeCredential
                                } else {
                                    RequestType::GetAssertion
                                };
                                let tx = result_tx.clone();
                                let notifier2 = notifier.clone();
                                let notifier3 = notifier.clone();
                                let task = tokio::spawn(async move {
                                    let r = ucabled::relay::run_qr_transaction(
                                        &payload,
                                        request_type,
                                        move |url| {
                                            notifier2.show(
                                                cid as u64,
                                                url,
                                                rp,
                                                ucabled::relay::BLE_ADVERT_TIMEOUT.as_secs(),
                                            )
                                        },
                                        move || notifier3.phone_found(),
                                    )
                                    .await;
                                    let _ = tx.send((cid, r));
                                });
                                pending = Some(Pending::Relay {
                                    cid,
                                    abort: task.abort_handle(),
                                });
                            }
                            Some(CtapAction::CancelRelay) => match pending.take() {
                                Some(Pending::Relay { abort, .. }) => {
                                    abort.abort();
                                    notifier.hide();
                                    tracing::info!("cancelled caBLE transaction");
                                }
                                Some(Pending::Select { .. }) => {
                                    // The host cancelled because another
                                    // authenticator (the user touched it) won
                                    // the selection; drop our prompt.
                                    notifier.hide();
                                    tracing::info!("device selection cancelled by the host");
                                }
                                None => {}
                            },
                            None => {}
                        }
                    }
                    UhidEvent::Open => tracing::info!("hidraw opened by a process"),
                    UhidEvent::Close => tracing::info!("hidraw closed"),
                    UhidEvent::Start | UhidEvent::Stop => {}
                    UhidEvent::Other(t) => tracing::debug!(t, "unhandled uhid event"),
                }
            }
            Some((cid, result)) = result_rx.recv() => {
                if let Some(Pending::Relay { cid: pending_cid, .. }) = &pending {
                    if *pending_cid == cid {
                        notifier.hide();
                        let payload = match result {
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
                                vec![e.ctap_status()]
                            }
                        };
                        let reports = transport.complete_relay(cid, &payload);
                        write_all(&device, reports, &mut write_failures).await;
                        pending = None;
                    }
                }
            }
            Some((tid, use_phone)) = select_rx.recv() => {
                if let Some(Pending::Select { cid, .. }) = &pending {
                    if *cid as u64 == tid {
                        let cid = *cid;
                        pending = None;
                        // CTAP2_ERR_PIN_NOT_SET: on the blink probe a Pin*
                        // status tells authenticator-rs that this device was
                        // selected, so Firefox proceeds with the phone flow.
                        let status = if use_phone { CTAP2_ERR_PIN_NOT_SET } else { CTAP2_ERR_INVALID_OPTION };
                        let reports = transport.complete_relay(cid, &[status]);
                        write_all(&device, reports, &mut write_failures).await;
                        tracing::info!(use_phone, "device selection answered");
                    }
                }
            }
            Some(tid) = cancel_rx.recv() => {
                // Cancellations are tagged with the transaction id the agent
                // was prompted for; a stale one (e.g. from a window closed
                // after its transaction already ended) must not abort a
                // later, unrelated transaction.
                match pending.take() {
                    Some(Pending::Relay { cid, abort }) if cid as u64 == tid => {
                        abort.abort();
                        let reports = transport.cancel_relay(cid);
                        write_all(&device, reports, &mut write_failures).await;
                        tracing::info!("transaction cancelled from UI");
                    }
                    Some(Pending::Select { cid, .. }) if cid as u64 == tid => {
                        let reports =
                            transport.complete_relay(cid, &[CTAP2_ERR_INVALID_OPTION]);
                        write_all(&device, reports, &mut write_failures).await;
                        tracing::info!("device selection cancelled from UI");
                    }
                    other => {
                        pending = other;
                        tracing::debug!(tid, "ignoring stale cancellation");
                    }
                }
            }
            _ = keepalive.tick(), if pending.is_some() => {
                let cid = pending.as_ref().map(Pending::cid).unwrap();
                let timed_out = matches!(
                    pending.as_ref(),
                    Some(Pending::Select { deadline, .. })
                        if tokio::time::Instant::now() >= *deadline
                );
                if timed_out {
                    pending = None;
                    let reports = transport.complete_relay(cid, &[CTAP2_ERR_INVALID_OPTION]);
                    write_all(&device, reports, &mut write_failures).await;
                    tracing::warn!("device selection timed out, declining");
                } else {
                    let reports = transport.keepalive(cid);
                    write_all(&device, reports, &mut write_failures).await;
                }
            }
        }
    }
}

/// Write response reports to the host, tolerating transient failures so a
/// hiccup does not kill an in-flight transaction. Persistent failure is
/// handled by the read path, which eventually restarts the daemon.
async fn write_all(device: &UhidDevice, reports: Vec<Vec<u8>>, failures: &mut u32) {
    for r in reports {
        match device.write_input(&r).await {
            Ok(()) => *failures = 0,
            Err(e) => {
                *failures += 1;
                if *failures == 1 || failures.is_multiple_of(WRITE_FAILURE_LOG_EVERY) {
                    tracing::warn!("uhid write failed ({failures} times): {e}");
                }
            }
        }
    }
}
