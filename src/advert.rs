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
    let mut d = Decoder {
        data: suffix,
        pos: 0,
    };
    let entries = d.map_len()?;
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

/// Minimal CBOR decoder: just enough to walk the suffix map and skip unknown
/// values, including nested arrays and maps.
struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos + n)?;
        self.pos += n;
        Some(s)
    }

    /// Decode the argument selected by a header's additional-information bits.
    fn argument(&mut self, info: u8) -> Option<u64> {
        match info {
            0..=cbor::ARG_INLINE_MAX => Some(info as u64),
            cbor::ARG_U8 => Some(self.byte()? as u64),
            cbor::ARG_U16 => {
                let b = self.take(2)?;
                Some(u16::from_be_bytes([b[0], b[1]]) as u64)
            }
            cbor::ARG_U32 => {
                let b = self.take(4)?;
                Some(u32::from_be_bytes(b.try_into().ok()?) as u64)
            }
            cbor::ARG_U64 => {
                let b = self.take(8)?;
                Some(u64::from_be_bytes(b.try_into().ok()?))
            }
            _ => None,
        }
    }

    /// Read a map header, returning its entry count.
    fn map_len(&mut self) -> Option<usize> {
        let h = self.byte()?;
        if h >> 5 != cbor::MAJOR_MAP {
            return None;
        }
        Some(self.argument(h & cbor::ARG_MASK)? as usize)
    }

    /// Read an unsigned integer, erroring on any other major type.
    fn uint(&mut self) -> Option<u64> {
        let h = self.byte()?;
        if h >> 5 != cbor::MAJOR_UINT {
            return None;
        }
        self.argument(h & cbor::ARG_MASK)
    }

    /// Consume one value of any type.
    fn skip_value(&mut self) -> Option<()> {
        let h = self.byte()?;
        let major = h >> 5;
        let info = h & cbor::ARG_MASK;
        match major {
            cbor::MAJOR_UINT | cbor::MAJOR_NEGINT => {
                self.argument(info)?;
                Some(())
            }
            cbor::MAJOR_BYTES | cbor::MAJOR_TEXT => {
                let n = self.argument(info)? as usize;
                self.take(n)?;
                Some(())
            }
            cbor::MAJOR_ARRAY => {
                for _ in 0..self.argument(info)? {
                    self.skip_value()?;
                }
                Some(())
            }
            cbor::MAJOR_MAP => {
                for _ in 0..self.argument(info)?.checked_mul(2)? {
                    self.skip_value()?;
                }
                Some(())
            }
            cbor::MAJOR_SIMPLE => match info {
                0..=23 => Some(()),
                24 => self.byte().map(|_| ()),
                25 => self.take(2).map(|_| ()),
                26 => self.take(4).map(|_| ()),
                27 => self.take(8).map(|_| ()),
                _ => None,
            },
            _ => None,
        }
    }
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
}
