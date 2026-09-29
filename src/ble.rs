use anyhow::{Context, Result};
use bluer::AdapterEvent;
use futures::StreamExt;
use std::time::Duration;

use crate::eid;
use crate::CABLE_BLE_UUID;

/// Scan BLE advertisements until one trial-decrypts with `eid_key`.
/// Returns the 16-byte plaintext EID.
pub async fn await_advert(
    eid_key: &[u8; eid::EID_KEY_SIZE],
    timeout: Duration,
) -> Result<[u8; eid::EID_PLAINTEXT_SIZE]> {
    let session = bluer::Session::new()
        .await
        .context("D-Bus session failed")?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    let uuid: bluer::Uuid = CABLE_BLE_UUID.parse()?;
    // Deliberately no UUID filter at the BlueZ level: we match on service
    // data ourselves, which is more robust while debugging.
    let filter = bluer::DiscoveryFilter {
        transport: bluer::DiscoveryTransport::Le,
        duplicate_data: true,
        ..Default::default()
    };
    adapter.set_discovery_filter(filter).await?;

    // `with_changes` re-emits DeviceAdded whenever a device's properties
    // change; service data typically arrives after the initial DeviceAdded.
    let mut events = adapter.discover_devices_with_changes().await?;
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        let event = match tokio::time::timeout_at(deadline, events.next()).await {
            Ok(Some(event)) => event,
            Ok(None) => anyhow::bail!("BLE discovery stream ended"),
            Err(_) => anyhow::bail!("timed out waiting for caBLE BLE advert"),
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
            if data_uuid != uuid {
                continue;
            }
            tracing::debug!(%address, len = payload.len(), "saw fff9 service data");
            if let Some(plaintext) = eid::decrypt(&payload, eid_key) {
                tracing::info!(%address, "caBLE advert trial decrypt succeeded");
                return Ok(plaintext);
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
