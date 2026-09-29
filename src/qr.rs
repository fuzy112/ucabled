use std::time::{SystemTime, UNIX_EPOCH};

pub const QR_SECRET_SIZE: usize = 16;
pub const COMPRESSED_PUBLIC_KEY_SIZE: usize = 33;

#[derive(Clone, Copy)]
pub enum RequestType {
    MakeCredential,
    GetAssertion,
}

impl RequestType {
    fn as_str(self) -> &'static str {
        match self {
            RequestType::MakeCredential => "mc",
            RequestType::GetAssertion => "ga",
        }
    }
}

const WIDTHS: [usize; 8] = [0, 3, 5, 8, 10, 13, 15, 17];

pub fn bytes_to_digits(input: &[u8]) -> String {
    let mut ret = String::new();
    for chunk in input.chunks(7) {
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
    while rest.len() >= 17 {
        let v: u64 = rest[..17].parse().ok()?;
        if v >> 56 != 0 {
            return None;
        }
        ret.extend_from_slice(&v.to_le_bytes()[..7]);
        rest = &rest[17..];
    }
    if !rest.is_empty() {
        let n = match rest.len() {
            3 => 1,
            5 => 2,
            8 => 3,
            10 => 4,
            13 => 5,
            15 => 6,
            _ => return None,
        };
        let v: u64 = rest.parse().ok()?;
        if n < 8 && v >> (n * 8) != 0 {
            return None;
        }
        ret.extend_from_slice(&v.to_le_bytes()[..n]);
    }
    Some(ret)
}

fn cbor_uint(out: &mut Vec<u8>, v: u64) {
    if v < 24 {
        out.push(v as u8);
    } else if v <= 0xff {
        out.extend_from_slice(&[0x18, v as u8]);
    } else if v <= 0xffff {
        out.push(0x19);
        out.extend_from_slice(&(v as u16).to_be_bytes());
    } else if v <= 0xffff_ffff {
        out.push(0x1a);
        out.extend_from_slice(&(v as u32).to_be_bytes());
    } else {
        out.push(0x1b);
        out.extend_from_slice(&v.to_be_bytes());
    }
}

pub fn encode_qr_contents(
    compressed_public_key: &[u8; COMPRESSED_PUBLIC_KEY_SIZE],
    secret: &[u8; QR_SECRET_SIZE],
    num_known_domains: u8,
    supports_linking: bool,
    request_type: RequestType,
) -> Vec<u8> {
    let mut out = vec![0xa6];

    out.push(0x00);
    out.push(0x58);
    out.push(33);
    out.extend_from_slice(compressed_public_key);

    out.push(0x01);
    out.push(0x50);
    out.extend_from_slice(secret);

    out.push(0x02);
    cbor_uint(&mut out, num_known_domains as u64);

    out.push(0x03);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    cbor_uint(&mut out, now);

    out.push(0x04);
    out.push(if supports_linking { 0xf5 } else { 0xf4 });

    out.push(0x05);
    let rt = request_type.as_str();
    out.push(0x60 | rt.len() as u8);
    out.extend_from_slice(rt.as_bytes());

    out
}

pub fn encode_qr_url(
    compressed_public_key: &[u8; COMPRESSED_PUBLIC_KEY_SIZE],
    secret: &[u8; QR_SECRET_SIZE],
    num_known_domains: u8,
    supports_linking: bool,
    request_type: RequestType,
) -> String {
    let contents = encode_qr_contents(
        compressed_public_key,
        secret,
        num_known_domains,
        supports_linking,
        request_type,
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
            0..=23 => Some(info as usize),
            24 => Some(read_u8(pos)? as usize),
            25 => {
                let b = read_bytes(pos, 2)?;
                Some(u16::from_be_bytes([b[0], b[1]]) as usize)
            }
            26 => {
                let b = read_bytes(pos, 4)?;
                Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize)
            }
            _ => None,
        }
    };
    let read_uint = |pos: &mut usize, info: u8| -> Option<u64> {
        match info {
            0..=23 => Some(info as u64),
            24 => Some(read_u8(pos)? as u64),
            25 => {
                let b = read_bytes(pos, 2)?;
                Some(u16::from_be_bytes([b[0], b[1]]) as u64)
            }
            26 => {
                let b = read_bytes(pos, 4)?;
                Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
            }
            27 => {
                let b = read_bytes(pos, 8)?;
                Some(u64::from_be_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ]))
            }
            _ => None,
        }
    };

    let header = read_u8(&mut pos)?;
    if header & 0xe0 != 0xa0 {
        return None;
    }
    let pairs = (header & 0x1f) as usize;

    let mut public_key: Option<[u8; COMPRESSED_PUBLIC_KEY_SIZE]> = None;
    let mut secret: Option<[u8; QR_SECRET_SIZE]> = None;

    for _ in 0..pairs {
        let key_header = read_u8(&mut pos)?;
        let key = read_uint(&mut pos, key_header & 0x1f)?;
        let value_header = read_u8(&mut pos)?;
        let major = value_header >> 5;
        let info = value_header & 0x1f;
        match major {
            0 | 1 => {
                read_uint(&mut pos, info)?;
            }
            2 | 3 => {
                let len = read_len(&mut pos, info)?;
                let bytes = read_bytes(&mut pos, len)?;
                if major == 2 && key == 0 && len == COMPRESSED_PUBLIC_KEY_SIZE {
                    public_key = Some(bytes.try_into().ok()?);
                }
                if major == 2 && key == 1 && len == QR_SECRET_SIZE {
                    secret = Some(bytes.try_into().ok()?);
                }
            }
            7 => {
                if info >= 24 {
                    let skip = match info {
                        24 => 1,
                        25 => 2,
                        26 => 4,
                        27 => 8,
                        _ => return None,
                    };
                    read_bytes(&mut pos, skip)?;
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
}
