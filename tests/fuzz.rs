// SPDX-License-Identifier: GPL-3.0-or-later

//! Property tests for the parsers that face untrusted input: the CBOR
//! decoder, BLE advert suffix, QR URL, CTAP command patching, CTAPHID
//! framing, and EID decryption. None of them may panic or wedge on arbitrary
//! bytes; where a valid encoding exists, mutated variants are fuzzed too.

use proptest::prelude::*;

use ucabled::qr::RequestType;
use ucabled::{advert, cbor, ctap, ctaphid, eid, qr};

fn arb_bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec(any::<u8>(), 0..=max)
}

/// `base` with up to 8 bytes additively perturbed.
fn mutated(base: Vec<u8>) -> impl Strategy<Value = Vec<u8>> {
    proptest::collection::vec((any::<proptest::sample::Index>(), any::<u8>()), 0..8).prop_map(
        move |edits| {
            let mut v = base.clone();
            for (at, by) in edits {
                let at = at.index(v.len());
                v[at] = v[at].wrapping_add(by);
            }
            v
        },
    )
}

fn valid_qr_url() -> String {
    qr::encode_qr_url(
        &[0x02; qr::COMPRESSED_PUBLIC_KEY_SIZE],
        &[0x42; qr::QR_SECRET_SIZE],
        2,
        false,
        RequestType::GetAssertion,
        &[qr::TRANSPORT_WEBSOCKET, qr::TRANSPORT_BLE],
    )
}

/// A valid getAssertion command with a one-entry allowList, built with the
/// canonical encoder.
fn valid_ctap_command() -> Vec<u8> {
    let mut v = vec![ctap::CMD_GET_ASSERTION];
    let mut params = Vec::new();
    cbor::map(&mut params, 2);
    cbor::uint(&mut params, 1);
    cbor::text(&mut params, "example.com");
    cbor::uint(&mut params, 2);
    cbor::bytes(&mut params, &[0xaa; 32]);
    v.extend_from_slice(&params);
    v
}

proptest! {
    #[test]
    fn cbor_skip_value_never_panics(data in arb_bytes(256)) {
        let mut d = cbor::Decoder::new(&data);
        let _ = d.skip_value();
        prop_assert!(d.pos() <= data.len());
    }

    #[test]
    fn advert_suffix_never_panics(data in arb_bytes(64)) {
        let _ = advert::parse_psm(&data);
    }

    #[test]
    fn qr_url_never_panics(url in any::<String>()) {
        let _ = qr::parse_qr_url(&url);
    }

    /// Mutations of a valid QR: the digit payload is still well-formed
    /// decimal, so this exercises the CBOR layer rather than the digit
    /// decoder's reject path.
    #[test]
    fn qr_url_mutations_never_panic(
        edits in proptest::collection::vec((any::<proptest::sample::Index>(), 0..10u8), 0..8),
    ) {
        let mut url = valid_qr_url().into_bytes();
        for (at, digit) in edits {
            // Never touch the "FIDO:/" scheme prefix.
            let at = 6 + at.index(url.len() - 6);
            url[at] = b'0' + digit;
        }
        let url = String::from_utf8(url).unwrap();
        let _ = qr::parse_qr_url(&url);
    }

    #[test]
    fn ctap_commands_never_panic(
        data in prop_oneof![
            arb_bytes(256),
            (Just(ctap::CMD_MAKE_CREDENTIAL), arb_bytes(255))
                .prop_map(|(cmd, mut rest)| { rest.insert(0, cmd); rest }),
            (Just(ctap::CMD_GET_ASSERTION), arb_bytes(255))
                .prop_map(|(cmd, mut rest)| { rest.insert(0, cmd); rest }),
            mutated(valid_ctap_command()),
        ],
    ) {
        let _ = ctap::extract_rp_id(&data);
        let _ = ctap::is_silent_probe(&data);
        let _ = ctap::is_blink_probe(&data);
        let _ = ctap::request_has_transport_hints(&data);
        // The patchers must themselves produce re-parseable (or identical)
        // output.
        let patched = ctap::patch_makecredential(&data);
        let stripped = ctap::strip_transport_hints(&data);
        let _ = ctap::extract_rp_id(&patched);
        let _ = ctap::extract_rp_id(&stripped);
    }

    #[test]
    fn eid_decrypt_never_panics(advert_bytes in arb_bytes(64), key_byte in any::<u8>()) {
        let key = [key_byte; eid::EID_KEY_SIZE];
        let _ = eid::decrypt(&advert_bytes, &key);
    }

    /// A valid EID advert with mutations: decryption must either fail or
    /// yield a plaintext, never panic.
    #[test]
    fn eid_decrypt_mutations_never_panic(perturbed in mutated(eid_sample())) {
        let key = [7u8; eid::EID_KEY_SIZE];
        let _ = eid::decrypt(&perturbed, &key);
    }

    #[test]
    fn ctaphid_reports_never_panic(
        reports in proptest::collection::vec(
            prop_oneof![
                arb_bytes(70),
                // Exactly-sized reports reach the interesting paths.
                proptest::collection::vec(any::<u8>(), ctaphid::REPORT_SIZE),
            ],
            0..16,
        ),
    ) {
        let mut transport = ctaphid::Transport::new([0u8; 16]);
        for report in &reports {
            let (responses, _action) = transport.handle_report(report);
            // Every response frame must be exactly one HID report.
            for r in responses {
                prop_assert_eq!(r.len(), ctaphid::REPORT_SIZE);
            }
        }
        // Liveness: arbitrary traffic must not wedge the transport; a valid
        // INIT on the broadcast channel always gets an answer.
        let mut init = vec![0u8; ctaphid::REPORT_SIZE];
        init[0..4].copy_from_slice(&ctaphid::BROADCAST_CID.to_be_bytes());
        init[4] = ctaphid::CMD_INIT;
        init[5..7].copy_from_slice(&8u16.to_be_bytes());
        let (responses, _) = transport.handle_report(&init);
        prop_assert_eq!(responses.len(), 1);
    }
}

fn eid_sample() -> Vec<u8> {
    let key = [7u8; eid::EID_KEY_SIZE];
    let plaintext = [0u8; eid::EID_PLAINTEXT_SIZE];
    eid::encrypt(&plaintext, &key).to_vec()
}
