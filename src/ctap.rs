// SPDX-License-Identifier: GPL-3.0-or-later

use crate::cbor;

/// Stable AAGUID reported by getInfo. A fixed value keeps the authenticator
/// identity consistent across daemon restarts; the real attestation AAGUID
/// still comes from the phone.
pub const AAGUID: [u8; 16] = *b"ucabled-aaguid01";

// CTAP2 command bytes (CTAP 2.2 §6).
pub const CMD_MAKE_CREDENTIAL: u8 = 0x01;
pub const CMD_GET_ASSERTION: u8 = 0x02;
pub const CMD_GET_INFO: u8 = 0x04;

// CTAP status bytes (CTAP 2.2 §8.2) used on this relay.
pub const CTAP2_OK: u8 = 0x00;
pub const CTAP1_ERR_INVALID_COMMAND: u8 = 0x01;
pub const CTAP1_ERR_TIMEOUT: u8 = 0x05;
pub const CTAP2_ERR_CBOR_UNEXPECTED_TYPE: u8 = 0x11;
pub const CTAP2_ERR_INVALID_OPTION: u8 = 0x2c;
pub const CTAP2_ERR_KEEPALIVE_CANCEL: u8 = 0x2d;
pub const CTAP2_ERR_NO_CREDENTIALS: u8 = 0x2e;
pub const CTAP2_ERR_PIN_NOT_SET: u8 = 0x35;

// Map keys of the CTAP2 requests parsed locally (CTAP 2.2 §6.1/§6.2).
const MC_KEY_RP: u64 = 2;
const MC_KEY_USER: u64 = 3;
const GA_KEY_RPID: u64 = 1;
const GA_KEY_ALLOW_LIST: u64 = 3;
const GA_KEY_OPTIONS: u64 = 5;

// Map keys of the getInfo response built locally (CTAP 2.2 §6.4).
const GETINFO_KEY_VERSIONS: u64 = 1;
const GETINFO_KEY_AAGUID: u64 = 3;
const GETINFO_KEY_OPTIONS: u64 = 4;
const GETINFO_KEY_MAX_MSG_SIZE: u64 = 5;
const GETINFO_KEY_TRANSPORTS: u64 = 9;

// Map keys of the getAssertion response built locally (CTAP 2.2 §6.2).
const GA_RESP_KEY_CREDENTIAL: u64 = 1;
const GA_RESP_KEY_AUTH_DATA: u64 = 2;
const GA_RESP_KEY_SIGNATURE: u64 = 3;

// Synthetic assertion fields (see `fake_silent_assertion`).
/// authData flags byte: user presence and user verification unset.
const AUTH_DATA_FLAGS: u8 = 0x00;
/// Signature counter width in authData.
const AUTH_DATA_SIGN_COUNTER_LEN: usize = 4;
/// Placeholder (all-zero) signature length.
const DUMMY_SIGNATURE_LEN: usize = 8;

/// GetInfo advertises this maxMsgSize (CTAP 2.2 §6.4 default upper bound).
pub const MAX_MSG_SIZE: u64 = 7609;

/// authenticatorGetInfo response (status byte + canonical CBOR map).
///
/// Advertised per FR-5: FIDO_2_0 only (no U2F_V2), rk/up/uv, no clientPin,
/// maxMsgSize [`MAX_MSG_SIZE`], transports ["hybrid"].
pub fn getinfo_response(aaguid: &[u8; 16]) -> Vec<u8> {
    let mut out = vec![CTAP2_OK];

    cbor::map(&mut out, 5);

    cbor::uint(&mut out, GETINFO_KEY_VERSIONS);
    cbor::array(&mut out, 1);
    cbor::text(&mut out, "FIDO_2_0");

    cbor::uint(&mut out, GETINFO_KEY_AAGUID);
    cbor::bytes(&mut out, aaguid);

    cbor::uint(&mut out, GETINFO_KEY_OPTIONS);
    cbor::map(&mut out, 3);
    for key in ["rk", "up", "uv"] {
        cbor::text(&mut out, key);
        out.push(cbor::TRUE);
    }

    // Must be minimally encoded: Chromium's CBOR reader rejects non-minimal
    // integers and would drop the whole device on a bad getInfo.
    cbor::uint(&mut out, GETINFO_KEY_MAX_MSG_SIZE);
    cbor::uint(&mut out, MAX_MSG_SIZE);

    cbor::uint(&mut out, GETINFO_KEY_TRANSPORTS);
    cbor::array(&mut out, 1);
    cbor::text(&mut out, "hybrid");

    out
}

