use anyhow::{anyhow, bail, Context, Result};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::Response;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, MaybeTlsStream, WebSocketStream};

use crate::WS_SUBPROTOCOL;

fn ensure_crypto_provider() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    });
}

pub type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub const ASSIGNED_TUNNEL_DOMAINS: [&str; 2] = ["cable.ua5v.com", "cable.auth.com"];

pub fn decode_tunnel_server_domain(encoded: u16) -> Option<String> {
    if encoded < 256 {
        return ASSIGNED_TUNNEL_DOMAINS
            .get(encoded as usize)
            .map(|s| s.to_string());
    }

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"caBLEv2 tunnel server domain");
    hasher.update(encoded.to_le_bytes());
    hasher.update([0u8]);
    let digest = hasher.finalize();

    let mut v = u64::from_le_bytes(digest[..8].try_into().unwrap());
    let tld_index = (v & 3) as usize;
    v >>= 2;

    const BASE32: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut ret = String::from("cable.");
    while v != 0 {
        ret.push(BASE32[(v & 31) as usize] as char);
        v >>= 5;
    }
    ret.push('.');
    ret.push_str(["com", "org", "net", "info"][tld_index]);
    Some(ret)
}

pub fn new_tunnel_url(domain: &str, tunnel_id: &[u8; 16]) -> String {
    format!("wss://{domain}/cable/new/{}", hex::encode(tunnel_id))
}

pub fn connect_url(domain: &str, routing_id: &[u8; 3], tunnel_id: &[u8; 16]) -> String {
    format!(
        "wss://{domain}/cable/connect/{}/{}",
        hex::encode(routing_id),
        hex::encode(tunnel_id)
    )
}

pub async fn dial(url: &str) -> Result<(Ws, Response<Option<Vec<u8>>>)> {
    ensure_crypto_provider();
    // The spec requires following HTTP redirects; tokio-tungstenite does not,
    // so do it manually.
    let mut url = url.to_string();
    for _ in 0..5 {
        let mut request = url
            .as_str()
            .into_client_request()
            .with_context(|| format!("invalid tunnel url {url}"))?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", WS_SUBPROTOCOL.parse().unwrap());
        match connect_async(request).await {
            Ok(ok) => return Ok(ok),
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                let status = response.status();
                let location = response
                    .headers()
                    .get("Location")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                match (status.as_u16(), location) {
                    (301 | 302 | 303 | 307 | 308, Some(loc)) => {
                        tracing::info!(%status, redirect = %loc, "following tunnel redirect");
                        url = loc;
                        continue;
                    }
                    _ => return Err(anyhow!("tunnel websocket HTTP error: {status}")),
                }
            }
            Err(e) => return Err(anyhow!("tunnel websocket connect failed: {e}")),
        }
    }
    Err(anyhow!("too many tunnel redirects"))
}

pub async fn write_binary(ws: &mut Ws, data: Vec<u8>) -> Result<()> {
    ws.send(Message::Binary(data.into())).await?;
    Ok(())
}

pub async fn read_binary(ws: &mut Ws) -> Result<Vec<u8>> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(data))) => return Ok(data.to_vec()),
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | None => bail!("tunnel websocket closed"),
            Some(Ok(other)) => bail!("unexpected websocket frame: {other:?}"),
            Some(Err(e)) => bail!("tunnel websocket error: {e}"),
        }
    }
}
