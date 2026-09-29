use anyhow::Result;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::SecretKey;
use rand::rngs::OsRng;
use rand::RngCore;
use std::time::Duration;

use crate::ble::BleError;
use crate::error::TransactionError;
use crate::kdf::{derive, Purpose};
use crate::qr::{self, RequestType};
use crate::session::DesktopFlow;
use crate::{eid, NUM_ASSIGNED_TUNNEL_DOMAINS};

/// How long the QR code stays valid while waiting for the phone's BLE advert.
pub const BLE_ADVERT_TIMEOUT: Duration = Duration::from_secs(300);

/// Run one full caBLE QR transaction: show a QR code, wait for the phone's
/// BLE advert, do the tunnel handshake, forward the CTAP command and return
/// the phone's CTAP response payload (status byte + CBOR).
///
/// `on_qr` is invoked with the QR contents once; the caller decides how to
/// present it (terminal, egui window, ...). `on_advert` is invoked when the
/// phone's BLE advert has been received, so the UI can stop waiting.
pub async fn run_qr_transaction(
    ctap_command: &[u8],
    request_type: RequestType,
    on_qr: impl FnOnce(&str),
    on_advert: impl FnOnce(),
) -> Result<Vec<u8>, TransactionError> {
    let identity = SecretKey::random(&mut OsRng);
    let compressed = identity.public_key().to_encoded_point(true);
    let compressed: &[u8; 33] = compressed.as_bytes().try_into().unwrap();

    let mut qr_secret = [0u8; 16];
    OsRng.fill_bytes(&mut qr_secret);

    // Linking is shelved (iOS does not implement it, see docs/linking.md), so
    // do not advertise it and do not stay on the tunnel after the reply.
    let supports_linking = false;
    let qr_url = qr::encode_qr_url(
        compressed,
        &qr_secret,
        NUM_ASSIGNED_TUNNEL_DOMAINS,
        supports_linking,
        request_type,
    );
    on_qr(&qr_url);

    let mut eid_key = [0u8; eid::EID_KEY_SIZE];
    derive(&qr_secret, &[], Purpose::EidKey, &mut eid_key);

    let plaintext_eid = match crate::ble::await_advert(&eid_key, BLE_ADVERT_TIMEOUT).await {
        Ok(eid) => eid,
        Err(BleError::Timeout) => return Err(TransactionError::Timeout),
        Err(BleError::Backend(e)) => return Err(TransactionError::transport(e)),
    };
    on_advert();
    tracing::info!("received valid caBLE advert");

    let flow = DesktopFlow {
        tunnel_base: None,
        qr_secret,
        identity,
        plaintext_eid,
        supports_linking,
    };
    let result = flow.run(ctap_command).await?;
    Ok(result.ctap_reply)
}
