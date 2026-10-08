// SPDX-License-Identifier: GPL-3.0-or-later

//! Diagnostic: show a caBLE QR that offers the CTAP 2.3 BLE data channel and
//! report whether the phone's BLE advert carries an L2CAP PSM. This is how we
//! check whether a given phone (e.g. iPhone) supports the offline transport.
//!
//! It only observes the proximity advert; it does not connect. If the advert
//! has no suffix/PSM, the phone did not offer the BLE channel.

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    #[cfg(not(feature = "ble"))]
    anyhow::bail!("built without BLE support; rebuild with --features ble");
    #[cfg(feature = "ble")]
    run().await
}

#[cfg(feature = "ble")]
async fn run() -> Result<()> {
    use anyhow::Context;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use p256::SecretKey;
    use rand::rngs::OsRng;
    use rand::RngCore;
    use std::time::Duration;
    use ucabled::kdf::{derive, Purpose};
    use ucabled::qr::{self, RequestType};
    use ucabled::{eid, NUM_ASSIGNED_TUNNEL_DOMAINS};

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let mut timeout = Duration::from_secs(300);
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--timeout-secs" => {
                let secs: u64 = it.next().context("--timeout-secs needs a value")?.parse()?;
                timeout = Duration::from_secs(secs);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }

    let identity = SecretKey::random(&mut OsRng);
    let compressed = identity.public_key().to_encoded_point(true);
    let compressed: &[u8; qr::COMPRESSED_PUBLIC_KEY_SIZE] =
        compressed.as_bytes().try_into().unwrap();
    let mut qr_secret = [0u8; qr::QR_SECRET_SIZE];
    OsRng.fill_bytes(&mut qr_secret);

    let qr_url = qr::encode_qr_url(
        compressed,
        &qr_secret,
        NUM_ASSIGNED_TUNNEL_DOMAINS,
        false,
        RequestType::GetAssertion,
        &[qr::TRANSPORT_WEBSOCKET, qr::TRANSPORT_BLE],
    );
    println!("caBLE QR URL: {qr_url}\n");
    let code = qrcode::QrCode::new(qr_url.as_bytes())?;
    let image = code
        .render::<qrcode::render::unicode::Dense1x2>()
        .dark_color(qrcode::render::unicode::Dense1x2::Dark)
        .light_color(qrcode::render::unicode::Dense1x2::Light)
        .build();
    println!("{image}");
    println!("Offering transport channels [0 (WebSocket), 1 (BLE)].");
    println!(
        "Scan with the phone; waiting up to {}s for its advert...\n",
        timeout.as_secs()
    );

    let mut eid_key = [0u8; eid::EID_KEY_SIZE];
    derive(&qr_secret, &[], Purpose::EidKey, &mut eid_key);

    let obs = ucabled::ble::await_advert_full(&eid_key, timeout).await?;
    println!("plaintext EID: {}", hex::encode(obs.plaintext_eid));
    println!("peer address:  {} ({:?})", obs.address, obs.address_type);
    println!("service data:  {} bytes", obs.service_data.len());
    println!("  raw: {}", hex::encode(&obs.service_data));
    match obs.psm {
        Some(psm) => println!("BLE L2CAP: offered (server PSM 0x{psm:04x})"),
        None => println!("BLE L2CAP: not offered (no advertisement suffix / PSM)"),
    }
    Ok(())
}
