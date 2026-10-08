// SPDX-License-Identifier: GPL-3.0-or-later

//! CTAP 2.3 hybrid advertisement suffix.
//!
//! When the client offers the BLE data channel (QR `Key 6`), the authenticator
//! may append a CBOR map after the 20-byte encrypted EID in the caBLE service
//! data. For the BLE channel the map is `{1: <server PSM>}` (CTAP 2.3
//! §11.5.1.1.2); the PSM tells the client which L2CAP CoC to connect to.

use crate::cbor;

/// Transport-channel identifier for Bluetooth Low Energy in the suffix map.
pub const TRANSPORT_CHANNEL_BLE: u64 = 1;

/// Extract the L2CAP server PSM from an advertisement suffix, if present.
///
/// Unknown channels and malformed suffixes yield `None`; the suffix is
/// attacker-controllable and only ever consulted after the EID trial-decrypt
/// has authenticated the advert.
pub fn parse_psm(suffix: &[u8]) -> Option<u16> {
    let mut d = cbor::Decoder::new(suffix);
    let entries = d.map_header()?;
    let mut psm = None;
    for _ in 0..entries {
        let key = d.uint()?;
        if key == TRANSPORT_CHANNEL_BLE {
            psm = u16::try_from(d.uint()?).ok();
        } else {
            d.skip_value()?;
        }
    }
    psm
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_psm() {
        // {1: 0x1234}
        assert_eq!(parse_psm(&[0xa1, 0x01, 0x19, 0x12, 0x34]), Some(0x1234));
        // {1: 0x80} (u8-width value)
        assert_eq!(parse_psm(&[0xa1, 0x01, 0x18, 0x80]), Some(0x80));
    }

    #[test]
    fn skips_unknown_entries() {
        // {2: 5, 1: 0x80}
        assert_eq!(parse_psm(&[0xa2, 0x02, 0x05, 0x01, 0x18, 0x80]), Some(0x80));
        // {1: 0x80, 2: [1, true]}
        assert_eq!(
            parse_psm(&[0xa2, 0x01, 0x18, 0x80, 0x02, 0x82, 0x01, 0xf5]),
            Some(0x80)
        );
    }

    #[test]
    fn rejects_missing_or_malformed() {
        assert_eq!(parse_psm(&[]), None);
        assert_eq!(parse_psm(&[0xa0]), None); // empty map
        assert_eq!(parse_psm(&[0x01]), None); // not a map
        assert_eq!(parse_psm(&[0xa1, 0x02, 0x05]), None); // no BLE entry
        assert_eq!(parse_psm(&[0xa1, 0x01, 0x1a, 0x00, 0x01, 0x00, 0x00]), None);
        // PSM wider than u16
    }

    #[test]
    fn rejects_deeply_nested_unknown_values() {
        // {2: [[[[...]]]]} — the shared decoder bounds nesting depth.
        let mut suffix = vec![0xa1, 0x02];
        suffix.extend_from_slice(&[0x81u8; cbor::MAX_DEPTH + 4]);
        suffix.push(0x00);
        assert_eq!(parse_psm(&suffix), None);
    }
}
