// SPDX-License-Identifier: GPL-3.0-or-later

use std::collections::HashMap;

pub const REPORT_SIZE: usize = 64;
pub const MAX_PAYLOAD: usize = 7609;

pub const CMD_PING: u8 = 0x81;
pub const CMD_MSG: u8 = 0x83;
pub const CMD_LOCK: u8 = 0x84;
pub const CMD_INIT: u8 = 0x86;
pub const CMD_WINK: u8 = 0x88;
pub const CMD_CBOR: u8 = 0x90;
pub const CMD_CANCEL: u8 = 0x91;
pub const CMD_ERROR: u8 = 0xbf;
pub const CMD_KEEPALIVE: u8 = 0xbb;

pub const ERR_INVALID_CMD: u8 = 0x01;
pub const ERR_INVALID_LEN: u8 = 0x03;
pub const ERR_INVALID_SEQ: u8 = 0x04;
pub const ERR_CHANNEL_BUSY: u8 = 0x06;
pub const ERR_INVALID_CHANNEL: u8 = 0x0b;

pub const BROADCAST_CID: u32 = 0xffff_ffff;

const CAP_WINK: u8 = 0x01;
const CAP_CBOR: u8 = 0x04;

#[derive(Debug, PartialEq)]
pub enum CtapAction {
    /// CTAP2-level request payload (after the CTAPHID_CBOR command byte),
    /// to be relayed to the phone; response comes back asynchronously.
    Relay(Vec<u8>),
    /// The host cancelled the pending relay on this channel.
    CancelRelay,
}

struct Assembly {
    cmd: u8,
    total_len: usize,
    buf: Vec<u8>,
    next_seq: u8,
}

pub struct Transport {
    next_cid: u32,
    aaguid: [u8; 16],
    /// Channel currently assembling or processing a multi-frame message.
    busy: Option<u32>,
    assembly: Option<Assembly>,
    pub channels: HashMap<u32, ()>,
}

impl Transport {
    pub fn new(aaguid: [u8; 16]) -> Self {
        Self {
            next_cid: 0x0001_0001,
            aaguid,
            busy: None,
            assembly: None,
            channels: HashMap::new(),
        }
    }

    fn alloc_cid(&mut self) -> u32 {
        let cid = self.next_cid;
        self.next_cid = self.next_cid.wrapping_add(1).max(1);
        cid
    }

    /// Feed one 64-byte input report. Returns response reports to write back,
    /// plus an optional higher-level action (CTAP relay).
    pub fn handle_report(&mut self, report: &[u8]) -> (Vec<Vec<u8>>, Option<CtapAction>) {
        if report.len() != REPORT_SIZE {
            return (vec![], None);
        }
        let cid = u32::from_be_bytes(report[0..4].try_into().unwrap());
        let b4 = report[4];

        if b4 & 0x80 != 0 {
            // INIT frame
            let cmd = b4;
            let len = u16::from_be_bytes([report[5], report[6]]) as usize;
            let payload = &report[7..];
            if len > MAX_PAYLOAD {
                return (error_response(cid, ERR_INVALID_LEN), None);
            }
            if len <= payload.len() {
                self.dispatch(cid, cmd, &payload[..len])
            } else {
                // Start assembly.
                if self.busy.is_some() && self.busy != Some(cid) {
                    return (error_response(cid, ERR_CHANNEL_BUSY), None);
                }
                self.busy = Some(cid);
                self.assembly = Some(Assembly {
                    cmd,
                    total_len: len,
                    buf: payload.to_vec(),
                    next_seq: 0,
                });
                (vec![], None)
            }
        } else {
            // CONT frame
            let seq = b4;
            let Some(assembly) = &mut self.assembly else {
                return (error_response(cid, ERR_INVALID_CMD), None);
            };
            if self.busy != Some(cid) {
                return (error_response(cid, ERR_INVALID_CHANNEL), None);
            }
            if seq != assembly.next_seq {
                self.assembly = None;
                self.busy = None;
                return (error_response(cid, ERR_INVALID_SEQ), None);
            }
            assembly.next_seq += 1;
            let remaining = assembly.total_len - assembly.buf.len();
            let take = remaining.min(REPORT_SIZE - 5);
            assembly.buf.extend_from_slice(&report[5..5 + take]);
            if assembly.buf.len() >= assembly.total_len {
                let assembly = self.assembly.take().unwrap();
                self.busy = None;
                self.dispatch(cid, assembly.cmd, &assembly.buf[..assembly.total_len])
            } else {
                (vec![], None)
            }
        }
    }

