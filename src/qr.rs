// SPDX-License-Identifier: GPL-3.0-or-later

use std::time::{SystemTime, UNIX_EPOCH};

use crate::cbor;

pub const QR_SECRET_SIZE: usize = 16;
pub const COMPRESSED_PUBLIC_KEY_SIZE: usize = 33;

/// QR CBOR map keys (CTAP 2.2 §11.5.1).
const QR_KEY_PUBLIC_KEY: u8 = 0;
const QR_KEY_SECRET: u8 = 1;
const QR_KEY_NUM_KNOWN_DOMAINS: u8 = 2;
const QR_KEY_EPOCH_SECONDS: u8 = 3;
const QR_KEY_SUPPORTS_LINKING: u8 = 4;
const QR_KEY_REQUEST_TYPE: u8 = 5;
/// CTAP 2.3 §11.5.1.1: list of data transfer channels the client supports.
const QR_KEY_TRANSPORTS: u8 = 6;

/// Data transfer channel identifiers for QR `Key 6`.
pub const TRANSPORT_WEBSOCKET: u64 = 0;
pub const TRANSPORT_BLE: u64 = 1;

/// Seven-byte chunks are encoded as 17-digit decimal numbers.
const DIGIT_CHUNK_BYTES: usize = 7;
const DIGIT_CHUNK_DIGITS: usize = 17;
/// Bits in a chunk hold over from the previous chunk (all but the last byte).
const DIGIT_CHUNK_OVERFLOW_BITS: u32 = 56;

#[derive(Clone, Copy, Debug)]
pub enum RequestType {
    MakeCredential,
    GetAssertion,
}

impl RequestType {
    /// The wire name used inside the QR contents (CTAP 2.2 §11.5.1, key 5)
    /// and handed to helpers so they can tell the two commands apart.
    pub fn as_str(self) -> &'static str {
        match self {
            RequestType::MakeCredential => "mc",
            RequestType::GetAssertion => "ga",
        }
    }
}

const WIDTHS: [usize; 8] = [0, 3, 5, 8, 10, 13, 15, 17];

pub fn bytes_to_digits(input: &[u8]) -> String {
    let mut ret = String::new();
    for chunk in input.chunks(DIGIT_CHUNK_BYTES) {
        let mut buf = [0u8; 8];
        buf[..chunk.len()].copy_from_slice(chunk);
        let v = u64::from_le_bytes(buf);
        let width = WIDTHS[chunk.len()];
        ret.push_str(&format!("{:0width$}", v, width = width));
    }
    ret
}

pub fn digits_to_bytes(digits: &str) -> Option<Vec<u8>> {
    if !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let mut ret = Vec::new();
    let mut rest = digits;
    while rest.len() >= DIGIT_CHUNK_DIGITS {
        let v: u64 = rest[..DIGIT_CHUNK_DIGITS].parse().ok()?;
        if v >> DIGIT_CHUNK_OVERFLOW_BITS != 0 {
            return None;
        }
        ret.extend_from_slice(&v.to_le_bytes()[..DIGIT_CHUNK_BYTES]);
        rest = &rest[DIGIT_CHUNK_DIGITS..];
    }
    if !rest.is_empty() {
        let n = (1..DIGIT_CHUNK_BYTES).find(|&n| WIDTHS[n] == rest.len())?;
        let v: u64 = rest.parse().ok()?;
        if v >> (n * 8) != 0 {
            return None;
        }
        ret.extend_from_slice(&v.to_le_bytes()[..n]);
    }
    Some(ret)
}

