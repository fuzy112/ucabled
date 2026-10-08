// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal hand-written CBOR (RFC 8949) encoding and decoding used for CTAP2
//! responses and the caBLE QR/advert payloads. Only the subset the protocols
//! need; the wire format bytes are named once here instead of being scattered
//! as literals.
//!
//! Everything is encoded in canonical (minimal-length) form: strict parsers,
//! notably Chromium's, reject non-minimal integers.
//!
//! [`Decoder`] is the single decoding entry point. All three payloads it
//! serves (QR contents, advertisement suffix, CTAP requests) are
//! attacker-influenced, so it is hardened uniformly: length arithmetic is
//! overflow-checked and container nesting is depth-bounded.

/// CBOR major types (RFC 8949 §3.1).
pub const MAJOR_UINT: u8 = 0;
pub const MAJOR_NEGINT: u8 = 1;
pub const MAJOR_BYTES: u8 = 2;
pub const MAJOR_TEXT: u8 = 3;
pub const MAJOR_ARRAY: u8 = 4;
pub const MAJOR_MAP: u8 = 5;
pub const MAJOR_SIMPLE: u8 = 7;

/// Simple values (major type 7).
pub const FALSE: u8 = 0xf4;
pub const TRUE: u8 = 0xf5;

/// Additional-information values selecting the argument width. Values up to
/// [`ARG_INLINE_MAX`] carry the argument directly.
pub const ARG_INLINE_MAX: u8 = 23;
pub const ARG_U8: u8 = 24;
pub const ARG_U16: u8 = 25;
pub const ARG_U32: u8 = 26;
pub const ARG_U64: u8 = 27;

/// Mask extracting the additional information from a header byte.
pub const ARG_MASK: u8 = 0x1f;

/// Append a type head (major type + argument), minimally encoded.
pub fn head(out: &mut Vec<u8>, major: u8, arg: u64) {
    debug_assert!(major <= MAJOR_SIMPLE);
    let mb = major << 5;
    if arg <= ARG_INLINE_MAX as u64 {
        out.push(mb | arg as u8);
    } else if arg <= u8::MAX as u64 {
        out.push(mb | ARG_U8);
        out.push(arg as u8);
    } else if arg <= u16::MAX as u64 {
        out.push(mb | ARG_U16);
        out.extend_from_slice(&(arg as u16).to_be_bytes());
    } else if arg <= u32::MAX as u64 {
        out.push(mb | ARG_U32);
        out.extend_from_slice(&(arg as u32).to_be_bytes());
    } else {
        out.push(mb | ARG_U64);
        out.extend_from_slice(&arg.to_be_bytes());
    }
}

pub fn uint(out: &mut Vec<u8>, v: u64) {
    head(out, MAJOR_UINT, v);
}

pub fn bytes(out: &mut Vec<u8>, data: &[u8]) {
    head(out, MAJOR_BYTES, data.len() as u64);
    out.extend_from_slice(data);
}

pub fn text(out: &mut Vec<u8>, s: &str) {
    head(out, MAJOR_TEXT, s.len() as u64);
    out.extend_from_slice(s.as_bytes());
}

pub fn array(out: &mut Vec<u8>, len: u64) {
    head(out, MAJOR_ARRAY, len);
}

pub fn map(out: &mut Vec<u8>, len: u64) {
    head(out, MAJOR_MAP, len);
}

/// Maximum nesting depth accepted while decoding. The payloads are
/// attacker-influenced and skipping a value recurses per nested array/map,
/// so the recursion must be bounded.
pub const MAX_DEPTH: usize = 16;

