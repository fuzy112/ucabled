// SPDX-License-Identifier: GPL-3.0-or-later

use std::fmt;
use std::sync::{Arc, OnceLock};

use anyhow::{anyhow, bail, Context, Result};
use futures::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderMap, Response, Uri};
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Bytes, Message};
use tokio_tungstenite::{
    connect_async_tls_with_config, Connector, MaybeTlsStream, WebSocketStream,
};

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

/// Assigned-domain IDs (PXP §6.1.1): indexes into
/// [`ASSIGNED_TUNNEL_DOMAINS`].
pub const ASSIGNED_DOMAIN_GOOGLE: u16 = 0;
pub const ASSIGNED_DOMAIN_APPLE: u16 = 1;

/// Domain IDs below this are assigned; at or above they name a hashed domain.
const ASSIGNED_DOMAIN_LIMIT: u16 = 256;
const HASHED_DOMAIN_PREFIX: &str = "cable.";
const BASE32_ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz234567";
const HASHED_TLDS: [&str; 4] = ["com", "org", "net", "info"];
/// The hashed domain input is NUL-terminated (PXP §6.1.1).
const HASHED_DOMAIN_TERMINATOR: u8 = 0;

/// Maximum number of HTTP redirect hops when dialling the tunnel.
const MAX_REDIRECTS: usize = 5;
/// HTTP statuses that trigger following a tunnel redirect.
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];

/// Upper bound for a single tunnel frame. CTAP payloads are at most
/// `ctaphid::MAX_PAYLOAD` (7609 B) plus framing, so 64 KiB is already
/// generous; the tunnel server is not trusted and must not be able to make
/// the daemon allocate arbitrary amounts of memory.
pub const MAX_FRAME_SIZE: usize = 64 * 1024;

/// How long to wait for the tunnel websocket handshake (per redirect hop).
pub const DIAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Websocket read limits for the tunnel connection.
///
/// [`read_binary`] rejects an oversized frame, but only after tungstenite has
/// already buffered it.  Without a limit at the parser the untrusted tunnel
/// server could make the daemon allocate up to tungstenite's 64 MiB default
/// before the check ever runs, which is exactly what [`MAX_FRAME_SIZE`] is
/// meant to prevent.  The defaults for the other fields are fine.
fn frame_config() -> WebSocketConfig {
    let mut config = WebSocketConfig::default();
    config.max_message_size = Some(MAX_FRAME_SIZE);
    config.max_frame_size = Some(MAX_FRAME_SIZE);
    config
}

/// Client TLS config for the tunnel: Mozilla roots, TLS 1.3 floor.
///
/// rustls is built without its default `tls12` feature, but feature
/// unification is fragile — any crate in the graph enabling rustls's
/// defaults would silently re-add TLS 1.2 — so the version floor is pinned
/// explicitly instead of resting on the feature set.  The roots are
/// compiled in and the config is immutable, so build it once.
fn tls_config() -> Arc<rustls::ClientConfig> {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let config =
                rustls::ClientConfig::builder_with_protocol_versions(&[&rustls::version::TLS13])
                    .with_root_certificates(root_store())
                    .with_no_client_auth();
            Arc::new(config)
        })
        .clone()
}

/// Mozilla's compiled-in roots, as rustls expects them.
fn root_store() -> rustls::RootCertStore {
    rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    }
}

/// The (scheme, host, port) of a tunnel URL.
///
/// The tunnel server is not trusted, so redirects must stay on the authority
/// the EID selected: an unchecked `Location` would let the server steer the
/// daemon's outbound connections at arbitrary hosts (blind SSRF) or downgrade
/// the transport to cleartext `ws://`.  Plain `ws` is accepted for the
/// initial URL only so the local test relay keeps working; the production
/// URLs built by [`new_tunnel_url`]/[`connect_url`] are always `wss`.
fn tunnel_authority(url: &str) -> Result<(String, String, u16)> {
    let uri = url
        .parse::<Uri>()
        .with_context(|| format!("invalid tunnel url {url}"))?;
    let scheme = uri.scheme_str().unwrap_or_default();
    if scheme != "wss" && scheme != "ws" {
        bail!("tunnel url must use ws(s)://");
    }
    let authority = uri
        .authority()
        .ok_or_else(|| anyhow!("tunnel url has no authority"))?;
    if authority.as_str().contains('@') {
        bail!("tunnel url must not carry userinfo");
    }
    let host = authority.host().to_lowercase();
    let port = authority
        .port_u16()
        .unwrap_or(if scheme == "wss" { 443 } else { 80 });
    Ok((scheme.to_string(), host, port))
}

/// The tunnel server answered with a redirect that failed re-validation
/// (different scheme/authority, userinfo, unparsable Location).  Unlike a
/// timeout this points at a misbehaving or hostile server, so the daemon
/// recognises it by type to warn the user.
#[derive(Debug)]
pub struct RedirectRefused(pub(crate) String);

impl fmt::Display for RedirectRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tunnel redirect refused: {}", self.0)
    }
}