pub fn encode_qr_contents(
    compressed_public_key: &[u8; COMPRESSED_PUBLIC_KEY_SIZE],
    secret: &[u8; QR_SECRET_SIZE],
    num_known_domains: u8,
    supports_linking: bool,
    request_type: RequestType,
    channels: &[u64],
) -> Vec<u8> {
    let mut out = Vec::new();
    cbor::map(&mut out, 7);

    cbor::uint(&mut out, QR_KEY_PUBLIC_KEY as u64);
    cbor::bytes(&mut out, compressed_public_key);

    cbor::uint(&mut out, QR_KEY_SECRET as u64);
    cbor::bytes(&mut out, secret);

    cbor::uint(&mut out, QR_KEY_NUM_KNOWN_DOMAINS as u64);
    cbor::uint(&mut out, num_known_domains as u64);

    cbor::uint(&mut out, QR_KEY_EPOCH_SECONDS as u64);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    cbor::uint(&mut out, now);

    cbor::uint(&mut out, QR_KEY_SUPPORTS_LINKING as u64);
    out.push(if supports_linking {
        cbor::TRUE
    } else {
        cbor::FALSE
    });

    cbor::uint(&mut out, QR_KEY_REQUEST_TYPE as u64);
    cbor::text(&mut out, request_type.as_str());

    cbor::uint(&mut out, QR_KEY_TRANSPORTS as u64);
    cbor::array(&mut out, channels.len() as u64);
    for &channel in channels {
        cbor::uint(&mut out, channel);
    }

    out
}

pub fn encode_qr_url(
    compressed_public_key: &[u8; COMPRESSED_PUBLIC_KEY_SIZE],
    secret: &[u8; QR_SECRET_SIZE],
    num_known_domains: u8,
    supports_linking: bool,
    request_type: RequestType,
    channels: &[u64],
) -> String {
    let contents = encode_qr_contents(
        compressed_public_key,
        secret,
        num_known_domains,
        supports_linking,
        request_type,
        channels,
    );
    format!("FIDO:/{}", bytes_to_digits(&contents))
}

pub struct ParsedQr {
    pub compressed_public_key: [u8; COMPRESSED_PUBLIC_KEY_SIZE],
    pub secret: [u8; QR_SECRET_SIZE],
}

/// Parse a "FIDO:/..." QR URL. Only extracts keys 0 and 1, which are all the
/// phone side needs; unknown keys are ignored as the spec requires.
pub fn parse_qr_url(url: &str) -> Option<ParsedQr> {
    let digits = url.get(6..)?;
    if !url[..6].eq_ignore_ascii_case("FIDO:/") {
        return None;
    }
    let cbor = digits_to_bytes(digits)?;
    parse_qr_cbor(&cbor)
}

