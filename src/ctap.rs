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
pub const CMD_GET_NEXT_ASSERTION: u8 = 0x08;

// CTAP status bytes (CTAP 2.2 §8.2) used on this relay.
pub const CTAP2_OK: u8 = 0x00;
pub const CTAP1_ERR_INVALID_COMMAND: u8 = 0x01;
pub const CTAP1_ERR_TIMEOUT: u8 = 0x05;
pub const CTAP2_ERR_CBOR_UNEXPECTED_TYPE: u8 = 0x11;
pub const CTAP2_ERR_INVALID_OPTION: u8 = 0x2c;
pub const CTAP2_ERR_KEEPALIVE_CANCEL: u8 = 0x2d;
pub const CTAP2_ERR_NO_CREDENTIALS: u8 = 0x2e;
pub const CTAP2_ERR_NOT_ALLOWED: u8 = 0x32;
pub const CTAP2_ERR_PIN_NOT_SET: u8 = 0x35;

// Map keys of the CTAP2 requests parsed locally (CTAP 2.2 §6.1/§6.2).
const MC_KEY_RP: u64 = 2;
const MC_KEY_USER: u64 = 3;
const MC_KEY_EXCLUDE_LIST: u64 = 5;
const GA_KEY_RPID: u64 = 1;
const GA_KEY_ALLOW_LIST: u64 = 3;
const GA_KEY_OPTIONS: u64 = 5;

// Map keys of the getInfo response built locally (CTAP 2.2 §6.4).
const GETINFO_KEY_VERSIONS: u64 = 1;
const GETINFO_KEY_EXTENSIONS: u64 = 2;
const GETINFO_KEY_AAGUID: u64 = 3;
const GETINFO_KEY_OPTIONS: u64 = 4;
const GETINFO_KEY_MAX_MSG_SIZE: u64 = 5;
const GETINFO_KEY_MAX_CRED_COUNT_IN_LIST: u64 = 7;
const GETINFO_KEY_MAX_CRED_ID_LENGTH: u64 = 8;
const GETINFO_KEY_TRANSPORTS: u64 = 9;

// Map keys of the getAssertion response built locally (CTAP 2.2 §6.2).
const GA_RESP_KEY_CREDENTIAL: u64 = 1;
const GA_RESP_KEY_AUTH_DATA: u64 = 2;
const GA_RESP_KEY_SIGNATURE: u64 = 3;
const GA_RESP_KEY_NUMBER_OF_CREDENTIALS: u64 = 5;

// Synthetic assertion fields (see `fake_silent_assertion`).
/// authData flags byte: user presence and user verification unset.
const AUTH_DATA_FLAGS: u8 = 0x00;
/// Signature counter width in authData.
const AUTH_DATA_SIGN_COUNTER_LEN: usize = 4;
/// Placeholder (all-zero) signature length.
const DUMMY_SIGNATURE_LEN: usize = 8;

/// GetInfo advertises this maxMsgSize (CTAP 2.2 §6.4 default upper bound).
pub const MAX_MSG_SIZE: u64 = 7609;

/// Advertised `maxCredentialCountInList`. Firefox/Chromium use this to decide
/// how many credential descriptors to put in one preflight `getAssertion` (and
/// to chunk the allowList). Relaying means there is no real per-authenticator
/// limit, so advertise a value large enough to keep a typical RP's whole
/// allowList in one request: chunking would let Firefox keep only the first
/// chunk (see `fake_silent_assertion`).
pub const MAX_CREDENTIAL_COUNT_IN_LIST: u64 = 64;

/// Advertised `maxCredentialIdLength`: Firefox drops allowList entries longer
/// than this before the preflight, so keep it generous.
pub const MAX_CREDENTIAL_ID_LENGTH: u64 = 1024;

