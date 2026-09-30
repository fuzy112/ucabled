// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal hand-written CBOR (RFC 8949) encoding used for CTAP2 responses and
//! the caBLE QR/advert payloads. Only the subset the protocols need; the wire
//! format bytes are named once here instead of being scattered as literals.
//!
//! Everything is encoded in canonical (minimal-length) form: strict parsers,
//! notably Chromium's, reject non-minimal integers.

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
