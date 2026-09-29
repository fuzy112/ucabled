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

pub const TUNNEL_SERVER_DOMAIN: &str = "cable.ua5v.com";
pub const NUM_ASSIGNED_TUNNEL_DOMAINS: u8 = 2;
pub const WS_SUBPROTOCOL: &str = "fido.cable";
pub const CABLE_BLE_UUID: &str = "0000fff9-0000-1000-8000-00805f9b34fb";