/// authenticatorGetInfo response (status byte + canonical CBOR map).
///
/// Advertised per FR-5: FIDO_2_0 only (no U2F_V2), the `credProtect`
/// extension, rk/up/uv, no clientPin, maxMsgSize [`MAX_MSG_SIZE`], the
/// credential-list limits ([`MAX_CREDENTIAL_COUNT_IN_LIST`],
/// [`MAX_CREDENTIAL_ID_LENGTH`]) and transports ["hybrid"].
///
/// `credProtect` is advertised because OpenSSH refuses to create resident or
/// verify-required `sk` keys unless the authenticator reports it; the actual
/// credential (and its protection) is created by the phone, which receives
/// the extension in the relayed makeCredential.
pub fn getinfo_response(aaguid: &[u8; 16]) -> Vec<u8> {
    let mut out = vec![CTAP2_OK];

    cbor::map(&mut out, 8);

    cbor::uint(&mut out, GETINFO_KEY_VERSIONS);
    cbor::array(&mut out, 1);
    cbor::text(&mut out, "FIDO_2_0");

    cbor::uint(&mut out, GETINFO_KEY_EXTENSIONS);
    cbor::array(&mut out, 1);
    cbor::text(&mut out, "credProtect");

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

    // Without these, Firefox/Chromium preflight the allowList one credential at
    // a time; because the faked silent answer below always reports success,
    // only the first credential would survive into the real getAssertion.
    cbor::uint(&mut out, GETINFO_KEY_MAX_CRED_COUNT_IN_LIST);
    cbor::uint(&mut out, MAX_CREDENTIAL_COUNT_IN_LIST);
    cbor::uint(&mut out, GETINFO_KEY_MAX_CRED_ID_LENGTH);
    cbor::uint(&mut out, MAX_CREDENTIAL_ID_LENGTH);

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
    let mut c = cbor::Decoder::new(params);
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
    let mut c = cbor::Decoder::new(data);
    let pairs = c.map_header()?;
    let header_len = c.pos();

    let mut entries: Vec<(String, std::ops::Range<usize>)> = Vec::new();
    let mut have_key = false;
    for _ in 0..pairs {
        let k = c.text()?;
        if k == add_key {
            have_key = true;
        }
        let start = c.pos();
        c.skip_value()?;
        entries.push((k, start..c.pos()));
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
    Some((out, c.pos()))
}

/// Detect Firefox's silent getAssertion probe: `options.up == false` **and** a
/// non-empty allowList. Firefox sends these (with an empty clientDataHash) to
/// filter the allowList to credentials present on this device; a phone cannot
/// answer silently, so the daemon answers locally with
/// [`fake_silent_assertion`].
///
/// The allowList is required. A `up=false` assertion without one is a
/// discoverable (resident) getAssertion — `ssh-keygen -t ecdsa-sk -O resident`
/// sends one with rpId `ssh:` — which has nothing to filter and must be
/// relayed to the phone instead.
pub fn is_silent_probe(command: &[u8]) -> bool {
    let Some((&CMD_GET_ASSERTION, params)) = command.split_first() else {
        return false;
    };
    let mut c = cbor::Decoder::new(params);
    let Some(pairs) = c.map_header() else {
        return false;
    };
    let (mut has_allow_list, mut up_false) = (false, false);
    for _ in 0..pairs {
        let Some(key) = c.uint() else { return false };
        match key {
            GA_KEY_ALLOW_LIST => {
                let Some(b) = c.byte() else { return false };
                if b >> 5 != cbor::MAJOR_ARRAY {
                    return false;
                }
                let Some(n) = c.argument(b & cbor::ARG_MASK) else {
                    return false;
                };
                has_allow_list = n > 0;
                for _ in 0..n {
                    if c.skip_value().is_none() {
                        return false;
                    }
                }
            }
            GA_KEY_OPTIONS => {
                let Some(inner) = c.map_header() else {
                    return false;
                };
                for _ in 0..inner {
                    let Some(k) = c.text() else { return false };
                    if k == "up" {
                        let Some(b) = c.byte() else { return false };
                        up_false = b == cbor::FALSE;
                    } else if c.skip_value().is_none() {
                        return false;
                    }
                }
            }
            _ => {
                if c.skip_value().is_none() {
                    return false;
                }
            }
        }
    }
    has_allow_list && up_false
}

/// Parse a getAssertion request into `(rpId, credential ids)`, collecting
/// *every* allowList credential (not just the first). Returns None on any
/// parse irregularity or if there is no allowList.
fn parse_assertion_targets(command: &[u8]) -> Option<(String, Vec<Vec<u8>>)> {
    let (&CMD_GET_ASSERTION, params) = command.split_first()? else {
        return None;
    };
    let mut c = cbor::Decoder::new(params);
    let pairs = c.map_header()?;

    let mut rp_id: Option<String> = None;
    let mut creds: Vec<Vec<u8>> = Vec::new();
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
                for _ in 0..n {
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
                    creds.push(cred_id?);
                }
            }
            _ => c.skip_value()?,
        }
    }

    Some((rp_id?, creds))
}

