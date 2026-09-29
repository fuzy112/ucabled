use anyhow::{bail, Context, Result};
use p256::SecretKey;

use crate::eid;
use crate::handshake::HandshakeInitiator;
use crate::kdf::{derive, Purpose};
use crate::phone::{CableLink, MSG_CTAP, MSG_SHUTDOWN, MSG_UPDATE};
use crate::tunnel::{self, decode_tunnel_server_domain};

pub struct DesktopFlow {
    /// e.g. "wss://cable.ua5v.com" or "ws://127.0.0.1:9000" for tests.
    pub tunnel_base: Option<String>,
    pub qr_secret: [u8; 16],
    pub identity: SecretKey,
    pub plaintext_eid: [u8; eid::EID_PLAINTEXT_SIZE],
}

/// Result of a completed desktop-side caBLE session.
pub struct DesktopResult {
    /// Raw CBOR of the post-handshake message from the phone.
    pub post_handshake: Vec<u8>,
    /// First CTAP reply payload (status byte + CBOR) received from the phone.
    pub ctap_reply: Vec<u8>,
    pub handshake_hash: [u8; 32],
    /// Update messages received after the CTAP reply (linking data etc.).
    pub updates: Vec<Vec<u8>>,
}

impl DesktopFlow {
    /// Run the desktop side: connect tunnel, perform Noise handshake, read the
    /// post-handshake message, send one CTAP command, return its reply.
    pub async fn run(self, ctap_command: &[u8]) -> Result<DesktopResult> {
        let components = eid::to_components(&self.plaintext_eid);
        let domain = decode_tunnel_server_domain(components.tunnel_server_domain)
            .context("unknown tunnel server domain")?;

        let tunnel_base = self
            .tunnel_base
            .unwrap_or_else(|| format!("wss://{domain}"));

        let mut tunnel_id = [0u8; 16];
        derive(&self.qr_secret, &[], Purpose::TunnelId, &mut tunnel_id);
        let mut psk = [0u8; 32];
        derive(&self.qr_secret, &self.plaintext_eid, Purpose::Psk, &mut psk);

        let url = format!(
            "{tunnel_base}/cable/connect/{}/{}",
            hex::encode(components.routing_id),
            hex::encode(tunnel_id)
        );
        tracing::info!(url, "connecting caBLE tunnel");
        let (mut ws, _response) = tunnel::dial(&url).await?;

        let mut handshake = HandshakeInitiator::new_qr(&psk, &self.identity);
        let initial = handshake.build_initial_message();
        tunnel::write_binary(&mut ws, initial).await?;

        let response = tunnel::read_binary(&mut ws).await?;
        let (crypter, handshake_hash) = handshake
            .process_response(&response)
            .context("caBLE handshake failed")?;
        tracing::info!("caBLE handshake complete");

        let mut link = CableLink {
            ws,
            crypter,
            handshake_hash,
        };

        let post_handshake = recv_raw(&mut link).await?;
        tracing::info!(
            len = post_handshake.len(),
            hex = hex::encode(&post_handshake),
            "received post-handshake message"
        );

        link.send_ctap(ctap_command).await?;

        let ctap_reply = loop {
            let (ty, payload) = link.recv_message().await?;
            match ty {
                MSG_CTAP => break payload,
                MSG_UPDATE => {
                    tracing::info!(len = payload.len(), "received update message");
                }
                MSG_SHUTDOWN => bail!("unexpected shutdown from authenticator"),
                other => bail!("unexpected message type {other}"),
            }
        };

        link.send_shutdown().await.ok();

        // After a transaction the phone may send linking data in update
        // messages (if the QR advertised supports_linking). Keep listening
        // briefly; the phone closes the tunnel when done.
        let updates = collect_updates(&mut link, std::time::Duration::from_secs(15)).await;

        Ok(DesktopResult {
            post_handshake,
            ctap_reply,
            handshake_hash,
            updates,
        })
    }
}

async fn collect_updates(link: &mut CableLink, window: std::time::Duration) -> Vec<Vec<u8>> {
    let mut updates = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    loop {
        match tokio::time::timeout_at(deadline, link.recv_message()).await {
            Ok(Ok((MSG_UPDATE, payload))) => {
                tracing::info!(len = payload.len(), hex = hex::encode(&payload), "update message");
                updates.push(payload);
            }
            Ok(Ok(_)) => {}
            _ => break,
        }
    }
    updates
}

async fn recv_raw(link: &mut CableLink) -> Result<Vec<u8>> {
    let ct = tunnel::read_binary(&mut link.ws).await?;
    link.crypter.decrypt(&ct).context("decrypt failed")
}