    fn dispatch(
        &mut self,
        cid: u32,
        cmd: u8,
        payload: &[u8],
    ) -> (Vec<Vec<u8>>, Option<CtapAction>) {
        match cmd {
            CMD_INIT => {
                if payload.len() != 8 {
                    return (error_response(cid, ERR_INVALID_LEN), None);
                }
                let new_cid = if cid == BROADCAST_CID {
                    self.alloc_cid()
                } else {
                    cid
                };
                self.channels.insert(new_cid, ());
                let mut resp = Vec::with_capacity(17);
                resp.extend_from_slice(payload); // nonce echo
                resp.extend_from_slice(&new_cid.to_be_bytes());
                resp.push(0x02); // U2FHID protocol version
                resp.push(0x01); // device version major
                resp.push(0x00); // minor
                resp.push(0x00); // build
                resp.push(CAP_WINK | CAP_CBOR);
                (build_response(cid, CMD_INIT, &resp), None)
            }
            CMD_PING => (build_response(cid, CMD_PING, payload), None),
            CMD_WINK => (build_response(cid, CMD_WINK, &[]), None),
            CMD_LOCK => (build_response(cid, CMD_LOCK, &[]), None),
            CMD_CANCEL => {
                // CTAPHID_CANCEL itself gets no response, but a pending CBOR
                // command on this channel must be answered with
                // CTAP2_ERR_KEEPALIVE_CANCEL, otherwise the host waits forever.
                if self.busy == Some(cid) || self.assembly.is_some() {
                    self.busy = None;
                    self.assembly = None;
                    (
                        build_response(cid, CMD_CBOR, &[0x2d]),
                        Some(CtapAction::CancelRelay),
                    )
                } else {
                    (vec![], None)
                }
            }
            CMD_CBOR => self.dispatch_cbor(cid, payload),
            CMD_MSG => (error_response(cid, ERR_INVALID_CMD), None),
            _ => (error_response(cid, ERR_INVALID_CMD), None),
        }
    }

    fn dispatch_cbor(&mut self, cid: u32, payload: &[u8]) -> (Vec<Vec<u8>>, Option<CtapAction>) {
        // One relayed operation at a time: a second CBOR command while one is
        // pending (on this or another channel) is rejected, never dropped.
        if self.busy.is_some() {
            return (error_response(cid, ERR_CHANNEL_BUSY), None);
        }
        let Some(&subcmd) = payload.first() else {
            return (build_response(cid, CMD_CBOR, &[0x11]), None); // INVALID_LENGTH
        };
        match subcmd {
            0x04 => {
                let resp = crate::ctap::getinfo_response(&self.aaguid);
                (build_response(cid, CMD_CBOR, &resp), None)
            }
            0x01 | 0x02 => {
                self.busy = Some(cid);
                (vec![], Some(CtapAction::Relay(payload.to_vec())))
            }
            _ => (build_response(cid, CMD_CBOR, &[0x01]), None), // INVALID_COMMAND
        }
    }

    /// Complete a relayed CTAP operation with the phone's response payload
    /// (status byte + CBOR), to be sent back over the given channel.
    pub fn complete_relay(&mut self, cid: u32, payload: &[u8]) -> Vec<Vec<u8>> {
        self.busy = None;
        build_response(cid, CMD_CBOR, payload)
    }

    pub fn cancel_relay(&mut self, cid: u32) -> Vec<Vec<u8>> {
        self.busy = None;
        // 0x2d = CTAP2_ERR_KEEPALIVE_CANCEL
        build_response(cid, CMD_CBOR, &[0x2d])
    }