/// Minimal bounded CBOR decoder: just enough to walk the maps and arrays the
/// caBLE and CTAP2 payloads contain and to skip unknown values, without
/// unbounded recursion or length-arithmetic overflow.
pub struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Offset of the next unread byte.
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// The whole input, for slicing out already-consumed spans.
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    pub fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    pub fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }

    /// Decode the argument selected by a header's additional-information bits.
    pub fn argument(&mut self, info: u8) -> Option<u64> {
        match info {
            0..=ARG_INLINE_MAX => Some(info as u64),
            ARG_U8 => Some(self.byte()? as u64),
            ARG_U16 => {
                let b = self.take(2)?;
                Some(u16::from_be_bytes([b[0], b[1]]) as u64)
            }
            ARG_U32 => {
                let b = self.take(4)?;
                Some(u32::from_be_bytes(b.try_into().ok()?) as u64)
            }
            ARG_U64 => {
                let b = self.take(8)?;
                Some(u64::from_be_bytes(b.try_into().ok()?))
            }
            _ => None,
        }
    }

    /// Read an unsigned integer, erroring on any other major type.
    pub fn uint(&mut self) -> Option<u64> {
        let h = self.byte()?;
        if h >> 5 != MAJOR_UINT {
            return None;
        }
        self.argument(h & ARG_MASK)
    }

    /// Read a map header, returning its entry count.
    pub fn map_header(&mut self) -> Option<u64> {
        let h = self.byte()?;
        if h >> 5 != MAJOR_MAP {
            return None;
        }
        self.argument(h & ARG_MASK)
    }

    /// Read a byte string, returning its contents.
    pub fn bytes(&mut self) -> Option<&'a [u8]> {
        let h = self.byte()?;
        if h >> 5 != MAJOR_BYTES {
            return None;
        }
        let len = self.argument(h & ARG_MASK)? as usize;
        self.take(len)
    }

    /// Read a text string.
    pub fn text(&mut self) -> Option<String> {
        let h = self.byte()?;
        if h >> 5 != MAJOR_TEXT {
            return None;
        }
        let len = self.argument(h & ARG_MASK)? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }

    /// Consume one value of any type.
    pub fn skip_value(&mut self) -> Option<()> {
        self.skip_value_at(0)
    }

    fn skip_value_at(&mut self, depth: usize) -> Option<()> {
        if depth > MAX_DEPTH {
            return None;
        }
        let h = self.byte()?;
        let major = h >> 5;
        let info = h & ARG_MASK;
        match major {
            MAJOR_UINT | MAJOR_NEGINT | MAJOR_SIMPLE => {
                self.argument(info)?;
            }
            MAJOR_BYTES | MAJOR_TEXT => {
                let len = self.argument(info)?;
                self.take(len as usize)?;
            }
            MAJOR_ARRAY => {
                let n = self.argument(info)?;
                for _ in 0..n {
                    self.skip_value_at(depth + 1)?;
                }
            }
            MAJOR_MAP => {
                let n = self.argument(info)?;
                for _ in 0..n {
                    self.skip_value_at(depth + 1)?;
                    self.skip_value_at(depth + 1)?;
                }
            }
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skip_value_rejects_deep_nesting() {
        let mut shallow = Decoder::new(&[0x81, 0x81, 0x81, 0x81, 0x00]);
        assert!(shallow.skip_value().is_some());

        let mut deep = vec![0x81u8; MAX_DEPTH + 4];
        deep.push(0x00);
        let mut d = Decoder::new(&deep);
        assert!(d.skip_value().is_none());
    }

    #[test]
    fn take_rejects_huge_length_without_overflow() {
        // bytes header with a u64 length that would wrap pos + n
        let data = [0x5b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let mut d = Decoder::new(&data);
        assert!(d.skip_value().is_none());
    }

    #[test]
    fn skip_value_covers_simple_widths() {
        // [false, half-float, float, double, uint 7]
        let data = [
            0x85, 0xf4, 0xf9, 0x00, 0x00, 0xfa, 0, 0, 0, 0, 0xfb, 0, 0, 0, 0, 0, 0, 0, 0, 0x07,
        ];
        let mut d = Decoder::new(&data);
        assert!(d.skip_value().is_some());
        assert_eq!(d.pos(), data.len());
        // break is not a well-formed standalone value
        assert!(Decoder::new(&[0xff]).skip_value().is_none());
    }
}
