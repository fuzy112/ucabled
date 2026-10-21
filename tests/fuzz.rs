// SPDX-License-Identifier: GPL-3.0-or-later

//! Property tests for the parsers that face untrusted input: the CBOR
//! decoder, BLE advert suffix, QR URL, CTAP command patching, CTAPHID
//! framing, and EID decryption. None of them may panic or wedge on arbitrary
//! bytes; where a valid encoding exists, mutated variants are fuzzed too.
//! The CTAPHID transport additionally gets a structured command storm: every
//! defined command byte, which must never yield CTAP1_ERR_OTHER (the
//! catch-all internal-failure status) or a wedged channel.

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
        let _ = ctap::answer_silent_probe(&data);
        let _ = ctap::is_blink_probe(&data);
        // The patchers must themselves produce re-parseable (or identical)
        // output.
        let patched = ctap::patch_makecredential(&data);
        let dissociated = ctap::dissociate_ssh_user(&data);
        let stripped = ctap::strip_transport_hints(&data);
        let _ = ctap::extract_rp_id(&patched);
        let _ = ctap::extract_rp_id(&dissociated);
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

/// CTAP1_ERR_OTHER (CTAP 2.2 §8.1): the catch-all status an authenticator
/// returns when something failed internally.  The storm test below asserts
/// the transport never answers with it; ctap.rs does not define the
/// constant because no code path is allowed to produce it.
const CTAP1_ERR_OTHER: u8 = 0x7f;

fn init_frame(cid: u32, cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; ctaphid::REPORT_SIZE];
    r[0..4].copy_from_slice(&cid.to_be_bytes());
    r[4] = cmd;
    r[5..7].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    let take = payload.len().min(ctaphid::REPORT_SIZE - 7);
    r[7..7 + take].copy_from_slice(&payload[..take]);
    r
}

fn arb_cid() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(ctaphid::BROADCAST_CID),
        Just(0), // reserved, must be rejected
        1u32..=4,
        any::<u32>(),
    ]
}

/// Every defined CTAPHID command plus undefined command bytes.
fn arb_ctaphid_cmd() -> impl Strategy<Value = u8> {
    prop_oneof![
        Just(ctaphid::CMD_PING),
        Just(ctaphid::CMD_MSG),
        Just(ctaphid::CMD_LOCK),
        Just(ctaphid::CMD_INIT),
        Just(ctaphid::CMD_WINK),
        Just(ctaphid::CMD_CBOR),
        Just(ctaphid::CMD_CANCEL),
        any::<u8>(),
    ]
}

/// Every CTAP2 subcommand the daemon relays or answers, plus ones it must
/// reject.
fn arb_ctap_subcmd() -> impl Strategy<Value = u8> {
    prop_oneof![
        Just(ctap::CMD_MAKE_CREDENTIAL),
        Just(ctap::CMD_GET_ASSERTION),
        Just(ctap::CMD_GET_INFO),
        Just(ctap::CMD_GET_NEXT_ASSERTION),
        Just(0x06u8), // clientPIN
        Just(0x07u8), // reset
        Just(0x0au8), // credentialManagement
        any::<u8>(),
    ]
}

/// Reports biased toward well-formed frames with structured commands, so
/// the dispatch paths (not just the framing rejects) get exercised.
fn arb_report() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        // Single-frame INIT with a structured CTAPHID command.
        (arb_cid(), arb_ctaphid_cmd(), arb_bytes(57))
            .prop_map(|(cid, cmd, p)| init_frame(cid, cmd, &p)),
        // CBOR carrying a structured CTAP subcommand.
        (arb_cid(), arb_ctap_subcmd(), arb_bytes(56)).prop_map(|(cid, sub, mut rest)| {
            rest.insert(0, sub);
            init_frame(cid, ctaphid::CMD_CBOR, &rest)
        }),
        // Truncated multi-frame announcement: the announced continuation
        // frames never arrive.
        (arb_cid(), arb_ctaphid_cmd(), arb_bytes(57), 1usize..700).prop_map(
            |(cid, cmd, p, extra)| {
                let mut r = init_frame(cid, cmd, &p);
                let len = (p.len() + extra) as u16;
                r[5..7].copy_from_slice(&len.to_be_bytes());
                r
            }
        ),
        // Stray continuation frame.
        (arb_cid(), any::<u8>(), arb_bytes(59)).prop_map(|(cid, seq, data)| {
            let mut r = vec![0u8; ctaphid::REPORT_SIZE];
            r[0..4].copy_from_slice(&cid.to_be_bytes());
            r[4] = seq & 0x7f;
            let take = data.len().min(ctaphid::REPORT_SIZE - 5);
            r[5..5 + take].copy_from_slice(&data[..take]);
            r
        }),
        // Raw noise.
        proptest::collection::vec(any::<u8>(), ctaphid::REPORT_SIZE),
    ]
}

