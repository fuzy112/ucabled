// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::PublicKey;

use ucabled::phone;
use ucabled::qr;

struct Args {
    qr_url: String,
    tunnel_base: String,
    advertise: bool,
}

fn parse_args() -> Result<Args> {
    let mut qr_url = None;
    let mut tunnel_base = None;
    let mut advertise = false;
    let mut it = std::env::args().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--qr-url" => qr_url = Some(it.next().context("--qr-url needs a value")?),
            "--qr-url-file" => {
                let path = it.next().context("--qr-url-file needs a value")?;
                qr_url = Some(std::fs::read_to_string(path)?.trim().to_string());
            }
            "--tunnel-base" => {
                tunnel_base = Some(it.next().context("--tunnel-base needs a value")?)
            }
            "--advertise" => advertise = true,
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(Args {
        qr_url: qr_url.context("--qr-url is required")?,
        tunnel_base: tunnel_base.context("--tunnel-base is required")?,
        advertise,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let args = parse_args()?;

    let parsed = qr::parse_qr_url(&args.qr_url).context("failed to parse QR URL")?;
    let peer_public = PublicKey::from_sec1_bytes(&parsed.compressed_public_key)
        .context("bad public key in QR")?;
    let peer_identity = peer_public.to_encoded_point(false).as_bytes().to_vec();

    let (outcome, ws, psk) = phone::phone_setup(&args.tunnel_base, &parsed.secret, None).await?;

    if args.advertise {
        #[cfg(feature = "ble")]
        {
            let _handle = ucabled::ble::advertise(&outcome.advert).await?;
            println!("Advertising caBLE EID over BLE.");
        }
        #[cfg(not(feature = "ble"))]
        anyhow::bail!("built without BLE support; rebuild with --features ble");
    } else {
        println!("advert hex: {}", hex::encode(outcome.advert));
    }
    println!("plaintext EID: {}", hex::encode(outcome.plaintext_eid));
    println!("Tunnel created; waiting for desktop handshake...");

    let getinfo_reply = ucabled::ctap::mock_getinfo_response();
    phone::phone_run(ws, &psk, &peer_identity, &getinfo_reply).await?;
    Ok(())
}