impl std::error::Error for RedirectRefused {}

/// Follow a redirect only when it stays on the expected scheme and authority.
fn validate_redirect(
    expected: &(String, String, u16),
    location: &str,
) -> std::result::Result<String, RedirectRefused> {
    let got = tunnel_authority(location)
        .map_err(|e| RedirectRefused(format!("unparsable Location {location:?}: {e:#}")))?;
    if &got != expected {
        return Err(RedirectRefused(format!(
            "different authority {}://{}:{}",
            got.0, got.1, got.2
        )));
    }
    Ok(location.to_string())
}

pub fn decode_tunnel_server_domain(encoded: u16) -> Option<String> {
    if encoded < ASSIGNED_DOMAIN_LIMIT {
        return ASSIGNED_TUNNEL_DOMAINS
            .get(encoded as usize)
            .map(|s| s.to_string());
    }

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"caBLEv2 tunnel server domain");
    hasher.update(encoded.to_le_bytes());
    hasher.update([HASHED_DOMAIN_TERMINATOR]);
    let digest = hasher.finalize();

    let mut v = u64::from_le_bytes(digest[..8].try_into().unwrap());
    let tld_bits = (HASHED_TLDS.len() as u64).trailing_zeros();
    let tld_index = (v & ((1 << tld_bits) - 1)) as usize;
    v >>= tld_bits;

    let mut ret = String::from(HASHED_DOMAIN_PREFIX);
    while v != 0 {
        ret.push(BASE32_ALPHABET[(v & 31) as usize] as char);
        v >>= 5;
    }
    ret.push('.');
    ret.push_str(HASHED_TLDS[tld_index]);
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

// We advertise the caBLE subprotocol; a server that does not echo it back
// is not the endpoint we expect, and the framing is undefined, so refuse
// the connection.
fn check_subprotocol(headers: &HeaderMap) -> Result<()> {
    match headers
        .get("Sec-WebSocket-Protocol")
        .and_then(|v| v.to_str().ok())
    {
        Some(p) if p == WS_SUBPROTOCOL => Ok(()),
        other => Err(anyhow!(
            "tunnel server did not select subprotocol {WS_SUBPROTOCOL:?} (got {other:?})"
        )),
    }
}

pub async fn dial(url: &str) -> Result<(Ws, Response<Option<Vec<u8>>>)> {
    ensure_crypto_provider();
    // The spec requires following HTTP redirects; tokio-tungstenite does not,
    // so do it manually. The redirect target must stay on the EID-selected
    // authority; the tunnel server is not trusted to pick our connections.
    let expected = tunnel_authority(url)?;
    let mut url = url.to_string();
    for _ in 0..MAX_REDIRECTS {
        let mut request = url
            .as_str()
            .into_client_request()
            .with_context(|| format!("invalid tunnel url {url}"))?;
        request
            .headers_mut()
            .insert("Sec-WebSocket-Protocol", WS_SUBPROTOCOL.parse().unwrap());
        match tokio::time::timeout(
            DIAL_TIMEOUT,
            connect_async_tls_with_config(
                request,
                Some(frame_config()),
                false,
                Some(Connector::Rustls(tls_config())),
            ),
        )
        .await
        {
            Ok(Ok((ws, response))) => {
                check_subprotocol(response.headers())?;
                return Ok((ws, response));
            }
            Ok(Err(tokio_tungstenite::tungstenite::Error::Http(response))) => {
                let status = response.status();
                let location = response
                    .headers()
                    .get("Location")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                match (status.as_u16(), location) {
                    (code, Some(loc)) if REDIRECT_STATUSES.contains(&code) => {
                        match validate_redirect(&expected, &loc) {
                            Ok(next) => {
                                tracing::info!(%status, redirect = %next, "following tunnel redirect");
                                url = next;
                                continue;
                            }
                            // The known tunnel servers do not redirect across
                            // authorities; if that ever changes this log line
                            // is where the failed transactions come from.
                            Err(e) => {
                                tracing::warn!(%status, redirect = %loc, "{e}");
                                return Err(e.into());
                            }
                        }
                    }
                    _ => return Err(anyhow!("tunnel websocket HTTP error: {status}")),
                }
            }
            Ok(Err(e)) => return Err(anyhow!("tunnel websocket connect failed: {e}")),
            Err(_elapsed) => return Err(anyhow!("tunnel websocket connect timed out")),
        }
    }
    Err(anyhow!("too many tunnel redirects"))
}

pub async fn write_binary(ws: &mut Ws, data: Vec<u8>) -> Result<()> {
    ws.send(Message::Binary(data.into())).await?;
    Ok(())
}

