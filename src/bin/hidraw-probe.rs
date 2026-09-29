use anyhow::{bail, Context, Result};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

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
    let mut frame = vec![0u8; 65];
    frame[0] = 0; // report ID
    frame[1..5].copy_from_slice(&0xffff_ffffu32.to_be_bytes());
    frame[5] = 0x86; // INIT
    frame[6..8].copy_from_slice(&8u16.to_be_bytes());
    frame[8..16].copy_from_slice(&nonce);

    f.write_all(&frame)?;
    println!("wrote INIT, waiting for response...");

    let fd = f.as_raw_fd();
    let mut pfd = libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    };
    let rc = unsafe { libc::poll(&mut pfd, 1, 3000) };
    if rc <= 0 {
        bail!("timed out waiting for INIT response (rc={rc})");
    }

    let mut buf = [0u8; 64];
    let n = f.read(&mut buf)?;
    println!("read {n} bytes: {}", hex::encode(&buf[..n]));
    if n >= 15 && &buf[7..15] == nonce.as_slice() {
        println!("INIT roundtrip OK: nonce echoed");
    } else {
        println!("unexpected response content");
    }
    Ok(())
}
