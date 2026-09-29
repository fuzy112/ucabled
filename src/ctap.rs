/// Stable AAGUID reported by getInfo. A fixed value keeps the authenticator
/// identity consistent across daemon restarts; the real attestation AAGUID
/// still comes from the phone.
pub const AAGUID: [u8; 16] = *b"ucabled-aaguid01";

/// authenticatorGetInfo response (status byte + canonical CBOR map).
///
/// Advertised per FR-5: FIDO_2_0 only (no U2F_V2), rk/up/uv, no clientPin,
/// maxMsgSize 7609, transports ["hybrid"].
pub fn getinfo_response(aaguid: &[u8; 16]) -> Vec<u8> {
    let mut out = vec![0x00]; // CTAP2 success status

    out.push(0xa5); // map(5)

    // 1: versions
    out.push(0x01);
    out.push(0x81); // array(1)
    out.push(0x68);
    out.extend_from_slice(b"FIDO_2_0");

    // 3: aaguid
    out.push(0x03);
    out.push(0x50);
    out.extend_from_slice(aaguid);

    // 4: options {rk, up, uv}
    out.push(0x04);
    out.push(0xa3);
    for key in [b"rk", b"up", b"uv"] {
        out.push(0x62);
        out.extend_from_slice(key);
        out.push(0xf5);
    }

    // 5: maxMsgSize = 7609
    out.push(0x05);
    out.push(0x1a);
    out.extend_from_slice(&7609u32.to_be_bytes());

    // 9: transports
    out.push(0x09);
    out.push(0x81);
    out.push(0x66);
    out.extend_from_slice(b"hybrid");

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
        0x01 => 2,
        0x02 => 1,
        _ => return None,
    };
    let mut rp_id = None;
    for _ in 0..pairs {
        let key = c.uint()?;
        if key == wanted && cmd == 0x01 {
            let inner_pairs = c.map_header()?;
            for _ in 0..inner_pairs {
                let k = c.text()?;
                if k == "id" {
                    rp_id = Some(c.text()?);
                } else {
                    c.skip_value()?;
                }
            }
        } else if key == wanted && cmd == 0x02 {
            rp_id = Some(c.text()?);
        } else {
            c.skip_value()?;
        }
    }
    rp_id
}

fn encode_text(out: &mut Vec<u8>, s: &str) {
    let len = s.len();
    if len < 24 {
        out.push(0x60 | len as u8);
    } else if len <= 0xff {
        out.extend_from_slice(&[0x78, len as u8]);
    } else {
        out.push(0x79);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    }
    out.extend_from_slice(s.as_bytes());
}

fn encode_map_header(out: &mut Vec<u8>, n: u64) {
    if n < 24 {
        out.push(0xa0 | n as u8);
    } else {
        out.push(0xb8);
        out.push(n as u8);
    }
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
    encode_map_header(&mut out, entries.len() as u64);
    for (k, span) in entries {
        encode_text(&mut out, &k);
        if k == add_key {
            encode_text(&mut out, add_value);
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
    let Some((&0x02, params)) = command.split_first() else {
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
        if key == 5 {
            // options map
            let Some(inner) = c.map_header() else {
                return false;
            };
            for _ in 0..inner {
                let Some(k) = c.text() else { return false };
                if k == "up" {
                    let Some(b) = c.byte() else { return false };
                    return b == 0xf4; // false
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

    let Some((&0x02, params)) = command.split_first() else {
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
            1 => rp_id = Some(c.text()?),
            3 => {
                // allowList: array of credential descriptor maps
                let b = c.byte()?;
                if b >> 5 != 4 {
                    return None;
                }
                let n = c.argument(b & 0x1f)?;
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
                        if b >> 5 != 2 {
                            return None;
                        }
                        let len = c.argument(b & 0x1f)? as usize;
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
    auth_data.push(0x00);
    auth_data.extend_from_slice(&[0, 0, 0, 0]);

    let mut out = vec![0x00]; // CTAP2 success
    out.push(0xa3); // map(3)
                    // 1: credential descriptor {"id": cred_id, "type": "public-key"}
    out.push(0x01);
    out.push(0xa2);
    encode_text(&mut out, "id");
    encode_bstr(&mut out, &cred_id);
    encode_text(&mut out, "type");
    encode_text(&mut out, "public-key");
    // 2: authData
    out.push(0x02);
    encode_bstr(&mut out, &auth_data);
    // 3: signature (dummy)
    out.push(0x03);
    encode_bstr(&mut out, &[0u8; 8]);
    Some(out)
}

fn encode_bstr(out: &mut Vec<u8>, data: &[u8]) {
    let len = data.len();
    if len < 24 {
        out.push(0x40 | len as u8);
    } else if len <= 0xff {
        out.extend_from_slice(&[0x58, len as u8]);
    } else {
        out.push(0x59);
        out.extend_from_slice(&(len as u16).to_be_bytes());
    }
    out.extend_from_slice(data);
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
    let Some((&0x01, params)) = command.split_first() else {
        return command.to_vec();
    };

    let result = (|| -> Option<Vec<u8>> {
        let mut c = CborCursor {
            data: params,
            pos: 0,
        };
        let pairs = c.map_header()?;

        let mut out = vec![0x01];
        let mut entries: Vec<(u64, std::ops::Range<usize>)> = Vec::new();
        for _ in 0..pairs {
            let key = c.uint()?;
            let start = c.pos;
            c.skip_value()?;
            entries.push((key, start..c.pos));
        }

        let mut changed = false;
        encode_map_header(&mut out, pairs);
        for (key, span) in entries {
            let value = &params[span.clone()];
            let patched = match key {
                2 => {
                    // rp entity: add "name" if missing
                    patch_text_map(value, "name", "").map(|(v, _)| v)
                }
                3 => {
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
            encode_uint(&mut out, key);
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

fn encode_uint(out: &mut Vec<u8>, v: u64) {
    if v < 24 {
        out.push(v as u8);
    } else if v <= 0xff {
        out.extend_from_slice(&[0x18, v as u8]);
    } else if v <= 0xffff {
        out.push(0x19);
        out.extend_from_slice(&(v as u16).to_be_bytes());
    } else {
        out.push(0x1a);
        out.extend_from_slice(&(v as u32).to_be_bytes());
    }
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
            0..=23 => Some(info as u64),
            24 => Some(self.byte()? as u64),
            25 => {
                let b = self.take(2)?;
                Some(u16::from_be_bytes([b[0], b[1]]) as u64)
            }
            26 => {
                let b = self.take(4)?;
                Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64)
            }
            27 => {
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
        if b >> 5 != 0 {
            return None;
        }
        self.argument(b & 0x1f)
    }

    fn map_header(&mut self) -> Option<u64> {
        let b = self.byte()?;
        if b >> 5 != 5 {
            return None;
        }
        self.argument(b & 0x1f)
    }

    fn text(&mut self) -> Option<String> {
        let b = self.byte()?;
        if b >> 5 != 3 {
            return None;
        }
        let len = self.argument(b & 0x1f)? as usize;
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
        let info = b & 0x1f;
        match major {
            0 | 1 | 7 => {
                self.argument(info)?;
            }
            2 | 3 => {
                let len = self.argument(info)?;
                self.take(len as usize)?;
            }
            4 => {
                let n = self.argument(info)?;
                for _ in 0..n {
                    self.skip_value_at(depth + 1)?;
                }
            }
            5 => {
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