pub async fn read_binary(ws: &mut Ws) -> Result<Bytes> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Binary(data))) => {
                if data.len() > MAX_FRAME_SIZE {
                    bail!("tunnel frame too large: {} bytes", data.len());
                }
                return Ok(data);
            }
            Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
            Some(Ok(Message::Close(_))) | None => bail!("tunnel websocket closed"),
            Some(Ok(other)) => bail!("unexpected websocket frame: {other:?}"),
            Some(Err(e)) => bail!("tunnel websocket error: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigned_domains() {
        assert_eq!(
            decode_tunnel_server_domain(0).as_deref(),
            Some("cable.ua5v.com")
        );
        assert_eq!(
            decode_tunnel_server_domain(1).as_deref(),
            Some("cable.auth.com")
        );
        assert!(decode_tunnel_server_domain(2).is_none());
        assert!(decode_tunnel_server_domain(3).is_none());
    }

    #[test]
    fn hashed_domains_are_deterministic() {
        let a = decode_tunnel_server_domain(256).unwrap();
        assert_eq!(a, decode_tunnel_server_domain(256).unwrap());
        assert!(a.starts_with("cable."));
        assert!([".com", ".org", ".net", ".info"]
            .iter()
            .any(|tld| a.ends_with(tld)));
        assert_ne!(a, decode_tunnel_server_domain(257).unwrap());
    }

    #[test]
    fn url_shapes() {
        assert_eq!(
            new_tunnel_url("cable.ua5v.com", &[0xab; 16]),
            format!("wss://cable.ua5v.com/cable/new/{}", "ab".repeat(16))
        );
        assert_eq!(
            connect_url("d.example", &[0x01, 0x02, 0x03], &[0x0f; 16]),
            format!("wss://d.example/cable/connect/010203/{}", "0f".repeat(16))
        );
    }

    #[test]
    fn redirect_to_same_authority_is_followed() {
        let expected = tunnel_authority("wss://cable.ua5v.com/cable/connect/010203/00").unwrap();
        let loc = "wss://cable.ua5v.com/cable/connect/010203/ff";
        assert_eq!(validate_redirect(&expected, loc).unwrap(), loc);
        // Explicit default port and host case differences still match.
        assert!(validate_redirect(&expected, "wss://CABLE.ua5v.com:443/x").is_ok());
        // The local test relay dials plain ws; same-scheme same-host is fine.
        let local = tunnel_authority("ws://127.0.0.1:8080/cable/connect/010203/00").unwrap();
        assert!(validate_redirect(&local, "ws://127.0.0.1:8080/other").is_ok());
        assert!(validate_redirect(&local, "ws://127.0.0.1:9090/other").is_err());
    }

    #[test]
    fn redirect_to_another_authority_is_rejected() {
        let expected = tunnel_authority("wss://cable.ua5v.com/cable/connect/010203/00").unwrap();
        for loc in [
            "wss://cable.auth.com/cable/connect/010203/ff",
            "wss://cable.ua5v.com:444/cable/connect/010203/ff",
            "wss://127.0.0.1/internal/admin",
        ] {
            assert!(validate_redirect(&expected, loc).is_err(), "{loc}");
        }
    }

    #[test]
    fn redirect_downgrade_and_userinfo_are_rejected() {
        let expected = tunnel_authority("wss://cable.ua5v.com/cable/connect/010203/00").unwrap();
        for loc in [
            "ws://cable.ua5v.com/cable/connect/010203/ff",
            "wss://user@cable.ua5v.com/cable/connect/010203/ff",
            "/cable/connect/010203/ff",
            "https://cable.ua5v.com/cable/connect/010203/ff",
        ] {
            assert!(validate_redirect(&expected, loc).is_err(), "{loc}");
        }
    }

    #[test]
    fn frame_config_caps_incoming_messages() {
        // The parser limit must be in place *before* read_binary's
        // post-allocation check, so an oversized message is refused while
        // being read rather than after it is buffered.
        let config = frame_config();
        assert_eq!(config.max_message_size, Some(MAX_FRAME_SIZE));
        assert_eq!(config.max_frame_size, Some(MAX_FRAME_SIZE));
    }

    #[test]
    fn tls_config_uses_the_compiled_in_roots() {
        // The compiled-in Mozilla roots must actually be loaded, and building
        // the client config (with the ring provider and TLS 1.3 only) must
        // not panic.
        assert!(root_store().roots.len() > 1);
        let _ = tls_config();
    }

    #[test]
    fn subprotocol_must_be_echoed() {
        let mut headers = HeaderMap::new();
        assert!(check_subprotocol(&headers).is_err());
        headers.insert("Sec-WebSocket-Protocol", "chat".parse().unwrap());
        assert!(check_subprotocol(&headers).is_err());
        headers.insert("Sec-WebSocket-Protocol", WS_SUBPROTOCOL.parse().unwrap());
        assert!(check_subprotocol(&headers).is_ok());
    }

    #[test]
    fn refused_redirect_is_a_typed_error() {
        let expected = tunnel_authority("wss://cable.ua5v.com/cable/connect/010203/00").unwrap();
        let err = validate_redirect(&expected, "wss://cable.auth.com/x").unwrap_err();
        // The daemon detects this through the anyhow chain, so the type
        // must survive the conversion.
        let any = anyhow::Error::new(err);
        assert!(any.chain().any(|c| c.is::<RedirectRefused>()));
    }
}
