use std::time::Duration;

use anyhow::{Context, Result};
use rand::rngs::OsRng;
use rand::RngCore;
use tokio::sync::mpsc;
use tokio::task::AbortHandle;

use ucabled::ctaphid::{CtapAction, Transport};
use ucabled::qr::RequestType;
use ucabled::uhid_dev::{UhidDevice, UhidEvent, FIDO_REPORT_DESCRIPTOR};

const KEEPALIVE_INTERVAL: Duration = Duration::from_millis(150);

fn show_qr(url: &str) {
    tracing::debug!(url, "caBLE QR");
    println!("\n=== Scan with your phone (passkey) ===");
    match qrcode::QrCode::new(url.as_bytes()) {
        Ok(code) => {
            let image = code
                .render::<qrcode::render::unicode::Dense1x2>()
                .dark_color(qrcode::render::unicode::Dense1x2::Dark)
                .light_color(qrcode::render::unicode::Dense1x2::Light)
                .build();
            println!("{image}");
        }
        Err(e) => println!("QR render failed: {e}\n{url}"),
    }
    println!("=======================================\n");
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut aaguid = [0u8; 16];
    OsRng.fill_bytes(&mut aaguid);

    let device = UhidDevice::create("Phone Passkey Bridge", &FIDO_REPORT_DESCRIPTOR)
        .context("failed to create uhid device (is /dev/uhid accessible?)")?;
    tracing::info!("virtual FIDO2 device registered as 'Phone Passkey Bridge'");
    println!("ucabled ready: virtual FIDO2 device registered (build with caBLE relay)");

    let mut transport = Transport::new(aaguid);
    let (result_tx, mut result_rx) = mpsc::unbounded_channel::<(u32, Result<Vec<u8>>)>();
    let mut pending: Option<(u32, AbortHandle)> = None;
    let mut keepalive = tokio::time::interval(KEEPALIVE_INTERVAL);

    loop {
        tokio::select! {
            event = device.next_event() => {
                let event = match event {
                    Ok(e) => e,
                    Err(e) => {
                        tracing::error!("uhid error: {e}");
                        return Err(e.into());
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
                        for r in responses {
                            device.write_input(&r).await?;
                        }
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
                                    for r in transport.complete_relay(cid, &response) {
                                        device.write_input(&r).await?;
                                    }
                                    continue;
                                }
                                // Firefox's dummy makeCredential exists only to
                                // make a physical key blink; don't bug the phone.
                                if ucabled::ctap::is_blink_probe(&payload) {
                                    tracing::info!("blink probe, answering locally");
                                    for r in transport.complete_relay(cid, &[0x2c]) {
                                        device.write_input(&r).await?;
                                    }
                                    continue;
                                }
                                let rp = ucabled::ctap::extract_rp_id(&payload);
                                tracing::info!(?rp, "starting caBLE transaction");
                                // iOS requires rp.name / user.displayName;
                                // inject them when Firefox omitted them.
                                let payload = ucabled::ctap::patch_makecredential(&payload);
                                tracing::debug!(payload = %hex::encode(&payload), "CTAP relay payload");
                                let request_type = if payload.first() == Some(&0x01) {
                                    RequestType::MakeCredential
                                } else {
                                    RequestType::GetAssertion
                                };
                                let tx = result_tx.clone();
                                let task = tokio::spawn(async move {
                                    let r = relay_or_log(payload, request_type).await;
                                    let _ = tx.send((cid, r));
                                });
                                pending = Some((cid, task.abort_handle()));
                            }
                            Some(CtapAction::CancelRelay) => {
                                if let Some((_, abort)) = pending.take() {
                                    abort.abort();
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
                                tracing::warn!("caBLE transaction failed: {e:#}");
                                vec![0x2e] // CTAP2_ERR_NO_CREDENTIALS
                            }
                        };
                        for r in transport.complete_relay(cid, &payload) {
                            device.write_input(&r).await?;
                        }
                        pending = None;
                    }
                }
            }
            _ = keepalive.tick(), if pending.is_some() => {
                if let Some((cid, _)) = &pending {
                    for r in transport.keepalive(*cid) {
                        device.write_input(&r).await?;
                    }
                }
            }
        }
    }
}

async fn relay_or_log(payload: Vec<u8>, request_type: RequestType) -> Result<Vec<u8>> {
    ucabled::relay::run_qr_transaction(&payload, request_type, show_qr).await
}