/// Minimal getInfo-shaped response ({versions, aaguid}) used only by the mock
/// phone and local relay tests, not by the real device.
pub fn mock_getinfo_response() -> Vec<u8> {
    let mut out = vec![CTAP2_OK];
    cbor::map(&mut out, 2);
    cbor::uint(&mut out, GETINFO_KEY_VERSIONS);
    cbor::array(&mut out, 1);
    cbor::text(&mut out, "FIDO_2_0");
    cbor::uint(&mut out, GETINFO_KEY_AAGUID);
    cbor::bytes(&mut out, &[0u8; 16]);
    out
}

/// Extract the RP ID from a makeCredential (key 2 -> rp map -> "id") or
/// getAssertion (key 1 -> string) request payload, for UI display only.
/// Returns None on any parse irregularity; callers must tolerate that.
pub fn extract_rp_id(command: &[u8]) -> Option<String> {
    let (&cmd, params) = command.split_first()?;
    let mut c = CborCursor {
        data: params,
        pos: 0,
    };
    let pairs = c.map_header()?;
    // makeCredential: key 2 = rp entity map; getAssertion: key 1 = rpId.
    let wanted = match cmd {
        CMD_MAKE_CREDENTIAL => MC_KEY_RP,
        CMD_GET_ASSERTION => GA_KEY_RPID,
        _ => return None,
    };
    let mut rp_id = None;
    for _ in 0..pairs {
        let key = c.uint()?;
        if key == wanted && cmd == CMD_MAKE_CREDENTIAL {
            let inner_pairs = c.map_header()?;
            for _ in 0..inner_pairs {
                let k = c.text()?;
                if k == "id" {
                    rp_id = Some(c.text()?);
                } else {
                    c.skip_value()?;
                }
            }
        } else if key == wanted && cmd == CMD_GET_ASSERTION {
            rp_id = Some(c.text()?);
        } else {
            c.skip_value()?;
        }
    }
    rp_id
}

/// Rebuild a CBOR map of text keys adding `add_key`/`add_value` if absent,
/// keeping canonical key order (length first, then lexicographic).
/// `data` must start exactly at the map header. Returns the rebuilt map and
/// the number of input bytes consumed.
fn patch_text_map(data: &[u8], add_key: &str, add_value: &str) -> Option<(Vec<u8>, usize)> {
    let mut c = CborCursor { data, pos: 0 };
    let pairs = c.map_header()?;
    let header_len = c.pos;

    let mut entries: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    let mut have_key = false;
    for _ in 0..pairs {
        let k = c.text()?;
        if k == add_key {
            have_key = true;
        }
        let start = c.pos;
        c.skip_value()?;
        entries.push((k, start..c.pos));
    }
    if have_key {
        return None;
    }

    entries.push((add_key.to_string(), 0..0)); // placeholder span
    entries.sort_by(|a, b| (a.0.len(), &a.0).cmp(&(b.0.len(), &b.0)));

    let mut out = Vec::new();
    cbor::map(&mut out, entries.len() as u64);
    for (k, span) in entries {
        cbor::text(&mut out, &k);
        if k == add_key {
            cbor::text(&mut out, add_value);
        } else {
            out.extend_from_slice(&data[span]);
        }
    }
    let _ = header_len;
    Some((out, c.pos))
}