fn parse_qr_cbor(data: &[u8]) -> Option<ParsedQr> {
    let mut pos = 0usize;
    let read_u8 = |pos: &mut usize| -> Option<u8> {
        let b = *data.get(*pos)?;
        *pos += 1;
        Some(b)
    };
    let read_bytes = |pos: &mut usize, n: usize| -> Option<&[u8]> {
        let s = data.get(*pos..*pos + n)?;
        *pos += n;
        Some(s)
    };
    let read_len = |pos: &mut usize, info: u8| -> Option<usize> {
        match info {
            0..=cbor::ARG_INLINE_MAX => Some(info as usize),
            cbor::ARG_U8 => Some(read_u8(pos)? as usize),
            cbor::ARG_U16 => {
                let b = read_bytes(pos, 2)?;
                Some(u16::from_be_bytes([b[0], b[1]]) as usize)
            }
            cbor::ARG_U32 => {
                let b = read_bytes(pos, 4)?;
                Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
            }
            _ => None,
        }
    };
    let read_uint = |pos: &mut usize, info: u8| -> Option<u64> {
        match info {
            0..=cbor::ARG_INLINE_MAX => Some(info as u64),
            cbor::ARG_U8 => Some(read_u8(pos)? as u64),
            cbor::ARG_U16 => {
                let b = read_bytes(pos, 2)?;
                Some(u16::from_be_bytes([b[0], b[1]]) as u64)
            }
            cbor::ARG_U32 => {
                let b = read_bytes(pos, 4)?;
                Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
            }
            cbor::ARG_U64 => {
                let b = read_bytes(pos, 8)?;
                Some(u64::from_be_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ]))
            }
            _ => None,
        }
    };

    let header = read_u8(&mut pos)?;
    if header >> 5 != cbor::MAJOR_MAP {
        return None;
    }
    let pairs = (header & cbor::ARG_MASK) as usize;

    let mut public_key: Option<[u8; COMPRESSED_PUBLIC_KEY_SIZE]> = None;
    let mut secret: Option<[u8; QR_SECRET_SIZE]> = None;

    for _ in 0..pairs {
        let key_header = read_u8(&mut pos)?;
        let key = read_uint(&mut pos, key_header & cbor::ARG_MASK)?;
        let value_header = read_u8(&mut pos)?;
        let major = value_header >> 5;
        let info = value_header & cbor::ARG_MASK;
        match major {
            cbor::MAJOR_UINT | cbor::MAJOR_NEGINT => {
                read_uint(&mut pos, info)?;
            }
            cbor::MAJOR_BYTES | cbor::MAJOR_TEXT => {
                let len = read_len(&mut pos, info)?;
                let bytes = read_bytes(&mut pos, len)?;
                if major == cbor::MAJOR_BYTES
                    && key == QR_KEY_PUBLIC_KEY as u64
                    && len == COMPRESSED_PUBLIC_KEY_SIZE
                {
                    public_key = Some(bytes.try_into().ok()?);
                }
                if major == cbor::MAJOR_BYTES
                    && key == QR_KEY_SECRET as u64
                    && len == QR_SECRET_SIZE
                {
                    secret = Some(bytes.try_into().ok()?);
                }
            }
            cbor::MAJOR_SIMPLE => {
                if (cbor::ARG_U8..=cbor::ARG_U64).contains(&info) {
                    let skip = 1usize << (info - cbor::ARG_U8);
                    read_bytes(&mut pos, skip)?;
                }
            }
            cbor::MAJOR_ARRAY => {
                // QR key 6 is a list of transport-channel integers, but skip
                // any array element generically so unknown keys cannot break
                // parsing of keys 0 and 1.
                let len = read_len(&mut pos, info)?;
                for _ in 0..len {
                    let h = read_u8(&mut pos)?;
                    let emajor = h >> 5;
                    let einfo = h & cbor::ARG_MASK;
                    match emajor {
                        cbor::MAJOR_UINT | cbor::MAJOR_NEGINT => {
                            read_uint(&mut pos, einfo)?;
                        }
                        cbor::MAJOR_BYTES | cbor::MAJOR_TEXT => {
                            let n = read_len(&mut pos, einfo)?;
                            read_bytes(&mut pos, n)?;
                        }
                        cbor::MAJOR_SIMPLE => {
                            if (cbor::ARG_U8..=cbor::ARG_U64).contains(&einfo) {
                                read_bytes(&mut pos, 1usize << (einfo - cbor::ARG_U8))?;
                            }
                        }
                        _ => return None,
                    }
                }
            }
            _ => return None,
        }
    }

    Some(ParsedQr {
        compressed_public_key: public_key?,
        secret: secret?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digits_roundtrip() {
        for len in [1usize, 2, 3, 5, 6, 7, 8, 13, 14, 49, 50, 60] {
            let data: Vec<u8> = (0..len).map(|i| (i * 37 + 11) as u8).collect();
            let digits = bytes_to_digits(&data);
            let back = digits_to_bytes(&digits).unwrap();
            assert_eq!(data, back, "len {len}");
        }
    }

    #[test]
    fn digits_leading_zero_chunk() {
        let data = vec![0u8; 7];
        let digits = bytes_to_digits(&data);
        assert_eq!(digits, "00000000000000000");
        assert_eq!(digits_to_bytes(&digits).unwrap(), data);
    }

    #[test]
    fn digits_reject_malformed_input() {
        assert!(digits_to_bytes("abc").is_none());
        assert!(digits_to_bytes("12a45").is_none());
        // Invalid tail length (1 digit is not a legal 1..6 byte chunk).
        assert!(digits_to_bytes("1").is_none());
        // Too large for the chunk's byte width.
        assert!(digits_to_bytes("18446744073709551616").is_none());
    }

    #[test]
    fn digits_empty_is_empty() {
        assert_eq!(digits_to_bytes("").unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn parse_ignores_transport_channels() {
        let public_key = [0x11u8; COMPRESSED_PUBLIC_KEY_SIZE];
        let secret = [0x22u8; QR_SECRET_SIZE];
        let url = encode_qr_url(
            &public_key,
            &secret,
            2,
            false,
            RequestType::GetAssertion,
            &[TRANSPORT_WEBSOCKET, TRANSPORT_BLE],
        );
        let parsed = parse_qr_url(&url).unwrap();
        assert_eq!(parsed.compressed_public_key, public_key);
        assert_eq!(parsed.secret, secret);
    }
}