fn check_responses(responses: &[Vec<u8>]) -> Result<(), TestCaseError> {
    for r in responses {
        prop_assert_eq!(r.len(), ctaphid::REPORT_SIZE);
    }
    let Some(first) = responses.first() else {
        return Ok(());
    };
    match first[4] {
        ctaphid::CMD_CBOR => {
            // The catch-all status means the authenticator failed
            // internally.  Valid local answers are success (getInfo),
            // INVALID_COMMAND, CBOR_UNEXPECTED_TYPE and KEEPALIVE_CANCEL.
            prop_assert_ne!(first[7], CTAP1_ERR_OTHER, "CTAP status 0x7f (OTHER)");
        }
        ctaphid::CMD_ERROR => {
            prop_assert!(
                matches!(
                    first[7],
                    ctaphid::ERR_INVALID_CMD
                        | ctaphid::ERR_INVALID_LEN
                        | ctaphid::ERR_INVALID_SEQ
                        | ctaphid::ERR_CHANNEL_BUSY
                        | ctaphid::ERR_INVALID_CHANNEL
                ),
                "undefined CTAPHID error code {:#04x}",
                first[7]
            );
        }
        _ => {}
    }
    Ok(())
}

proptest! {
    /// A storm of structured commands across every CTAPHID and CTAP command
    /// byte: the transport must never answer with CTAP1_ERR_OTHER, never
    /// emit an undefined error code, and never wedge the channel.  Relayed
    /// commands are resolved immediately (the storm tests the transport,
    /// not the phone link), so any busy slot left at the end belongs to a
    /// truncated assembly, which the idle-expiry path must clear.
    #[test]
    fn ctaphid_command_storm_never_wedges(
        reports in proptest::collection::vec(arb_report(), 1..64),
        decisions in proptest::collection::vec((any::<bool>(), arb_bytes(120)), 1..64),
    ) {
        let mut t = ctaphid::Transport::new([0u8; 16]);
        let mut decisions = decisions.iter().cycle();
        for report in &reports {
            let (responses, action) = t.handle_report(report);
            check_responses(&responses)?;
            if let Some(ctaphid::CtapAction::Relay(_)) = action {
                let cid = t.busy_channel().expect("relay without a busy channel");
                let (complete, payload) = decisions.next().expect("decisions is non-empty");
                // The payload stands in for the phone's answer; only the
                // framing is the transport's doing, so check shape only.
                let out = if *complete {
                    t.complete_relay(cid, payload)
                } else {
                    t.cancel_relay(cid)
                };
                for r in &out {
                    prop_assert_eq!(r.len(), ctaphid::REPORT_SIZE);
                }
            }
        }

        // Liveness: expiring whatever assembly the storm truncated must
        // restore full service — INIT allocates a channel and getInfo
        // answers with success.
        let now = std::time::Instant::now();
        let _ = t.expire_assembly(now + ctaphid::ASSEMBLY_IDLE_TIMEOUT);
        let (responses, _) = t.handle_report(&init_frame(
            ctaphid::BROADCAST_CID,
            ctaphid::CMD_INIT,
            &[1, 2, 3, 4, 5, 6, 7, 8],
        ));
        prop_assert_eq!(responses.len(), 1);
        let cid = u32::from_be_bytes(responses[0][15..19].try_into().unwrap());
        let (responses, action) = t.handle_report(&init_frame(
            cid,
            ctaphid::CMD_CBOR,
            &[ctap::CMD_GET_INFO],
        ));
        prop_assert!(action.is_none());
        check_responses(&responses)?;
        prop_assert_eq!(responses[0][4], ctaphid::CMD_CBOR);
        prop_assert_eq!(responses[0][7], ctap::CTAP2_OK);
    }
}