/// Detect a silent getAssertion probe: options.up == false. Firefox sends
/// these (with an empty clientDataHash) to filter the allowList to
/// credentials present on this device. A phone cannot answer silently, so
/// the daemon answers locally with `fake_silent_assertion`.
pub fn is_silent_probe(command: &[u8]) -> bool {
    let Some((&CMD_GET_ASSERTION, params)) = command.split_first() else {
        return false;
    };
    let mut c = CborCursor {
        data: params,
        pos: 0,
    };
    let Some(pairs) = c.map_header() else {
        return false;
    };
    for _ in 0..pairs {
        let Some(key) = c.uint() else { return false };
        if key == GA_KEY_OPTIONS {
            // options map
            let Some(inner) = c.map_header() else {
                return false;
            };
            for _ in 0..inner {
                let Some(k) = c.text() else { return false };
                if k == "up" {
                    let Some(b) = c.byte() else { return false };
                    return b == cbor::FALSE;
                }
                if c.skip_value().is_none() {
                    return false;
                }
            }
            return false;
        }
        if c.skip_value().is_none() {
            return false;
        }
    }
    false
}

/// Build a structurally valid getAssertion "success" response that echoes the
/// first allowList credential, for answering Firefox's silent preflight probe
/// (up=false) without bothering the phone. Firefox only extracts the
/// credential descriptor from it; the real interactive request still goes to
/// the phone with the allowList intact. The fake signature never leaves the
/// browser's local filtering code.
///
/// Returns None (caller should fall back to relaying) if the request doesn't
/// parse or has no allowList.
pub fn fake_silent_assertion(command: &[u8]) -> Option<Vec<u8>> {
    use sha2::{Digest, Sha256};

    let Some((&CMD_GET_ASSERTION, params)) = command.split_first() else {
        return None;
    };
    let mut c = CborCursor {
        data: params,
        pos: 0,
    };
    let pairs = c.map_header()?;

    let mut rp_id: Option<String> = None;
    let mut first_cred: Option<Vec<u8>> = None;
    for _ in 0..pairs {
        let key = c.uint()?;
        match key {
            GA_KEY_RPID => rp_id = Some(c.text()?),
            GA_KEY_ALLOW_LIST => {
                // allowList: array of credential descriptor maps
                let b = c.byte()?;
                if b >> 5 != cbor::MAJOR_ARRAY {
                    return None;
                }
                let n = c.argument(b & cbor::ARG_MASK)?;
                if n == 0 {
                    return None;
                }
                // First entry: map with "id" bytestring.
                let entries = c.map_header()?;
                let mut cred_id = None;
                for _ in 0..entries {
                    let k = c.text()?;
                    if k == "id" {
                        let b = c.byte()?;
                        if b >> 5 != cbor::MAJOR_BYTES {
                            return None;
                        }
                        let len = c.argument(b & cbor::ARG_MASK)? as usize;
                        cred_id = Some(c.take(len)?.to_vec());
                    } else {
                        c.skip_value()?;
                    }
                }
                first_cred = cred_id;
                // Skip remaining allowList entries.
                for _ in 1..n {
                    c.skip_value()?;
                }
            }
            _ => c.skip_value()?,
        }
    }

    let rp_id = rp_id?;
    let cred_id = first_cred?;

    // authData = rpIdHash || flags(0) || counter(0)
    let mut auth_data = Sha256::digest(rp_id.as_bytes()).to_vec();
    auth_data.push(AUTH_DATA_FLAGS); // no UP/UV bits
    auth_data.extend_from_slice(&[0u8; AUTH_DATA_SIGN_COUNTER_LEN]);

    let mut out = vec![CTAP2_OK];
    cbor::map(&mut out, 3);
    // credential descriptor {"id": cred_id, "type": "public-key"}
    cbor::uint(&mut out, GA_RESP_KEY_CREDENTIAL);
    cbor::map(&mut out, 2);
    cbor::text(&mut out, "id");
    cbor::bytes(&mut out, &cred_id);
    cbor::text(&mut out, "type");
    cbor::text(&mut out, "public-key");
    cbor::uint(&mut out, GA_RESP_KEY_AUTH_DATA);
    cbor::bytes(&mut out, &auth_data);
    cbor::uint(&mut out, GA_RESP_KEY_SIGNATURE);
    cbor::bytes(&mut out, &[0u8; DUMMY_SIGNATURE_LEN]);
    Some(out)
}

/// Detect Firefox's "make me blink" dummy makeCredential, which Firefox sends
/// to physically identify the selected authenticator (a real key would flash).
/// There is nothing to blink on a phone; the daemon answers locally.
pub fn is_blink_probe(command: &[u8]) -> bool {
    extract_rp_id(command).as_deref() == Some("make.me.blink")
}