/// Build one structurally valid getAssertion "success" response echoing
/// `cred_id`, for answering Firefox's silent preflight probe (up=false) without
/// bothering the phone. When `total > 1`, the `numberOfCredentials` field is
/// set so the client fetches the rest with `authenticatorGetNextAssertion`;
/// the daemon serves those from [`fake_next_assertion`]. Firefox only extracts
/// the credential descriptors from these; the real interactive request still
/// goes to the phone with the whole allowList intact. The fake signatures never
/// leave the browser's local filtering code.
fn fake_assertion_response(rp_id: &str, cred_id: &[u8], total: usize) -> Vec<u8> {
    use sha2::{Digest, Sha256};

    // authData = rpIdHash || flags(0) || counter(0)
    let mut auth_data = Sha256::digest(rp_id.as_bytes()).to_vec();
    auth_data.push(AUTH_DATA_FLAGS); // no UP/UV bits
    auth_data.extend_from_slice(&[0u8; AUTH_DATA_SIGN_COUNTER_LEN]);

    let mut out = vec![CTAP2_OK];
    cbor::map(&mut out, if total > 1 { 4 } else { 3 });
    // credential descriptor {"id": cred_id, "type": "public-key"}
    cbor::uint(&mut out, GA_RESP_KEY_CREDENTIAL);
    cbor::map(&mut out, 2);
    cbor::text(&mut out, "id");
    cbor::bytes(&mut out, cred_id);
    cbor::text(&mut out, "type");
    cbor::text(&mut out, "public-key");
    cbor::uint(&mut out, GA_RESP_KEY_AUTH_DATA);
    cbor::bytes(&mut out, &auth_data);
    cbor::uint(&mut out, GA_RESP_KEY_SIGNATURE);
    cbor::bytes(&mut out, &[0u8; DUMMY_SIGNATURE_LEN]);
    if total > 1 {
        cbor::uint(&mut out, GA_RESP_KEY_NUMBER_OF_CREDENTIALS);
        cbor::uint(&mut out, total as u64);
    }
    out
}

/// Build the first faked response for a silent preflight probe, plus the
/// remaining credential ids to serve from `getNextAssertion` responses.
///
/// Returns None (caller answers with `NO_CREDENTIALS`) if the request doesn't
/// parse or has no allowList.
pub fn fake_silent_assertion(command: &[u8]) -> Option<(Vec<u8>, Vec<Vec<u8>>)> {
    let (rp_id, creds) = parse_assertion_targets(command)?;
    let (first, rest) = creds.split_first()?;
    let response = fake_assertion_response(&rp_id, first, creds.len());
    Some((response, rest.to_vec()))
}

