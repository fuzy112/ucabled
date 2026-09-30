// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use rand::rngs::OsRng;
use rand::RngCore;

use crate::cbor;
use crate::ctap;
use crate::crypter::Crypter;
use crate::eid;
use crate::handshake;
use crate::kdf::{derive, Purpose};
use crate::tunnel::{self, Ws};

pub const MSG_SHUTDOWN: u8 = 0;
pub const MSG_CTAP: u8 = 1;
pub const MSG_UPDATE: u8 = 2;

pub struct CableLink {
    pub ws: Ws,
    pub crypter: Crypter,
    pub handshake_hash: [u8; 32],
}

impl CableLink {
    pub async fn send_message(&mut self, msg_type: u8, payload: &[u8]) -> Result<()> {
        let mut msg = vec![msg_type];
        msg.extend_from_slice(payload);
        let ct = self.crypter.encrypt(&msg).context("encrypt failed")?;
        tunnel::write_binary(&mut self.ws, ct).await
    }

    pub async fn recv_message(&mut self) -> Result<(u8, Vec<u8>)> {
        let ct = tunnel::read_binary(&mut self.ws).await?;
        let pt = self.crypter.decrypt(&ct).context("decrypt failed")?;
        if pt.is_empty() {
            bail!("empty cable message");
        }
        Ok((pt[0], pt[1..].to_vec()))
    }

    pub async fn send_ctap(&mut self, payload: &[u8]) -> Result<()> {
        self.send_message(MSG_CTAP, payload).await
    }

    pub async fn send_shutdown(&mut self) -> Result<()> {
        self.send_message(MSG_SHUTDOWN, &[]).await
    }
}

pub struct PhoneOutcome {
    pub advert: [u8; eid::ADVERT_SIZE],
    pub plaintext_eid: [u8; eid::EID_PLAINTEXT_SIZE],
}

/// Phone-side tunnel setup after scanning a QR code: derive keys, create the
/// tunnel, and produce the BLE advert contents. Returns the advert plus the
/// websocket so the caller can complete the handshake.
pub async fn phone_setup(
    tunnel_base: &str,
    qr_secret: &[u8; 16],
    routing_id_from_server: Option<[u8; 3]>,
) -> Result<(PhoneOutcome, Ws, [u8; 32])> {
    let mut tunnel_id = [0u8; 16];
    derive(qr_secret, &[], Purpose::TunnelId, &mut tunnel_id);

    let mut eid_key = [0u8; eid::EID_KEY_SIZE];
    derive(qr_secret, &[], Purpose::EidKey, &mut eid_key);

    let url = format!("{tunnel_base}/cable/new/{}", hex::encode(tunnel_id));
    let (ws, response) = tunnel::dial(&url).await?;

    let routing_id = match routing_id_from_server {
        Some(id) => id,
        None => {
            let header = response
                .headers()
                .get("X-caBLE-Routing-Id")
                .context("tunnel server did not specify routing ID")?
                .to_str()?;
            let bytes = hex::decode(header.trim())?;
            <[u8; 3]>::try_from(bytes.as_slice()).context("routing ID wrong size")?
        }
    };

    let mut nonce = [0u8; eid::NONCE_SIZE];
    OsRng.fill_bytes(&mut nonce);
    let components = eid::EidComponents {
        nonce,
        routing_id,
        tunnel_server_domain: tunnel::ASSIGNED_DOMAIN_GOOGLE,
    };
    let plaintext_eid = eid::plaintext_from_components(&components);
    let advert = eid::encrypt(&plaintext_eid, &eid_key);

    let mut psk = [0u8; 32];
    derive(qr_secret, &plaintext_eid, Purpose::Psk, &mut psk);

    Ok((
        PhoneOutcome {
            advert,
            plaintext_eid,
        },
        ws,
        psk,
    ))
}

/// Complete the phone side: answer the handshake, send the post-handshake
/// message with a canned getInfo reply, then echo CTAP requests back with a
/// canned error so the exchange can be observed end to end.
pub async fn phone_run(
    mut ws: Ws,
    psk: &[u8; 32],
    peer_identity_x962: &[u8],
    getinfo_reply: &[u8],
) -> Result<()> {
    let initial = tunnel::read_binary(&mut ws).await?;
    let (response, crypter, handshake_hash) =
        handshake::respond_qr(psk, peer_identity_x962, &initial).context("handshake failed")?;
    tunnel::write_binary(&mut ws, response).await?;

    let mut link = CableLink {
        ws,
        crypter,
        handshake_hash,
    };

    let mut post = Vec::new();
    cbor::map(&mut post, 1);
    cbor::uint(&mut post, 1);
    cbor::bytes(&mut post, getinfo_reply);
    // The post-handshake message is not typed; it is sent as a raw encrypted
    // frame whose plaintext is the CBOR map itself.
    send_raw(&mut link, &post).await?;

    loop {
        match link.recv_message().await {
            Ok((MSG_CTAP, payload)) => {
                tracing::info!(len = payload.len(), "phone received CTAP command");
                let reply = canned_ctap_reply(&payload);
                link.send_ctap(&reply).await?;
            }
            Ok((MSG_SHUTDOWN, _)) => {
                tracing::info!("phone received shutdown");
                return Ok(());
            }
            Ok((ty, payload)) => {
                tracing::info!(ty, len = payload.len(), "phone received other message");
            }
            Err(e) => {
                tracing::info!("phone connection ended: {e:#}");
                return Ok(());
            }
        }
    }
}

async fn send_raw(link: &mut CableLink, plaintext: &[u8]) -> Result<()> {
    let ct = link.crypter.encrypt(plaintext).context("encrypt failed")?;
    tunnel::write_binary(&mut link.ws, ct).await
}

fn canned_ctap_reply(command: &[u8]) -> Vec<u8> {
    if command.first() == Some(&ctap::CMD_GET_INFO) {
        ctap::mock_getinfo_response()
    } else {
        vec![ctap::CTAP1_ERR_INVALID_COMMAND]
    }
}