/// iOS requires `rp.name` and `user.displayName` in makeCredential requests;
/// Firefox omits them when the RP didn't provide them, and iOS then aborts
/// the hybrid session. Inject the missing fields (`user.displayName` falls
/// back to `user.name`, `rp.name` to ""). These are display-only fields not
/// covered by clientDataHash or attestation, so injecting them does not
/// affect cryptographic integrity. Returns the input unchanged if no
/// patching is needed or the payload doesn't parse.
pub fn patch_makecredential(command: &[u8]) -> Vec<u8> {
    let Some((&CMD_MAKE_CREDENTIAL, params)) = command.split_first() else {
        return command.to_vec();
    };

    let result = (|| -> Option<Vec<u8>> {
        let mut c = CborCursor {
            data: params,
            pos: 0,
        };
        let pairs = c.map_header()?;

        let mut out = vec![CMD_MAKE_CREDENTIAL];
        let mut entries: Vec<(u64, std::ops::Range<usize>)> = Vec::new();
        for _ in 0..pairs {
            let key = c.uint()?;
            let start = c.pos;
            c.skip_value()?;
            entries.push((key, start..c.pos));
        }

        let mut changed = false;
        cbor::map(&mut out, pairs);
        for (key, span) in entries {
            let value = &params[span.clone()];
            let patched = match key {
                MC_KEY_RP => {
                    // rp entity: add "name" if missing
                    patch_text_map(value, "name", "").map(|(v, _)| v)
                }
                MC_KEY_USER => {
                    // user entity: add "displayName", falling back to "name"
                    let display = {
                        let mut inner = CborCursor {
                            data: value,
                            pos: 0,
                        };
                        let mut found = None;
                        if let Some(p) = inner.map_header() {
                            for _ in 0..p {
                                let Some(k) = inner.text() else { break };
                                if k == "name" {
                                    found = inner.text();
                                } else {
                                    if inner.skip_value().is_none() {
                                        break;
                                    }
                                }
                            }
                        }
                        found.unwrap_or_default()
                    };
                    patch_text_map(value, "displayName", &display).map(|(v, _)| v)
                }
                _ => None,
            };
            cbor::uint(&mut out, key);
            match patched {
                Some(v) => {
                    changed = true;
                    out.extend_from_slice(&v);
                }
                None => out.extend_from_slice(value),
            }
        }
        if changed {
            Some(out)
        } else {
            None
        }
    })();

    result.unwrap_or_else(|| command.to_vec())
}

struct CborCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

/// Maximum nesting depth accepted while skipping CBOR values. The payload is
/// browser/page-influenced, so skipping it must not recurse unbounded.
const MAX_CBOR_DEPTH: usize = 16;

