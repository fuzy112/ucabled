use anyhow::{Context, Result};
use p256::SecretKey;

use crate::eid;
use crate::error::TransactionError;
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
    /// True only when the QR advertised `supports_linking`, i.e. the phone may
    /// send linking data in update messages after the transaction. Collecting
    /// those messages means staying on the tunnel after the CTAP reply, which
    /// must never delay that reply, so this defaults to false.
    pub supports_linking: bool,
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
    pub async fn run(self, ctap_command: &[u8]) -> Result<DesktopResult, TransactionError> {
        let components = eid::to_components(&self.plaintext_eid);
        let domain = decode_tunnel_server_domain(components.tunnel_server_domain)
            .ok_or_else(|| TransactionError::failed(anyhow::anyhow!("unknown tunnel server domain")))?;

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
        tracing::info!("connecting caBLE tunnel");
        let (mut ws, _response) = tunnel::dial(&url)
            .await
            .map_err(TransactionError::transport)?;

        let mut handshake = HandshakeInitiator::new_qr(&psk, &self.identity);
        let initial = handshake.build_initial_message();
        tunnel::write_binary(&mut ws, initial)
            .await
            .map_err(TransactionError::transport)?;

        let response = tunnel::read_binary(&mut ws)
            .await
            .map_err(TransactionError::transport)?;
        let (crypter, handshake_hash) = handshake
            .process_response(&response)
            .ok_or_else(|| TransactionError::failed(anyhow::anyhow!("caBLE handshake failed")))?;
        tracing::info!("caBLE handshake complete");

        let mut link = CableLink {
            ws,
            crypter,
            handshake_hash,
        };

        let post_handshake = recv_raw(&mut link).await.map_err(TransactionError::transport)?;
        tracing::info!(len = post_handshake.len(), "received post-handshake message");

        link.send_ctap(ctap_command)
            .await
            .map_err(TransactionError::transport)?;

        let ctap_reply = loop {
            let (ty, payload) = link.recv_message().await.map_err(TransactionError::transport)?;
            match ty {
                MSG_CTAP => break payload,
                MSG_UPDATE => {
                    tracing::info!(len = payload.len(), "received update message");
                }
                MSG_SHUTDOWN => {
                    return Err(TransactionError::failed(anyhow::anyhow!(
                        "unexpected shutdown from authenticator"
                    )))
                }
                other => {
                    return Err(TransactionError::failed(anyhow::anyhow!(
                        "unexpected message type {other}"
                    )))
                }
            }
        };

        link.send_shutdown().await.ok();

        // Only when linking was advertised is it worth staying on the tunnel
        // to catch update messages; the CTAP reply above is already complete
        // and is returned as soon as this function returns.
        let updates = if self.supports_linking {
            collect_updates(&mut link, std::time::Duration::from_secs(15)).await
        } else {
            Vec::new()
        };

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
                tracing::info!(len = payload.len(), "received update message");
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
