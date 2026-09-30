// SPDX-License-Identifier: GPL-3.0-or-later

pub mod agent;
pub mod cbor;
pub mod crypter;
pub mod ctap;
pub mod ctaphid;
pub mod eid;
pub mod error;
pub mod handshake;
pub mod kdf;
pub mod noise;
pub mod phone;
pub mod qr;
#[cfg(feature = "ble")]
pub mod relay;
pub mod session;
pub mod tunnel;
pub mod uhid_dev;
pub mod ui;

#[cfg(feature = "ble")]
pub mod ble;

pub const NUM_ASSIGNED_TUNNEL_DOMAINS: u8 = tunnel::ASSIGNED_TUNNEL_DOMAINS.len() as u8;
pub const WS_SUBPROTOCOL: &str = "fido.cable";
pub const CABLE_BLE_UUID: &str = "0000fff9-0000-1000-8000-00805f9b34fb";

/// Best-effort wipe of secret bytes that the optimizer must not elide.
pub fn secure_erase(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: `b` is a valid, exclusively borrowed byte.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}