    pub fn keepalive(&self, cid: u32) -> Vec<Vec<u8>> {
        // 0x02 = STATUS_UPNEEDED
        build_response(cid, CMD_KEEPALIVE, &[0x02])
    }

    pub fn busy_channel(&self) -> Option<u32> {
        self.busy
    }
}

pub fn build_response(cid: u32, cmd: u8, payload: &[u8]) -> Vec<Vec<u8>> {
    let mut reports = Vec::new();
    let mut first = vec![0u8; REPORT_SIZE];
    first[0..4].copy_from_slice(&cid.to_be_bytes());
    first[4] = cmd;
    first[5..7].copy_from_slice(&(payload.len() as u16).to_be_bytes());
    let first_take = payload.len().min(REPORT_SIZE - 7);
    first[7..7 + first_take].copy_from_slice(&payload[..first_take]);
    reports.push(first);

    let mut offset = first_take;
    let mut seq = 0u8;
    while offset < payload.len() {
        let mut cont = vec![0u8; REPORT_SIZE];
        cont[0..4].copy_from_slice(&cid.to_be_bytes());
        cont[4] = seq;
        seq += 1;
        let take = (payload.len() - offset).min(REPORT_SIZE - 5);
        cont[5..5 + take].copy_from_slice(&payload[offset..offset + take]);
        reports.push(cont);
        offset += take;
    }
    reports
}

