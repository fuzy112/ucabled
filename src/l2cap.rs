// SPDX-License-Identifier: GPL-3.0-or-later

//! LE L2CAP Connection-oriented Channel client for the hybrid BLE data
//! channel.
//!
//! The channel is deliberately insecure (no pairing, no link encryption):
//! CTAP traffic has its own Noise layer on top.  Every caBLE message is
//! framed with a 4-byte big-endian length prefix (PXP §6.1.2), so the socket
//! need not preserve message boundaries.  This uses the LE Credit Based Flow
//! Control mode (`SOCK_SEQPACKET`); if a phone turns out to use Enhanced
//! Credit Based Flow Control instead, this is where that changes.

use anyhow::{Context, Result, ensure};
use bluer::l2cap::{Security, SecurityLevel, SeqPacket, SocketAddr};
use bluer::{Address, AddressType};

/// Upper bound for a single caBLE message; matches the WebSocket frame limit.
const MAX_SDU: usize = crate::tunnel::MAX_FRAME_SIZE;

/// Length in bytes of the big-endian length prefix framing every caBLE
/// message (PXP §6.1.2).
pub(crate) const LENGTH_PREFIX: usize = 4;

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

/// Prefix `data` with its big-endian length (PXP §6.1.2).
fn frame(data: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        data.len() <= MAX_SDU,
        "caBLE message too large for L2CAP: {} > {MAX_SDU}",
        data.len()
    );
    let mut framed = Vec::with_capacity(LENGTH_PREFIX + data.len());
    framed.extend_from_slice(&(data.len() as u32).to_be_bytes());
    framed.extend_from_slice(data);
    Ok(framed)
}

/// Split one length-prefixed frame, validating the peer-controlled header.
fn unframe(buf: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        buf.len() >= LENGTH_PREFIX,
        "short L2CAP message: {} bytes",
        buf.len()
    );
    let len = u32::from_be_bytes(buf[..LENGTH_PREFIX].try_into().unwrap()) as usize;
    ensure!(
        len <= MAX_SDU,
        "L2CAP frame length {len} exceeds the {MAX_SDU}-byte limit"
    );
    ensure!(
        buf.len() == LENGTH_PREFIX + len,
        "L2CAP frame length mismatch: header says {len}, frame has {}",
        buf.len() - LENGTH_PREFIX
    );
    Ok(buf[LENGTH_PREFIX..].to_vec())
}

/// Send one caBLE message, prefixed with its big-endian length.
pub async fn send(stream: &SeqPacket, data: &[u8]) -> Result<()> {
    let framed = frame(data)?;
    let n = stream.send(&framed).await.context("L2CAP send failed")?;
    ensure!(n == framed.len(), "short L2CAP send: {n}/{}", framed.len());
    Ok(())
}

/// Receive one caBLE message: a length prefix followed by that many bytes.
pub async fn recv(stream: &SeqPacket) -> Result<Vec<u8>> {
    // One SDU holds a whole frame (LE Credit Based mode preserves the
    // boundary); the prefix is validated by `unframe`.
    let mut buf = vec![0u8; LENGTH_PREFIX + MAX_SDU];
    let n = stream.recv(&mut buf).await.context("L2CAP recv failed")?;
    buf.truncate(n);
    unframe(&buf)
}

/// The largest caBLE message this channel may send, from the peer's
/// negotiated receive MTU.  `None` when the kernel does not report one, in
/// which case the caller proceeds and lets the send fail if it must.
pub fn send_mtu(stream: &SeqPacket) -> Option<usize> {
    stream.send_mtu().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_roundtrips_and_carries_the_length() {
        for len in [0usize, 1, 4, 255, 256, MAX_SDU] {
            let data: Vec<u8> = (0..len).map(|i| (i * 31 + 7) as u8).collect();
            let framed = frame(&data).unwrap();
            assert_eq!(framed.len(), LENGTH_PREFIX + len);
            assert_eq!(&framed[..LENGTH_PREFIX], &(len as u32).to_be_bytes());
            assert_eq!(unframe(&framed).unwrap(), data);
        }
    }

    #[test]
    fn frame_rejects_an_oversized_message() {
        assert!(frame(&vec![0u8; MAX_SDU + 1]).is_err());
    }

    #[test]
    fn unframe_rejects_bad_framing() {
        // Shorter than the header.
        assert!(unframe(&[0, 0, 0]).is_err());
        // A declared length beyond the cap, even if fewer bytes follow.
        let mut huge = ((MAX_SDU + 1) as u32).to_be_bytes().to_vec();
        huge.push(0);
        assert!(unframe(&huge).is_err());
        // Header length disagrees with the bytes present (the peer controls
        // both, so neither may be trusted implicitly).
        assert!(unframe(&[0, 0, 0, 2, 0xaa]).is_err());
        assert!(unframe(&[0, 0, 0, 0, 0xaa]).is_err());
    }

    #[test]
    fn unframe_accepts_an_empty_message() {
        assert_eq!(unframe(&[0, 0, 0, 0]).unwrap(), Vec::<u8>::new());
    }
}
