// SPDX-License-Identifier: GPL-3.0-or-later

//! Hardware-only checks for the CTAP 2.3 BLE data channel. These need a real
//! Bluetooth LE adapter, so they are `#[ignore]`d by default; run with
//! `cargo test --features l2cap --test l2cap -- --ignored`.
#![cfg(feature = "l2cap")]

use anyhow::Result;
use bluer::l2cap::{SeqPacketListener, SocketAddr};

/// Validate the LE L2CAP CoC client path (connect + one SDU round trip) against
/// a local listener on the same adapter.
#[tokio::test]
#[ignore = "requires a Bluetooth LE adapter"]
async fn l2cap_self_loopback() -> Result<()> {
    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;
    let address = adapter.address().await?;
    let address_type = adapter.address_type().await?;

    let listener = SeqPacketListener::bind(SocketAddr::any_le()).await?;
    let psm = listener.as_ref().local_addr()?.psm;

    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 64];
        let n = stream.recv(&mut buf).await.expect("server recv");
        stream.send(&buf[..n]).await.expect("server send");
    });

    let client = ucabled::l2cap::connect(address, address_type, psm).await?;
    ucabled::l2cap::send(&client, b"ping").await?;
    assert_eq!(ucabled::l2cap::recv(&client).await?, b"ping");

    server.await?;
    Ok(())
}