fn error_response(cid: u32, code: u8) -> Vec<Vec<u8>> {
    build_response(cid, CMD_ERROR, &[code])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn init_frame(cid: u32, cmd: u8, payload: &[u8]) -> Vec<u8> {
        let mut r = vec![0u8; REPORT_SIZE];
        r[0..4].copy_from_slice(&cid.to_be_bytes());
        r[4] = cmd;
        r[5..7].copy_from_slice(&(payload.len() as u16).to_be_bytes());
        r[7..7 + payload.len()].copy_from_slice(payload);
        r
    }

    #[test]
    fn init_allocates_channel() {
        let mut t = Transport::new([0u8; 16]);
        let (resp, action) = t.handle_report(&init_frame(
            BROADCAST_CID,
            CMD_INIT,
            &[1, 2, 3, 4, 5, 6, 7, 8],
        ));
        assert!(action.is_none());
        assert_eq!(resp.len(), 1);
        assert_eq!(resp[0][4], CMD_INIT);
        assert_eq!(&resp[0][7..15], &[1, 2, 3, 4, 5, 6, 7, 8]);
        let new_cid = u32::from_be_bytes(resp[0][15..19].try_into().unwrap());
        assert!(t.channels.contains_key(&new_cid));
        assert_eq!(resp[0][19], 0x02); // protocol version
        assert_eq!(resp[0][23], CAP_WINK | CAP_CBOR);
    }

    #[test]
    fn ping_echo() {
        let mut t = Transport::new([0u8; 16]);
        let (resp, _) = t.handle_report(&init_frame(1, CMD_PING, &[9, 9, 9]));
        assert_eq!(resp.len(), 1);
        assert_eq!(&resp[0][7..10], &[9, 9, 9]);
    }

    #[test]
    fn getinfo_single_and_cont() {
        let mut t = Transport::new([7u8; 16]);
        let (resp, action) = t.handle_report(&init_frame(1, CMD_CBOR, &[0x04]));
        assert!(action.is_none());
        let total = u16::from_be_bytes([resp[0][5], resp[0][6]]) as usize;
        assert_eq!(resp[0][4], CMD_CBOR);
        assert_eq!(resp[0][7], 0x00); // CTAP2 success
        let mut assembled = resp[0][7..].to_vec();
        for r in &resp[1..] {
            assembled.extend_from_slice(&r[5..]);
        }
        let assembled = &assembled[..total];
        assert!(assembled.len() > 57 - 7 + 1); // spans into continuation
        assert!(assembled.contains(&0x66)); // "hybrid" string prefix present
    }

    #[test]
    fn msg_rejected() {
        let mut t = Transport::new([0u8; 16]);
        let (resp, _) = t.handle_report(&init_frame(1, CMD_MSG, &[0x03, 0, 0]));
        assert_eq!(resp[0][4], CMD_ERROR);
        assert_eq!(resp[0][7], ERR_INVALID_CMD);
    }

    #[test]
    fn multiframe_request() {
        let mut t = Transport::new([0u8; 16]);
        let payload: Vec<u8> = (0..200u32).map(|i| (i % 256) as u8).collect();
        let mut f = vec![0u8; REPORT_SIZE];
        f[0..4].copy_from_slice(&1u32.to_be_bytes());
        f[4] = CMD_PING;
        f[5..7].copy_from_slice(&200u16.to_be_bytes());
        f[7..].copy_from_slice(&payload[..57]);
        let (resp, _) = t.handle_report(&f);
        assert!(resp.is_empty());
        let mut offset = 57;
        let mut seq = 0u8;
        while offset < 200 {
            let mut c = vec![0u8; REPORT_SIZE];
            c[0..4].copy_from_slice(&1u32.to_be_bytes());
            c[4] = seq;
            seq += 1;
            let take = (200 - offset).min(59);
            c[5..5 + take].copy_from_slice(&payload[offset..offset + take]);
            let (resp, _) = t.handle_report(&c);
            offset += take;
            if offset >= 200 {
                assert_eq!(resp.len(), 4); // 200-byte echo: 1 init + 3 cont
                let total = u16::from_be_bytes([resp[0][5], resp[0][6]]) as usize;
                assert_eq!(total, 200);
            }
        }
    }

    #[test]
    fn cancel_answers_pending_cbor() {
        let mut t = Transport::new([0u8; 16]);
        let (_, action) = t.handle_report(&init_frame(1, CMD_CBOR, &[0x01, 0xa4]));
        assert_eq!(action, Some(CtapAction::Relay(vec![0x01, 0xa4])));

        let (resp, action) = t.handle_report(&init_frame(1, CMD_CANCEL, &[]));
        assert_eq!(resp.len(), 1);
        assert_eq!(resp[0][4], CMD_CBOR);
        assert_eq!(resp[0][7], 0x2d); // KEEPALIVE_CANCEL
        assert_eq!(action, Some(CtapAction::CancelRelay));
        assert_eq!(t.busy_channel(), None);

        // Cancel with nothing pending: no response.
        let (resp, action) = t.handle_report(&init_frame(1, CMD_CANCEL, &[]));
        assert!(resp.is_empty());
        assert!(action.is_none());
    }

    #[test]
    fn busy_channel_rejects_other_cid() {
        let mut t = Transport::new([0u8; 16]);
        let _ = t.handle_report(&init_frame(1, CMD_CBOR, &[0x01, 0xa4]));
        let (resp, _) = t.handle_report(&init_frame(2, CMD_CBOR, &[0x04]));
        assert_eq!(resp[0][4], CMD_ERROR);
        assert_eq!(resp[0][7], ERR_CHANNEL_BUSY);
    }

    #[test]
    fn busy_channel_rejects_same_cid() {
        let mut t = Transport::new([0u8; 16]);
        let _ = t.handle_report(&init_frame(1, CMD_CBOR, &[0x01, 0xa4]));
        let (resp, action) = t.handle_report(&init_frame(1, CMD_CBOR, &[0x02, 0xa4]));
        assert!(action.is_none());
        assert_eq!(resp[0][4], CMD_ERROR);
        assert_eq!(resp[0][7], ERR_CHANNEL_BUSY);
        // The pending relay is untouched and can still complete.
        assert_eq!(t.busy_channel(), Some(1));
    }

    #[test]
    fn makecredential_triggers_relay() {
        let mut t = Transport::new([0u8; 16]);
        let (resp, action) = t.handle_report(&init_frame(1, CMD_CBOR, &[0x01, 0xa4]));
        assert!(resp.is_empty());
        assert_eq!(action, Some(CtapAction::Relay(vec![0x01, 0xa4])));
        assert_eq!(t.busy_channel(), Some(1));
        let reports = t.complete_relay(1, &[0x00, 0xa1]);
        assert_eq!(reports[0][4], CMD_CBOR);
    }
}
