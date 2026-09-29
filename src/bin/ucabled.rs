use std::time::Duration;

use anyhow::{Context, Result};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use ucabled::ctaphid::{CtapAction, Transport};
use ucabled::qr::RequestType;
use ucabled::uhid_dev::{UhidDevice, UhidEvent, FIDO_REPORT_DESCRIPTOR};
use ucabled::ui::Notifier;

const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(150);

/// Consecutive uhid read failures tolerated before giving up (and letting
/// systemd restart the service and recreate the device).
const MAX_UHID_READ_FAILURES: u32 = 10;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let no_ui = std::env::args().any(|a| a == "--no-ui");

    let (cancel_tx, cancel_rx) = mpsc::unbounded_channel::<()>();
    let notifier = if no_ui {
        Notifier::Terminal
    } else {
        match ucabled::agent::start(cancel_tx.clone()) {
            Ok(ui) => Notifier::Agent(ui),
            Err(e) => {
                tracing::warn!("UI bridge unavailable ({e:#}), falling back to terminal QR");
                Notifier::Terminal
            }
        }
    };

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(daemon_loop(notifier, cancel_rx))
}

async fn daemon_loop(notifier: Notifier, mut cancel_rx: mpsc::UnboundedReceiver<()>) -> Result<()> {
    let device = UhidDevice::create("Phone Passkey Bridge", &FIDO_REPORT_DESCRIPTOR)
        .context("failed to create uhid device (is /dev/uhid accessible?)")?;
    tracing::info!("virtual FIDO2 device registered as 'Phone Passkey Bridge'");

    let mut transport = Transport::new(ucabled::ctap::AAGUID);
    let (result_tx, mut result_rx) =
        mpsc::unbounded_channel::<(u32, Result<Vec<u8>, ucabled::error::TransactionError>)>();
    let mut pending: Option<(u32, AbortHandle)> = None;
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
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                match event {
                    UhidEvent::Output(report) => {
                        if report.len() >= 5 {
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
                                // Firefox probes with getAssertion up=false to
                                // filter the allowList to credentials present on
                                // this device. Answer locally with a fake success
                                // echoing the credential ID, so Firefox proceeds
                                // to the real interactive request without a
                                // pointless QR scan. (Error answers make Firefox
                                // drop the device from the transaction entirely.)
                                if ucabled::ctap::is_silent_probe(&payload) {
                                    let response = ucabled::ctap::fake_silent_assertion(&payload)
                                        .unwrap_or_else(|| vec![0x2e]);
                                    tracing::info!("silent probe (up=false), answering locally");
                                    let reports = transport.complete_relay(cid, &response);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                // Firefox's dummy makeCredential exists only to
                                // make a physical key blink; don't bug the phone.
                                if ucabled::ctap::is_blink_probe(&payload) {
                                    tracing::info!("blink probe, answering locally");
                                    let reports = transport.complete_relay(cid, &[0x2c]);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                if !notifier.available() {
                                    tracing::warn!(
                                        "no UI agent registered; refusing the transaction"
                                    );
                                    let reports = transport
                                        .complete_relay(cid, &[ucabled::error::CTAP1_ERR_TIMEOUT]);
                                    write_all(&device, reports, &mut write_failures).await;
                                    continue;
                                }
                                let rp = ucabled::ctap::extract_rp_id(&payload);
                                tracing::info!(
                                    ?rp,
                                    cmd = %format_args!("{:#04x}", payload.first().copied().unwrap_or(0)),
                                    len = payload.len(),
                                    "starting caBLE transaction"
                                );
                                // iOS requires rp.name / user.displayName;
                                // inject them when Firefox omitted them.
                                let payload = ucabled::ctap::patch_makecredential(&payload);
                                let request_type = if payload.first() == Some(&0x01) {
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
                                pending = Some((cid, task.abort_handle()));
                            }
                            Some(CtapAction::CancelRelay) => {
                                if let Some((_, abort)) = pending.take() {
                                    abort.abort();
                                    notifier.hide();
                                    tracing::info!("cancelled caBLE transaction");
                                }
                            }
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
                if let Some((pending_cid, _)) = &pending {
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
            Some(()) = cancel_rx.recv(), if pending.is_some() => {
                if let Some((cid, abort)) = pending.take() {
                    abort.abort();
                    let reports = transport.cancel_relay(cid);
                    write_all(&device, reports, &mut write_failures).await;
                    tracing::info!("transaction cancelled from UI");
                }
            }
            _ = keepalive.tick(), if pending.is_some() => {
                if let Some((cid, _)) = &pending {
                    let reports = transport.keepalive(*cid);
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
                if *failures == 1 || failures.is_multiple_of(100) {
                    tracing::warn!("uhid write failed ({failures} times): {e}");
                }
            }
        }
    }
}
