// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{anyhow, Context, Result};
use bluer::AdapterEvent;
use futures::StreamExt;
use std::time::Duration;

use crate::eid;
use crate::CABLE_BLE_UUID;

/// Failure while waiting for a caBLE BLE advert.
#[derive(Debug)]
pub enum BleError {
    /// No matching advert arrived within the timeout.
    Timeout,
    /// Bluetooth/BlueZ backend failure.
    Backend(anyhow::Error),
}

impl std::fmt::Display for BleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout => write!(f, "timed out waiting for caBLE BLE advert"),
            Self::Backend(e) => write!(f, "BLE backend error: {e:#}"),
        }
    }
}

impl std::error::Error for BleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Backend(e) => Some(e.as_ref()),
            Self::Timeout => None,
        }
    }
}

impl From<anyhow::Error> for BleError {
    fn from(e: anyhow::Error) -> Self {
        Self::Backend(e)
    }
}

impl From<bluer::Error> for BleError {
    fn from(e: bluer::Error) -> Self {
        Self::Backend(e.into())
    }
}

/// A matching caBLE advertisement: the decrypted EID plus what the CTAP 2.3
/// BLE data channel needs (the peer address and, if present, the L2CAP PSM).
pub struct AdvertObservation {
    pub plaintext_eid: [u8; eid::EID_PLAINTEXT_SIZE],
    pub address: bluer::Address,
    pub address_type: bluer::AddressType,
    /// Full service data as seen by BlueZ (20-byte EID plus any suffix); kept
    /// for diagnostics.
    pub service_data: Vec<u8>,
    /// L2CAP server PSM from the advertisement suffix, if the phone offered
    /// the BLE data channel.
    pub psm: Option<u16>,
}

/// Scan BLE advertisements until one trial-decrypts with `eid_key`.
/// Returns the 16-byte plaintext EID.
pub async fn await_advert(
    eid_key: &[u8; eid::EID_KEY_SIZE],
    timeout: Duration,
) -> Result<[u8; eid::EID_PLAINTEXT_SIZE], BleError> {
    Ok(await_advert_full(eid_key, timeout).await?.plaintext_eid)
}

/// Like [`await_advert`], but also reports the peer address and advertised PSM.
pub async fn await_advert_full(
    eid_key: &[u8; eid::EID_KEY_SIZE],
    timeout: Duration,
) -> Result<AdvertObservation, BleError> {
    let session = bluer::Session::new()
        .await
        .context("D-Bus session failed")?;
    let adapter = session.default_adapter().await?;

    // Do not switch the radio on behind the user's back; Chromium behaves the
    // same way. Without power there is nothing to scan, so fail clearly.
    if !adapter.is_powered().await? {
        return Err(BleError::Backend(anyhow!(
            "Bluetooth adapter is powered off; enable it to use a phone passkey"
        )));
    }

    let uuid: bluer::Uuid = CABLE_BLE_UUID
        .parse()
        .map_err(|e| BleError::Backend(anyhow!("invalid cable UUID: {e}")))?;

    let filter = bluer::DiscoveryFilter {
        transport: bluer::DiscoveryTransport::Le,
        duplicate_data: true,
        ..Default::default()
    };
    adapter.set_discovery_filter(filter).await?;

    // The discovery stream lives inside `scan`, so dropping it when scan
    // returns ends discovery before we hand control back.
    scan(&adapter, &uuid, eid_key, timeout).await
}

async fn scan(
    adapter: &bluer::Adapter,
    uuid: &bluer::Uuid,
    eid_key: &[u8; eid::EID_KEY_SIZE],
    timeout: Duration,
) -> Result<AdvertObservation, BleError> {
    // `with_changes` re-emits DeviceAdded whenever a device's properties
    // change; service data typically arrives after the initial DeviceAdded.
    //
    // No service_uuids filter at the BlueZ level: the caBLE advert carries
    // its UUID in service *data* (AD type 0x16), which such a filter would not
    // match, so we trial-decrypt the service data ourselves instead.
    let mut events = adapter.discover_devices_with_changes().await?;
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let event = match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(event)) => event,
            Ok(None) => return Err(BleError::Backend(anyhow!("BLE discovery stream ended"))),
            Err(_) => return Err(BleError::Timeout),
        };
        let address = match event {
            AdapterEvent::DeviceAdded(address) => address,
            _ => continue,
        };
        let device = match adapter.device(address) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let service_data = match device.service_data().await {
            Ok(Some(sd)) => sd,
            _ => continue,
        };
        for (data_uuid, payload) in service_data {
            if data_uuid != *uuid || payload.len() < eid::ADVERT_SIZE {
                continue;
            }
            tracing::debug!(%address, len = payload.len(), "saw fff9 service data");
            // Only the first 20 bytes are the encrypted EID; any remainder is
            // the CTAP 2.3 advertisement suffix (e.g. the L2CAP PSM).
            if let Some(plaintext) = eid::decrypt(&payload[..eid::ADVERT_SIZE], eid_key) {
                let psm = crate::advert::parse_psm(&payload[eid::ADVERT_SIZE..]);
                let address_type = device
                    .address_type()
                    .await
                    .unwrap_or(bluer::AddressType::LePublic);
                tracing::info!(%address, psm, len = payload.len(), "caBLE advert trial decrypt succeeded");
                return Ok(AdvertObservation {
                    plaintext_eid: plaintext,
                    address,
                    address_type,
                    service_data: payload,
                    psm,
                });
            }
            tracing::debug!(%address, "trial decrypt failed (advert for another QR)");
        }
    }
}

/// Advertise a caBLE EID (mock phone role).
pub async fn advertise(advert: &[u8; eid::ADVERT_SIZE]) -> Result<bluer::adv::AdvertisementHandle> {
    let session = bluer::Session::new()
        .await
        .context("D-Bus session failed")?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    let uuid: bluer::Uuid = CABLE_BLE_UUID.parse()?;
    let adv = bluer::adv::Advertisement {
        service_uuids: [uuid].into_iter().collect(),
        service_data: [(uuid, advert.to_vec())].into_iter().collect(),
        discoverable: Some(true),
        ..Default::default()
    };
    let handle = adapter.advertise(adv).await?;
    Ok(handle)
}
