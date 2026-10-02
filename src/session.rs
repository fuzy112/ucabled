// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{Context, Result};
use p256::SecretKey;
use std::time::Duration;

use crate::eid;
use crate::error::TransactionError;
use crate::handshake::HandshakeInitiator;
use crate::kdf::{derive, Purpose};
use crate::phone::{CableLink, CableTransport, MSG_CTAP, MSG_SHUTDOWN, MSG_UPDATE};
use crate::tunnel::{self, decode_tunnel_server_domain};

/// The phone sends the post-handshake message right after the handshake.
const POST_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
/// Upper bound for the phone's answer to one CTAP command; the user may still
/// be confirming on the phone. Firefox gives the whole operation 5 minutes.
const CTAP_REPLY_TIMEOUT: Duration = Duration::from_secs(300);

/// Which data transfer channel the authenticator chose.
pub enum Channel {
    /// The default WebSocket tunnel server path.
    Websocket,
    /// The CTAP 2.3 optional BLE data channel: an LE L2CAP CoC identified by
    /// the server PSM from the advertisement suffix.
    #[cfg(feature = "l2cap")]
    L2cap {
        address: bluer::Address,
        address_type: bluer::AddressType,
        psm: u16,
    },
}

pub struct DesktopFlow {
    /// e.g. "wss://cable.ua5v.com" or "ws://127.0.0.1:9000" for tests.
    pub tunnel_base: Option<String>,
    /// Channel selected from the BLE advert (WebSocket unless it carried a PSM).
    pub channel: Channel,
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
    pub async fn run(mut self, ctap_command: &[u8]) -> Result<DesktopResult, TransactionError> {
        let components = eid::to_components(&self.plaintext_eid);

        let mut tunnel_id = [0u8; 16];
        derive(&self.qr_secret, &[], Purpose::TunnelId, &mut tunnel_id);
        let mut psk = [0u8; 32];
        derive(&self.qr_secret, &self.plaintext_eid, Purpose::Psk, &mut psk);
        // The QR secret is no longer needed once the tunnel ID and PSK exist.
        crate::secure_erase(&mut self.qr_secret);

        let mut transport = self.connect_transport(&components, &tunnel_id).await?;

        let mut handshake = HandshakeInitiator::new_qr(&psk, &self.identity);
        // The PSK now lives inside the handshake state.
        crate::secure_erase(&mut psk);
        let initial = handshake.build_initial_message();
        transport
            .send(initial)
            .await
            .map_err(TransactionError::transport)?;

        let response = transport
            .recv()
            .await
            .map_err(TransactionError::transport)?;
        let (crypter, handshake_hash) = handshake
            .process_response(&response)
            .ok_or_else(|| TransactionError::failed(anyhow::anyhow!("caBLE handshake failed")))?;
        tracing::info!("caBLE handshake complete");

        let mut link = CableLink {
            transport,
            crypter,
            handshake_hash,
        };

        let post_handshake = tokio::time::timeout(POST_HANDSHAKE_TIMEOUT, recv_raw(&mut link))
            .await
            .map_err(|_| TransactionError::Timeout)?
            .map_err(TransactionError::transport)?;
        tracing::info!(
            len = post_handshake.len(),
            "received post-handshake message"
        );

        link.send_ctap(ctap_command)
            .await
            .map_err(TransactionError::transport)?;

        // A single deadline for the whole reply wait: the tunnel server is not
        // trusted and must not be able to keep the transaction alive forever
        // by trickling update messages.
        let reply_deadline = tokio::time::Instant::now() + CTAP_REPLY_TIMEOUT;
        let ctap_reply = loop {
            let (ty, payload) = tokio::time::timeout_at(reply_deadline, link.recv_message())
                .await
                .map_err(|_| TransactionError::Timeout)?
                .map_err(TransactionError::transport)?;
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

    /// Open the data transfer channel chosen by the authenticator.
    async fn connect_transport(
        &self,
        components: &eid::EidComponents,
        tunnel_id: &[u8; 16],
    ) -> Result<CableTransport, TransactionError> {
        match &self.channel {
            Channel::Websocket => {
                let domain = decode_tunnel_server_domain(components.tunnel_server_domain)
                    .ok_or_else(|| {
                        TransactionError::failed(anyhow::anyhow!("unknown tunnel server domain"))
                    })?;
                let tunnel_base = self
                    .tunnel_base
                    .clone()
                    .unwrap_or_else(|| format!("wss://{domain}"));
                let url = format!(
                    "{tunnel_base}/cable/connect/{}/{}",
                    hex::encode(components.routing_id),
                    hex::encode(tunnel_id)
                );
                tracing::info!("connecting caBLE tunnel");
                let (ws, _response) = tunnel::dial(&url)
                    .await
                    .map_err(TransactionError::transport)?;
                Ok(CableTransport::Websocket(Box::new(ws)))
            }
            #[cfg(feature = "l2cap")]
            Channel::L2cap {
                address,
                address_type,
                psm,
            } => {
                tracing::info!(%address, psm, "connecting caBLE L2CAP channel");
                let stream = crate::l2cap::connect(*address, *address_type, *psm)
                    .await
                    .map_err(TransactionError::transport)?;
                Ok(CableTransport::L2cap(stream))
            }
        }
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
    let ct = link.transport.recv().await?;
    link.crypter.decrypt(&ct).context("decrypt failed")
}