/// Build the faked response for one `authenticatorGetNextAssertion` follow-up
/// to [`fake_silent_assertion`].
pub fn fake_next_assertion(rp_id: &str, cred_id: &[u8]) -> Vec<u8> {
    fake_assertion_response(rp_id, cred_id, 1)
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
        let mut c = cbor::Decoder::new(params);
        let pairs = c.map_header()?;

        let mut out = vec![CMD_MAKE_CREDENTIAL];
        let mut entries: Vec<(u64, std::ops::Range<usize>)> = Vec::new();
        for _ in 0..pairs {
            let key = c.uint()?;
            let start = c.pos();
            c.skip_value()?;
            entries.push((key, start..c.pos()));
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
                        let mut inner = cbor::Decoder::new(value);
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

/// Remove the advisory `transports` hint from every credential descriptor in a
/// getAssertion `allowList` (key 3) or makeCredential `excludeList` (key 5).
///
/// Firefox tags credentials it learned over USB-HID with
/// `transports: ["usb"]`. Forwarded untouched over caBLE, some phone
/// authenticators reject the credential as incompatible with the hybrid
/// transport ("no passkeys found"). The hint is advisory and covered by no
/// signature, so dropping it is safe. Descriptor order and every other field
/// are preserved byte-for-byte. Returns the input unchanged if nothing needed
/// stripping or the payload doesn't parse.
pub fn strip_transport_hints(command: &[u8]) -> Vec<u8> {
    let Some((&cmd, params)) = command.split_first() else {
        return command.to_vec();
    };
    // CTAP2 parameter numbering: getAssertion.allowList = 3,
    // makeCredential.excludeList = 5 (4 is pubKeyCredParams).
    let list_key = match cmd {
        CMD_MAKE_CREDENTIAL => MC_KEY_EXCLUDE_LIST,
        CMD_GET_ASSERTION => GA_KEY_ALLOW_LIST,
        _ => return command.to_vec(),
    };

    let result = (|| -> Option<Vec<u8>> {
        let mut c = cbor::Decoder::new(params);
        let pairs = c.map_header()?;

        let mut out = vec![cmd];
        let mut entries: Vec<(u64, std::ops::Range<usize>)> = Vec::new();
        for _ in 0..pairs {
            let key = c.uint()?;
            let start = c.pos();
            c.skip_value()?;
            entries.push((key, start..c.pos()));
        }

        let mut changed = false;
        cbor::map(&mut out, pairs);
        for (key, span) in entries {
            cbor::uint(&mut out, key);
            let value = &params[span.clone()];
            if key == list_key {
                if let Some((stripped, true)) = strip_transports_from_array(value) {
                    changed = true;
                    out.extend_from_slice(&stripped);
                    continue;
                }
            }
            out.extend_from_slice(value);
        }
        changed.then_some(out)
    })();

    match result {
        Some(out) => {
            tracing::debug!("stripped transports hint(s) from CTAP request");
            out
        }
        None => command.to_vec(),
    }
}

/// Drop a descriptor's `transports` entry while keeping the remaining pairs in
/// order. Returns `(rebuilt, removed)`; `None` when `data` is not a text-keyed
/// map or lacks an `id`, so callers can fail safe and keep the original.
fn strip_transports_from_descriptor(data: &[u8]) -> Option<(Vec<u8>, bool)> {
    let mut c = cbor::Decoder::new(data);
    let pairs = c.map_header()?;

    let mut kept: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> = Vec::new();
    let mut removed = false;
    let mut has_id = false;
    for _ in 0..pairs {
        let key_start = c.pos();
        let key = c.text()?;
        let key_end = c.pos();
        let val_start = c.pos();
        c.skip_value()?;
        let val_end = c.pos();
        if key == "transports" {
            removed = true;
        } else {
            has_id |= key == "id";
            kept.push((key_start..key_end, val_start..val_end));
        }
    }
    if !has_id {
        return None;
    }

    let mut out = Vec::new();
    cbor::map(&mut out, kept.len() as u64);
    for (k, v) in kept {
        out.extend_from_slice(&data[k]);
        out.extend_from_slice(&data[v]);
    }
    Some((out, removed))
}

/// Strip transport hints from every descriptor in an array. Returns
/// `(rebuilt, removed)`; `None` when `data` is not an array.
fn strip_transports_from_array(data: &[u8]) -> Option<(Vec<u8>, bool)> {
    let mut c = cbor::Decoder::new(data);
    let b = c.byte()?;
    if b >> 5 != cbor::MAJOR_ARRAY {
        return None;
    }
    let n = c.argument(b & cbor::ARG_MASK)?;

    let mut out = Vec::new();
    cbor::array(&mut out, n);
    let mut removed = false;
    for _ in 0..n {
        let start = c.pos();
        c.skip_value()?;
        let elem = &data[start..c.pos()];
        match strip_transports_from_descriptor(elem) {
            Some((stripped, true)) => {
                removed = true;
                out.extend_from_slice(&stripped);
            }
            _ => out.extend_from_slice(elem),
        }
    }
    Some((out, removed))
}

/// Diagnostic: whether the request carries any `transports` hint in its
/// getAssertion `allowList` (key 3) or makeCredential `excludeList` (key 5)
/// descriptors. Read-only, never fails (returns false on any irregularity).
/// Logged by the daemon so we can tell whether Firefox ever sends the hint.
pub fn request_has_transport_hints(command: &[u8]) -> bool {
    let Some((&cmd, params)) = command.split_first() else {
        return false;
    };
    let list_key = match cmd {
        CMD_MAKE_CREDENTIAL => MC_KEY_EXCLUDE_LIST,
        CMD_GET_ASSERTION => GA_KEY_ALLOW_LIST,
        _ => return false,
    };

    let result = (|| -> Option<bool> {
        let mut c = cbor::Decoder::new(params);
        let pairs = c.map_header()?;
        for _ in 0..pairs {
            let key = c.uint()?;
            if key == list_key {
                return Some(array_has_transport_hint(&mut c));
            }
            c.skip_value()?;
        }
        Some(false)
    })();

    result.unwrap_or(false)
}

fn array_has_transport_hint(c: &mut cbor::Decoder) -> bool {
    let Some(b) = c.byte() else { return false };
    if b >> 5 != cbor::MAJOR_ARRAY {
        return false;
    }
    let Some(n) = c.argument(b & cbor::ARG_MASK) else {
        return false;
    };
    for _ in 0..n {
        let start = c.pos();
        if c.skip_value().is_none() {
            return false;
        }
        let elem = &c.data()[start..c.pos()];
        if descriptor_has_transport_hint(elem) {
            return true;
        }
    }
    false
}

fn descriptor_has_transport_hint(data: &[u8]) -> bool {
    let mut c = cbor::Decoder::new(data);
    let Some(pairs) = c.map_header() else {
        return false;
    };
    for _ in 0..pairs {
        let Some(key) = c.text() else { return false };
        if key == "transports" {
            return true;
        }
        if c.skip_value().is_none() {
            return false;
        }
    }
    false
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
        // credProtect is required for OpenSSH resident / verify-required keys.
        assert!(s.contains("credProtect"));
        // maxMsgSize must use the minimal two-byte encoding (canonical CBOR),
        // otherwise strict parsers (Chromium) reject the whole response.
        assert!(r.windows(3).any(|w| w == [0x19, 0x1d, 0xb9]));
        assert!(!r.windows(5).any(|w| w == [0x1a, 0x00, 0x00, 0x1d, 0xb9]));
        // Advertise the list limits so the browser does not preflight the
        // allowList one credential at a time (key 7 -> 64, key 8 -> 1024).
        assert!(r.windows(3).any(|w| w == [0x07, 0x18, 0x40]));
        assert!(r.windows(4).any(|w| w == [0x08, 0x19, 0x04, 0x00]));
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

    /// `{"id": h'0102', "type": "public-key", "transports": ["usb"]}`
    const DESC_WITH_TRANSPORTS: &str =
        "a362696442010264747970656a7075626c69632d6b65796a7472616e73706f7274738163757362";

    /// The same descriptor without the transports hint.
    const DESC_NO_TRANSPORTS: &str = "a262696442010264747970656a7075626c69632d6b6579";

    fn get_assertion_with(descriptor_hex: &str) -> Vec<u8> {
        let mut cmd = hex::decode("02a2016b6578616d706c652e636f6d0381").unwrap();
        cmd.extend_from_slice(&hex::decode(descriptor_hex).unwrap());
        cmd
    }

    fn get_assertion_with_transport_hint() -> Vec<u8> {
        get_assertion_with(DESC_WITH_TRANSPORTS)
    }

    #[test]
    fn strip_transport_hints_get_assertion() {
        let cmd = get_assertion_with_transport_hint();
        let out = strip_transport_hints(&cmd);

        assert_ne!(out, cmd);
        assert!(!String::from_utf8_lossy(&out).contains("transports"));
        // the credential id (bstr 42 01 02) survives
        assert!(hex::encode(&out).contains("420102"));
        // the rest of the request is untouched
        assert_eq!(extract_rp_id(&out).as_deref(), Some("example.com"));
        // idempotent
        assert_eq!(strip_transport_hints(&out), out);
    }

    #[test]
    fn strip_transport_hints_make_credential_exclude_list() {
        // {1: h'aa'x32, 2: {"id": "example.com"}, 5: [descriptor-with-transports]}
        let mut cmd = hex::decode("01a3015820").unwrap();
        cmd.extend_from_slice(&[0xaa; 32]);
        cmd.extend_from_slice(&hex::decode("02a16269646b6578616d706c652e636f6d0581").unwrap());
        cmd.extend_from_slice(&hex::decode(DESC_WITH_TRANSPORTS).unwrap());

        let out = strip_transport_hints(&cmd);
        assert_ne!(out, cmd);
        assert!(!String::from_utf8_lossy(&out).contains("transports"));
        assert!(hex::encode(&out).contains("420102"));
        assert_eq!(strip_transport_hints(&out), out);
    }

    #[test]
    fn strip_transport_hints_noop_without_hint() {
        // Interactive getAssertion with an allowList but no transports.
        let cmd = get_assertion_with(DESC_NO_TRANSPORTS);
        assert_eq!(strip_transport_hints(&cmd), cmd);
        // Unrelated command byte is left alone.
        assert_eq!(strip_transport_hints(&[0x06, 0x00]), vec![0x06, 0x00]);
    }

    #[test]
    fn request_has_transport_hints_detects_and_defaults_false() {
        assert!(request_has_transport_hints(
            &get_assertion_with_transport_hint()
        ));
        assert!(!request_has_transport_hints(&get_assertion_with(
            DESC_NO_TRANSPORTS
        )));
        // makeCredential excludeList (key 5)
        let mut mc = hex::decode("01a3015820").unwrap();
        mc.extend_from_slice(&[0xaa; 32]);
        mc.extend_from_slice(&hex::decode("02a16269646b6578616d706c652e636f6d0581").unwrap());
        mc.extend_from_slice(&hex::decode(DESC_WITH_TRANSPORTS).unwrap());
        assert!(request_has_transport_hints(&mc));
        // unrelated / empty
        assert!(!request_has_transport_hints(&[0x06, 0x00]));
        assert!(!request_has_transport_hints(&[]));
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
        // Captured from `ssh-keygen -t ecdsa-sk -O resident`: a discoverable
        // getAssertion for rpId "ssh:" with options {up:false, uv:true} and
        // no allowList. It must be relayed, not answered locally.
        let ssh_resident = hex::decode(
            "02a301647373683a02582066687aadf862bd776c8fc18b8e9f8e20\
             089714856ee233b3902a591d0d5f292505a2627570f4627576f5",
        )
        .unwrap();
        assert!(!is_silent_probe(&ssh_resident));
    }

    #[test]
    fn fake_silent_assertion_echoes_credential() {
        let probe = hex::decode("02a4016b776562617574686e2e696f025820e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b8550381a26269645456ebb4bab2368cd5c71e173c5213c85203e2511964747970656a7075626c69632d6b657905a1627570f4").unwrap();
        let (resp, rest) = fake_silent_assertion(&probe).unwrap();
        assert!(rest.is_empty());
        assert_eq!(resp[0], 0x00);
        // echoes the credential id 56ebb4bab2368cd5c71e173c5213c85203e25119
        let cred = hex::decode("56ebb4bab2368cd5c71e173c5213c85203e25119").unwrap();
        assert!(resp.windows(cred.len()).any(|w| w == cred.as_slice()));
        assert!(resp.windows(10).any(|w| w == b"public-key"));
        // single credential: no numberOfCredentials field
        assert!(!resp.ends_with(&[0x05, 0x01]));
        // getAssertion without allowList cannot be faked
        let plain = hex::decode("02a2016b6578616d706c652e636f6d025820aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        assert!(fake_silent_assertion(&plain).is_none());
    }

    #[test]
    fn fake_silent_assertion_reports_all_credentials() {
        // {1: "example.com", 2: h'aa'x32, 3: [{"id": h'0102'}, {"id": h'0304'}]}
        let mut two = hex::decode("02a4016b6578616d706c652e636f6d025820").unwrap();
        two.extend_from_slice(&[0xaa; 32]);
        two.extend_from_slice(&hex::decode("0382").unwrap());
        two.extend_from_slice(
            &hex::decode("a262696442010264747970656a7075626c69632d6b6579").unwrap(),
        );
        two.extend_from_slice(
            &hex::decode("a262696442030464747970656a7075626c69632d6b6579").unwrap(),
        );
        two.extend_from_slice(&hex::decode("05a1627570f5").unwrap());

        let (resp, rest) = fake_silent_assertion(&two).unwrap();
        assert_eq!(resp[0], 0x00);
        // numberOfCredentials = 2 (key 5) so the client asks for the rest
        assert_eq!(&resp[resp.len() - 2..], &[0x05, 0x02]);
        assert_eq!(rest, vec![vec![0x03, 0x04]]);

        // the follow-up echoes the second credential without a count field
        let next = fake_next_assertion("example.com", &[0x03, 0x04]);
        assert_eq!(next[0], 0x00);
        assert!(next.windows(2).any(|w| w == [0x03, 0x04]));
        assert!(!next.windows(2).any(|w| w == [0x05, 0x01]));
    }
}