impl<'a> CborCursor<'a> {
    fn byte(&mut self) -> Option<u8> {
        let b = *self.data.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }

    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.data.get(self.pos..self.pos.checked_add(n)?)?;
        self.pos += n;
        Some(s)
    }

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
                Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
            }
            cbor::ARG_U64 => {
                let b = self.take(8)?;
                Some(u64::from_be_bytes([
                    b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
                ]))
            }
            _ => None,
        }
    }

    fn uint(&mut self) -> Option<u64> {
        let b = self.byte()?;
        if b >> 5 != cbor::MAJOR_UINT {
            return None;
        }
        self.argument(b & cbor::ARG_MASK)
    }

    fn map_header(&mut self) -> Option<u64> {
        let b = self.byte()?;
        if b >> 5 != cbor::MAJOR_MAP {
            return None;
        }
        self.argument(b & cbor::ARG_MASK)
    }

    fn text(&mut self) -> Option<String> {
        let b = self.byte()?;
        if b >> 5 != cbor::MAJOR_TEXT {
            return None;
        }
        let len = self.argument(b & cbor::ARG_MASK)? as usize;
        String::from_utf8(self.take(len)?.to_vec()).ok()
    }

    fn skip_value(&mut self) -> Option<()> {
        self.skip_value_at(0)
    }

    fn skip_value_at(&mut self, depth: usize) -> Option<()> {
        if depth > MAX_CBOR_DEPTH {
            return None;
        }
        let b = self.byte()?;
        let major = b >> 5;
        let info = b & cbor::ARG_MASK;
        match major {
            cbor::MAJOR_UINT | cbor::MAJOR_NEGINT | cbor::MAJOR_SIMPLE => {
                self.argument(info)?;
            }
            cbor::MAJOR_BYTES | cbor::MAJOR_TEXT => {
                let len = self.argument(info)?;
                self.take(len as usize)?;
            }
            cbor::MAJOR_ARRAY => {
                let n = self.argument(info)?;
                for _ in 0..n {
                    self.skip_value_at(depth + 1)?;
                }
            }
            cbor::MAJOR_MAP => {
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
    fn getinfo_shape() {
        let r = getinfo_response(&[0u8; 16]);
        assert_eq!(r[0], 0x00);
        // contains "FIDO_2_0" and "hybrid"
        let s = String::from_utf8_lossy(&r);
        assert!(s.contains("FIDO_2_0"));
        assert!(s.contains("hybrid"));
        // maxMsgSize must use the minimal two-byte encoding (canonical CBOR),
        // otherwise strict parsers (Chromium) reject the whole response.
        assert!(r.windows(3).any(|w| w == [0x19, 0x1d, 0xb9]));
        assert!(!r.windows(5).any(|w| w == [0x1a, 0x00, 0x00, 0x1d, 0xb9]));
    }

    #[test]
    fn extract_rp_id_getassertion() {
        // {1: "example.com", 2: h'aa..'}
        let mut cmd = vec![0x02, 0xa2, 0x01, 0x6b];
        cmd.extend_from_slice(b"example.com");
        cmd.extend_from_slice(&[0x02, 0x58, 0x20]);
        cmd.extend_from_slice(&[0xaa; 32]);
        assert_eq!(extract_rp_id(&cmd).as_deref(), Some("example.com"));
    }

    #[test]
    fn extract_rp_id_makecredential() {
        // {1: h'..32..', 2: {"id": "webauthn.io"}, 3: {...user...}}
        let mut cmd = vec![0x01, 0xa3, 0x01, 0x58, 0x20];
        cmd.extend_from_slice(&[0xbb; 32]); // clientDataHash
                                            // key 2: rp map {"id": "webauthn.io"}
        cmd.extend_from_slice(&[0x02, 0xa1, 0x62]);
        cmd.extend_from_slice(b"id");
        cmd.push(0x6b);
        cmd.extend_from_slice(b"webauthn.io");
        // key 3: user map {id: h'01', name: "u", displayName: "u"}
        cmd.extend_from_slice(&[0x03, 0xa3, 0x62]);
        cmd.extend_from_slice(b"id");
        cmd.extend_from_slice(&[0x41, 0x01]);
        cmd.push(0x64);
        cmd.extend_from_slice(b"name");
        cmd.push(0x61);
        cmd.extend_from_slice(b"u");
        cmd.push(0x6b);
        cmd.extend_from_slice(b"displayName");
        cmd.push(0x61);
        cmd.extend_from_slice(b"u");
        assert_eq!(extract_rp_id(&cmd).as_deref(), Some("webauthn.io"));
    }

    #[test]
    fn patch_real_firefox_payload() {
        let cmd = hex::decode("01a6015820cc1f4afda7a13ed02de520a7b2c8d5cce2b1e6d8dbe392f022a0cf88ee456fcb02a16269646b776562617574686e2e696f03a26269644d776562617574686e696f2d7172646e616d656271720483a263616c672764747970656a7075626c69632d6b6579a263616c672664747970656a7075626c69632d6b6579a263616c6739010064747970656a7075626c69632d6b657906a16b686d61632d736563726574f407a262726bf5627576f5").unwrap();
        let patched = patch_makecredential(&cmd);
        assert_ne!(patched, cmd);
        // rpId extraction still works on the patched request
        assert_eq!(extract_rp_id(&patched).as_deref(), Some("webauthn.io"));
        // injected fields are present
        let text = String::from_utf8_lossy(&patched);
        assert!(text.contains("displayName"));
        assert!(text.contains("name"));
        // user.displayName falls back to user.name ("qr")
        let key_pos = patched
            .windows(11)
            .position(|w| w == b"displayName")
            .unwrap();
        assert_eq!(patched[key_pos + 11], 0x62); // text(2)
        assert_eq!(&patched[key_pos + 12..key_pos + 14], b"qr");
    }

    #[test]
    fn patch_is_idempotent_and_untouched_when_complete() {
        let cmd = hex::decode("01a5015820cc1f4afda7a13ed02de520a7b2c8d5cce2b1e6d8dbe392f022a0cf88ee456fcb02a26269646b776562617574686e2e696f646e616d656b776562617574686e2e696f03a36269644d776562617574686e696f2d7172646e616d656271726b646973706c61794e616d656271720483a263616c672764747970656a7075626c69632d6b6579a263616c672664747970656a7075626c69632d6b6579a263616c6739010064747970656a7075626c69632d6b657907a162726bf5").unwrap();
        assert_eq!(patch_makecredential(&cmd), cmd);
    }

    #[test]
    fn blink_probe_detection() {
        // Captured from Firefox's dummy_make_credentials_cmd.
        let probe = hex::decode("01a6015820d0cee6fc7dbf599a919db8fb951311269f0eb781f7841c6cc0544ad9da34154b02a26269646d6d616b652e6d652e626c696e6b646e616d656003a36269644100646e616d656d6d616b652e6d652e626c696e6b6b646973706c61794e616d656d6d616b652e6d652e626c696e6b0481a263616c672664747970656a7075626c69632d6b657908400901").unwrap();
        assert!(is_blink_probe(&probe));
        assert!(!is_blink_probe(&[0x02, 0xa1, 0x01, 0x61, 0x61]));
    }

    #[test]
    fn silent_probe_detection() {
        // Real probe captured from Firefox: options {up: false}
        let probe = hex::decode("02a4016b776562617574686e2e696f025820e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8550381a26269645456ebb4bab2368cd5c71e173c5213c85203e2511964747970656a7075626c69632d6b657905a1627570f4").unwrap();
        assert!(is_silent_probe(&probe));
        // Real interactive request: options {uv: true, up: true}
        let interactive = hex::decode("02a4016b776562617574686e2e696f025820341a4e06e8dfd6cd4244803bd2079eb48ec2f1b41f832f2e18de51bd91248e8e0381a26269645456ebb4bab2368cd5c71e173c5213c85203e2511964747970656a7075626c69632d6b657905a2627576f5627570f5").unwrap();
        assert!(!is_silent_probe(&interactive));
        // No options at all
        let plain = hex::decode("02a2016b6578616d706c652e636f6d025820aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        assert!(!is_silent_probe(&plain));
    }

    #[test]
    fn fake_silent_assertion_echoes_credential() {
        let probe = hex::decode("02a4016b776562617574686e2e696f025820e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8550381a26269645456ebb4bab2368cd5c71e173c5213c85203e2511964747970656a7075626c69632d6b657905a1627570f4").unwrap();
        let resp = fake_silent_assertion(&probe).unwrap();
        assert_eq!(resp[0], 0x00);
        // echoes the credential id 56ebb4bab2368cd5c71e173c5213c85203e25119
        let cred = hex::decode("56ebb4bab2368cd5c71e173c5213c85203e25119").unwrap();
        assert!(resp.windows(cred.len()).any(|w| w == cred.as_slice()));
        assert!(resp.windows(10).any(|w| w == b"public-key"));
        // getAssertion without allowList cannot be faked
        let plain = hex::decode("02a2016b6578616d706c652e636f6d025820aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        assert!(fake_silent_assertion(&plain).is_none());
    }

    #[test]
    fn cbor_skip_rejects_deep_nesting() {
        let mut shallow = CborCursor {
            data: &[0x81, 0x81, 0x81, 0x81, 0x00],
            pos: 0,
        };
        assert!(shallow.skip_value().is_some());

        let mut deep = vec![0x81u8; MAX_CBOR_DEPTH + 4];
        deep.push(0x00);
        let mut c = CborCursor {
            data: &deep,
            pos: 0,
        };
        assert!(c.skip_value().is_none());
    }
}
