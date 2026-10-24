// SPDX-License-Identifier: GPL-3.0-or-later

mod common;

use p256::SecretKey;
use p256::elliptic_curve::Generate;
use p256::elliptic_curve::sec1::ToSec1Point;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use anyhow::Result;
use common::run_relay;
use ucabled::error::TransactionError;
use ucabled::phone;
use ucabled::qr::{self, RequestType};
use ucabled::session::DesktopFlow;

#[tokio::test]
async fn full_roundtrip_over_local_relay() -> Result<()> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let base = format!("ws://{addr}");
    tokio::spawn(run_relay(listener));

    let identity = SecretKey::generate();
    let compressed = identity.public_key().to_sec1_point(true);
    let compressed: &[u8; 33] = compressed.as_bytes().try_into().unwrap();
    let mut qr_secret = [0u8; 16];
    rand::fill(&mut qr_secret);

    let qr_url = qr::encode_qr_url(
        compressed,
        &qr_secret,
        2,
        false,
        RequestType::GetAssertion,
        &[qr::TRANSPORT_WEBSOCKET],
    );
    let parsed = qr::parse_qr_url(&qr_url).unwrap();
    assert_eq!(&parsed.compressed_public_key, compressed);
    assert_eq!(parsed.secret, qr_secret);

    let peer_public = p256::PublicKey::from_sec1_bytes(&parsed.compressed_public_key).unwrap();
    let peer_identity = peer_public.to_sec1_point(false).as_bytes().to_vec();

    let (eid_tx, eid_rx) = oneshot::channel::<[u8; 16]>();
    let phone_base = base.clone();
    let phone_task = tokio::spawn(async move {
        let r = async {
            let (outcome, ws, psk) = phone::phone_setup(&phone_base, &parsed.secret, None).await?;
            eid_tx
                .send(outcome.plaintext_eid)
                .map_err(|_| anyhow::anyhow!("eid send"))?;
            let getinfo_reply = vec![0xa1, 0x01, 0x41, 0x00];
            phone::phone_run(ws, &psk, &peer_identity, &getinfo_reply).await?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = &r {
            eprintln!("phone task failed: {e:#}");
        }
        r
    });

    let plaintext_eid = eid_rx.await?;

    let flow = DesktopFlow {
        tunnel_base: Some(base),
        channel: ucabled::session::Channel::Websocket,
        qr_secret,
        identity,
        plaintext_eid,
        handshake_timeout: ucabled::session::HANDSHAKE_RESPONSE_TIMEOUT,
        connect_timeout: ucabled::tunnel::DIAL_TIMEOUT,
    };
    let result = flow.run(&[0x04]).await?;

    phone_task.await??;

    assert_eq!(result.ctap_reply[0], 0x00, "expected CTAP success status");
    assert!(!result.post_handshake.is_empty());
    Ok(())
}

/// A phone (or tunnel server) that connects but never answers the handshake
/// must not hold the transaction forever: the desktop gives up after its
/// handshake timeout.
#[tokio::test]
async fn a_stalled_handshake_times_out() -> Result<()> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let base = format!("ws://{addr}");
    tokio::spawn(run_relay(listener));

    let identity = SecretKey::generate();
    let compressed = identity.public_key().to_sec1_point(true);
    let compressed: &[u8; 33] = compressed.as_bytes().try_into().unwrap();
    let mut qr_secret = [0u8; 16];
    rand::fill(&mut qr_secret);

    let qr_url = qr::encode_qr_url(
        compressed,
        &qr_secret,
        2,
        false,
        RequestType::GetAssertion,
        &[qr::TRANSPORT_WEBSOCKET],
    );
    let parsed = qr::parse_qr_url(&qr_url).unwrap();

    let (eid_tx, eid_rx) = oneshot::channel::<[u8; 16]>();
    let phone_base = base.clone();
    let phone_task = tokio::spawn(async move {
        let r = async {
            let (outcome, _ws, _psk) =
                phone::phone_setup(&phone_base, &parsed.secret, None).await?;
            eid_tx
                .send(outcome.plaintext_eid)
                .map_err(|_| anyhow::anyhow!("eid send"))?;
            // Stall: hold the tunnel open but never answer the handshake.
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = &r {
            eprintln!("phone task failed: {e:#}");
        }
        r
    });

    let plaintext_eid = eid_rx.await?;
    let flow = DesktopFlow {
        tunnel_base: Some(base),
        channel: ucabled::session::Channel::Websocket,
        qr_secret,
        identity,
        plaintext_eid,
        handshake_timeout: std::time::Duration::from_millis(500),
        connect_timeout: ucabled::tunnel::DIAL_TIMEOUT,
    };

    let start = std::time::Instant::now();
    let result = flow.run(&[0x04]).await;
    let elapsed = start.elapsed();
    phone_task.abort();

    let outcome = result.as_ref().map(|_| ()).map_err(|e| e.to_string());
    assert!(
        matches!(result, Err(TransactionError::Timeout)),
        "a stalled handshake must time out, got {outcome:?}"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "the stall was not bounded by the handshake timeout: {elapsed:?}"
    );
    Ok(())
}

