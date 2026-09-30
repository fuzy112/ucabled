// SPDX-License-Identifier: GPL-3.0-or-later

use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

use ucabled::ctaphid::{BROADCAST_CID, CMD_INIT, REPORT_SIZE};

/// hidraw prepends a report-ID byte to the 64-byte report when writing.
const WRITE_FRAME_LEN: usize = REPORT_SIZE + 1;
/// Offsets within the report write frame, report-ID byte included.
const REQUEST_REPORT_ID_OFFSET: usize = 0;
const REQUEST_CID_OFFSET: usize = 1;
const REQUEST_CMD_OFFSET: usize = 5;
const REQUEST_LEN_OFFSET: usize = 6;
const REQUEST_DATA_OFFSET: usize = 8;
/// Offset of the data in the (unprefixed) 64-byte response report.
const RESPONSE_DATA_OFFSET: usize = 7;
/// How long to wait for the INIT response.
const POLL_TIMEOUT_MS: i32 = 3000;

fn find_device() -> Result<String> {
    for entry in fs::read_dir("/sys/class/hidraw")? {
        let entry = entry?;
        let uevent = entry.path().join("device/uevent");
        let Ok(contents) = fs::read_to_string(&uevent) else {
            continue;
        };
        if contents.contains("Phone Passkey Bridge") {
            let name = entry.file_name().to_string_lossy().to_string();
            return Ok(format!("/dev/{name}"));
        }
    }
    bail!("Phone Passkey Bridge hidraw device not found (is ucabled running?)")
}

fn main() -> Result<()> {
    let path = find_device()?;
    println!("using {path}");
    let mut f = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .context("open hidraw failed")?;

    let nonce = [0xde, 0xad, 0xbe, 0xef, 0x01, 0x02, 0x03, 0x04];
    let mut frame = vec![0u8; WRITE_FRAME_LEN];
    frame[REQUEST_REPORT_ID_OFFSET] = 0;
    frame[REQUEST_CID_OFFSET..REQUEST_CID_OFFSET + 4].copy_from_slice(&BROADCAST_CID.to_be_bytes());
    frame[REQUEST_CMD_OFFSET] = CMD_INIT;
    frame[REQUEST_LEN_OFFSET..REQUEST_LEN_OFFSET + 2]
        .copy_from_slice(&(nonce.len() as u16).to_be_bytes());
    frame[REQUEST_DATA_OFFSET..REQUEST_DATA_OFFSET + nonce.len()].copy_from_slice(&nonce);

    f.write_all(&frame)?;
    println!("wrote INIT, waiting for response...");

    let fd = f.as_raw_fd();
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let rc = unsafe { libc::poll(&mut pfd, 1, POLL_TIMEOUT_MS) };
    if rc <= 0 {
        bail!("timed out waiting for INIT response (rc={rc})");
    }

    let mut buf = [0u8; REPORT_SIZE];
    let n = f.read(&mut buf)?;
    println!("read {n} bytes: {}", hex::encode(&buf[..n]));
    let nonce_echo_end = RESPONSE_DATA_OFFSET + nonce.len();
    if n >= nonce_echo_end && &buf[RESPONSE_DATA_OFFSET..nonce_echo_end] == nonce.as_slice() {
        println!("INIT roundtrip OK: nonce echoed");
    } else {
        println!("unexpected response content");
    }
    Ok(())
}
