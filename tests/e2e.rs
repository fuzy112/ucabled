use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use futures::{SinkExt, StreamExt};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::SecretKey;
use rand::rngs::OsRng;
use rand::RngCore;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

use ucabled::phone;
use ucabled::qr::{self, RequestType};
use ucabled::session::DesktopFlow;

type ServerWs = WebSocketStream<TcpStream>;
type PendingTunnels = Arc<Mutex<HashMap<String, ServerWs>>>;

#[allow(clippy::result_large_err)] // signature dictated by tungstenite's callback
async fn run_relay(listener: TcpListener) {
    let pending: PendingTunnels = Arc::new(Mutex::new(HashMap::new()));
    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let pending = pending.clone();
        tokio::spawn(async move {
            let mut path = String::new();
            let ws = match tokio_tungstenite::accept_hdr_async(
                stream,
                |req: &Request, mut resp: Response| {
                    path = req.uri().path().to_string();
                    resp.headers_mut()
                        .insert("X-caBLE-Routing-Id", "02aa55".parse().unwrap());
                    if req.headers().get("Sec-WebSocket-Protocol").is_some() {
                        resp.headers_mut().insert(
                            "Sec-WebSocket-Protocol",
                            ucabled::WS_SUBPROTOCOL.parse().unwrap(),
                        );
                    }
                    Ok(resp)
                },
            )
            .await
            {
                Ok(ws) => ws,
                Err(e) => {
                    eprintln!("relay accept failed: {e}");
                    return;
                }
            };

            if let Some(id) = path.strip_prefix("/cable/new/") {
                pending.lock().unwrap().insert(id.to_string(), ws);
            } else if let Some(rest) = path.strip_prefix("/cable/connect/") {
                let id = rest.split('/').nth(1).unwrap_or("").to_string();
                let other = pending.lock().unwrap().remove(&id);
                match other {
                    Some(other) => pipe(ws, other).await,
                    None => eprintln!("relay: unknown tunnel {id}"),
                }
            } else {
                eprintln!("relay: unexpected path {path}");
            }
        });
    }
}

async fn pipe(a: ServerWs, b: ServerWs) {
    let (a_tx, a_rx) = a.split();
    let (b_tx, b_rx) = b.split();
    let fwd = |mut rx: futures::stream::SplitStream<ServerWs>,
               mut tx: futures::stream::SplitSink<ServerWs, Message>| async move {
        while let Some(msg) = rx.next().await {
            match msg {
                Ok(m) => {
                    if tx.send(m).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        // Propagate teardown so the peer sees a close instead of hanging.
        let _ = tx.send(Message::Close(None)).await;
    };
    tokio::join!(fwd(a_rx, b_tx), fwd(b_rx, a_tx));
}

#[tokio::test]
async fn full_roundtrip_over_local_relay() -> Result<()> {
    let _ = tracing_subscriber::fmt().with_test_writer().try_init();

    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let base = format!("ws://{addr}");
    tokio::spawn(run_relay(listener));

    let identity = SecretKey::random(&mut OsRng);
    let compressed = identity.public_key().to_encoded_point(true);
    let compressed: &[u8; 33] = compressed.as_bytes().try_into().unwrap();
    let mut qr_secret = [0u8; 16];
    OsRng.fill_bytes(&mut qr_secret);

    let qr_url = qr::encode_qr_url(compressed, &qr_secret, 2, false, RequestType::GetAssertion);
    let parsed = qr::parse_qr_url(&qr_url).unwrap();
    assert_eq!(&parsed.compressed_public_key, compressed);
    assert_eq!(parsed.secret, qr_secret);

    let peer_public = p256::PublicKey::from_sec1_bytes(&parsed.compressed_public_key).unwrap();
    let peer_identity = peer_public.to_encoded_point(false).as_bytes().to_vec();

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
        qr_secret,
        identity,
        plaintext_eid,
    };
    let result = flow.run(&[0x04]).await?;

    phone_task.await??;

    assert_eq!(result.ctap_reply[0], 0x00, "expected CTAP success status");
    assert!(!result.post_handshake.is_empty());
    Ok(())
}