/// Real phones keep the tunnel open after the transaction instead of closing
/// it. The desktop must still hand the CTAP reply back promptly: an earlier
/// version listened for linking update messages after shutdown, delaying
/// every operation by the whole listening window.
#[tokio::test]
async fn reply_is_not_delayed_by_a_lingering_phone() -> Result<()> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let base = format!("ws://{addr}");
    tokio::spawn(run_relay(listener));

    let identity = SecretKey::generate();
    let compressed = identity.public_key().to_sec1_point(true);
    let compressed: &[u8; 33] = compressed.as_bytes().try_into().unwrap();
    let mut qr_secret = [0u8; 16];
    rand::fill(&mut qr_secret);

    let qr_url = qr::encode_qr_url(
        compressed,
        &qr_secret,
        2,
        false,
        RequestType::GetAssertion,
        &[qr::TRANSPORT_WEBSOCKET],
    );
    let parsed = qr::parse_qr_url(&qr_url).unwrap();
    let peer_public = p256::PublicKey::from_sec1_bytes(&parsed.compressed_public_key).unwrap();
    let peer_identity = peer_public.to_sec1_point(false).as_bytes().to_vec();

    let (eid_tx, eid_rx) = oneshot::channel::<[u8; 16]>();
    let phone_base = base.clone();
    let phone_task = tokio::spawn(async move {
        let r = async {
            let (outcome, mut ws, psk) =
                phone::phone_setup(&phone_base, &parsed.secret, None).await?;
            eid_tx
                .send(outcome.plaintext_eid)
                .map_err(|_| anyhow::anyhow!("eid send"))?;

            let initial = ucabled::tunnel::read_binary(&mut ws).await?;
            let (response, crypter, handshake_hash) =
                ucabled::handshake::respond_qr(&psk, &peer_identity, &initial)
                    .ok_or_else(|| anyhow::anyhow!("handshake failed"))?;
            ucabled::tunnel::write_binary(&mut ws, response).await?;

            let mut link = phone::CableLink {
                transport: phone::CableTransport::Websocket(Box::new(ws)),
                crypter,
                handshake_hash,
            };
            // Post-handshake message; contents are not inspected by the desktop.
            let post = vec![0xa1, 0x01, 0x40];
            let ct = link
                .crypter
                .encrypt(&post)
                .ok_or_else(|| anyhow::anyhow!("encrypt failed"))?;
            link.transport.send(ct).await?;

            loop {
                match link.recv_message().await {
                    Ok((phone::MSG_CTAP, _)) => {
                        link.send_ctap(&[0x00, 0xa1, 0x01, 0x40]).await?;
                    }
                    Ok((phone::MSG_SHUTDOWN, _)) => {
                        // Linger like a real phone: keep the tunnel open.
                        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = &r {
            eprintln!("phone task failed: {e:#}");
        }
        r
    });

    let plaintext_eid = eid_rx.await?;
    let flow = DesktopFlow {
        tunnel_base: Some(base),
        channel: ucabled::session::Channel::Websocket,
        qr_secret,
        identity,
        plaintext_eid,
        handshake_timeout: ucabled::session::HANDSHAKE_RESPONSE_TIMEOUT,
        connect_timeout: ucabled::tunnel::DIAL_TIMEOUT,
    };

    let start = std::time::Instant::now();
    let result = flow.run(&[0x04]).await?;
    let elapsed = start.elapsed();
    phone_task.abort();

    assert_eq!(result.ctap_reply[0], 0x00, "expected CTAP success status");
    assert!(
        elapsed < std::time::Duration::from_secs(5),
        "CTAP reply waited for the lingering phone: {elapsed:?}"
    );
    Ok(())
}

/// Build one structurally valid getAssertion success reply (status byte +
/// CBOR).  `total` sets `numberOfCredentials`, as the phone does on the first
/// of several matching credentials.
fn assertion_reply(cred_id: &[u8], total: Option<u64>) -> Vec<u8> {
    let mut out = vec![ucabled::ctap::CTAP2_OK];
    ucabled::cbor::map(&mut out, if total.is_some() { 4 } else { 3 });
    // credential descriptor {"id": cred_id, "type": "public-key"}
    ucabled::cbor::uint(&mut out, 1);
    ucabled::cbor::map(&mut out, 2);
    ucabled::cbor::text(&mut out, "id");
    ucabled::cbor::bytes(&mut out, cred_id);
    ucabled::cbor::text(&mut out, "type");
    ucabled::cbor::text(&mut out, "public-key");
    // authData (rpIdHash || flags || counter) and a dummy signature.
    ucabled::cbor::uint(&mut out, 2);
    ucabled::cbor::bytes(&mut out, &[0u8; 37]);
    ucabled::cbor::uint(&mut out, 3);
    ucabled::cbor::bytes(&mut out, &[0u8; 8]);
    if let Some(total) = total {
        ucabled::cbor::uint(&mut out, 5);
        ucabled::cbor::uint(&mut out, total);
    }
    out
}

/// A getAssertion whose reply announces several credentials must keep the
/// caBLE session open: the host asks for the rest with
/// `authenticatorGetNextAssertion`, and those commands have to reach the same
/// phone session, which still holds the credential list.  A one-shot
/// transaction would answer them from an empty local cache instead.
#[tokio::test]
async fn multi_credential_get_assertion_stays_on_one_session() -> Result<()> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let base = format!("ws://{addr}");
    tokio::spawn(run_relay(listener));

    let identity = SecretKey::generate();
    let compressed = identity.public_key().to_sec1_point(true);
    let compressed: &[u8; 33] = compressed.as_bytes().try_into().unwrap();
    let mut qr_secret = [0u8; 16];
    rand::fill(&mut qr_secret);

    let qr_url = qr::encode_qr_url(
        compressed,
        &qr_secret,
        2,
        false,
        RequestType::GetAssertion,
        &[qr::TRANSPORT_WEBSOCKET],
    );
    let parsed = qr::parse_qr_url(&qr_url).unwrap();
    let peer_public = p256::PublicKey::from_sec1_bytes(&parsed.compressed_public_key).unwrap();
    let peer_identity = peer_public.to_sec1_point(false).as_bytes().to_vec();

    let (eid_tx, eid_rx) = oneshot::channel::<[u8; 16]>();
    let phone_base = base.clone();
    let phone_task = tokio::spawn(async move {
        let r = async {
            let (outcome, mut ws, psk) =
                phone::phone_setup(&phone_base, &parsed.secret, None).await?;
            eid_tx
                .send(outcome.plaintext_eid)
                .map_err(|_| anyhow::anyhow!("eid send"))?;

            let initial = ucabled::tunnel::read_binary(&mut ws).await?;
            let (response, crypter, handshake_hash) =
                ucabled::handshake::respond_qr(&psk, &peer_identity, &initial)
                    .ok_or_else(|| anyhow::anyhow!("handshake failed"))?;
            ucabled::tunnel::write_binary(&mut ws, response).await?;

            let mut link = phone::CableLink {
                transport: phone::CableTransport::Websocket(Box::new(ws)),
                crypter,
                handshake_hash,
            };
            // Post-handshake message; contents are not inspected.
            let ct = link
                .crypter
                .encrypt(&[0xa1, 0x01, 0x40])
                .ok_or_else(|| anyhow::anyhow!("encrypt failed"))?;
            link.transport.send(ct).await?;

            let first = assertion_reply(&[0xaa; 32], Some(2));
            let second = assertion_reply(&[0xbb; 32], None);
            loop {
                match link.recv_message().await {
                    Ok((phone::MSG_CTAP, command)) => {
                        let reply = match command.first() {
                            Some(&ucabled::ctap::CMD_GET_ASSERTION) => first.clone(),
                            Some(&ucabled::ctap::CMD_GET_NEXT_ASSERTION) => second.clone(),
                            other => anyhow::bail!("unexpected CTAP command {other:?}"),
                        };
                        link.send_ctap(&reply).await?;
                    }
                    Ok((phone::MSG_SHUTDOWN, _)) => break,
                    Ok(_) => {}
                    Err(e) => return Err(e),
                }
            }
            Ok::<_, anyhow::Error>(())
        }
        .await;
        if let Err(e) = &r {
            eprintln!("phone task failed: {e:#}");
        }
        r
    });

    let plaintext_eid = eid_rx.await?;
    let flow = DesktopFlow {
        tunnel_base: Some(base),
        channel: ucabled::session::Channel::Websocket,
        qr_secret,
        identity,
        plaintext_eid,
        handshake_timeout: ucabled::session::HANDSHAKE_RESPONSE_TIMEOUT,
        connect_timeout: ucabled::tunnel::DIAL_TIMEOUT,
    };

    let command = vec![ucabled::ctap::CMD_GET_ASSERTION];
    let mut session = flow.open(command.len()).await?;
    let reply = session.ctap(&command).await?;
    assert_eq!(
        ucabled::ctap::number_of_credentials(&reply),
        Some(2),
        "the first reply must report both credentials"
    );

    // The follow-up rides the same session and reaches the phone.
    let next = session
        .ctap(&[ucabled::ctap::CMD_GET_NEXT_ASSERTION])
        .await?;
    assert_eq!(next[0], ucabled::ctap::CTAP2_OK);
    assert_ne!(next, reply, "the follow-up must be a different assertion");
    assert_eq!(
        ucabled::ctap::number_of_credentials(&next),
        None,
        "only the first reply carries numberOfCredentials"
    );

    session.shutdown().await;
    phone_task.await??;
    Ok(())
}
