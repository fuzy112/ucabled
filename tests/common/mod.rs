// SPDX-License-Identifier: GPL-3.0-or-later

//! A minimal in-process caBLE tunnel relay shared by the e2e tests.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use futures::{SinkExt, StreamExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;

type ServerWs = WebSocketStream<TcpStream>;
type PendingTunnels = Arc<Mutex<HashMap<String, ServerWs>>>;

#[allow(clippy::result_large_err)] // signature dictated by tungstenite's callback
pub async fn run_relay(listener: TcpListener) {
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
