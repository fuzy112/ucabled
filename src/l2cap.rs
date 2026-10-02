// SPDX-License-Identifier: GPL-3.0-or-later

//! LE L2CAP Connection-oriented Channel client for the CTAP 2.3 hybrid BLE
//! data channel.
//!
//! The channel is deliberately insecure (no pairing, no link encryption):
//! CTAP traffic has its own Noise layer on top (CTAP 2.3 §11.5.1.1.2). Each
//! caBLE message is carried as one L2CAP SDU, so the message boundaries the
//! spec's socket abstraction relies on are preserved. This uses the LE Credit
//! Based Flow Control mode (`SOCK_SEQPACKET`); if a phone turns out to use
//! Enhanced Credit Based Flow Control instead, this is where that changes.

use anyhow::{ensure, Context, Result};
use bluer::l2cap::{Security, SecurityLevel, SeqPacket, SocketAddr};
use bluer::{Address, AddressType};

/// Upper bound for a single caBLE message; matches the WebSocket frame limit.
const MAX_SDU: usize = crate::tunnel::MAX_FRAME_SIZE;

/// Connect an LE L2CAP CoC to `psm` on the peer described by the BLE advert.
pub async fn connect(address: Address, address_type: AddressType, psm: u16) -> Result<SeqPacket> {
    let stream = SeqPacket::connect(SocketAddr::new(address, address_type, psm))
        .await
        .context("L2CAP CoC connect failed")?;

    // Insecure by design; do not let the kernel force a security level the
    // authenticator's server socket does not offer.
    let _ = stream.as_ref().set_security(Security {
        level: SecurityLevel::Sdp,
        key_size: 0,
    });
    // Advertise the largest receive SDU we can so a whole CTAP message fits in
    // one caBLE message. Best effort: not every controller permits this.
    let _ = stream.as_ref().set_recv_mtu(u16::MAX);

    Ok(stream)
}

/// Send one caBLE message as a single L2CAP SDU.
pub async fn send(stream: &SeqPacket, data: &[u8]) -> Result<()> {
    let n = stream.send(data).await.context("L2CAP send failed")?;
    ensure!(n == data.len(), "short L2CAP send: {n}/{}", data.len());
    Ok(())
}

/// Receive one caBLE message (one L2CAP SDU).
pub async fn recv(stream: &SeqPacket) -> Result<Vec<u8>> {
    let mut buf = vec![0u8; MAX_SDU];
    let n = stream.recv(&mut buf).await.context("L2CAP recv failed")?;
    buf.truncate(n);
    Ok(buf)
}
