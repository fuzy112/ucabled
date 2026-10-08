// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;

use tokio::io::unix::AsyncFd;

// uhid event types (linux/uhid.h).
const UHID_START: u32 = 2;
const UHID_STOP: u32 = 3;
const UHID_OPEN: u32 = 4;
const UHID_CLOSE: u32 = 5;
const UHID_OUTPUT: u32 = 6;
const UHID_CREATE2: u32 = 11;
const UHID_INPUT2: u32 = 12;

const UHID_EVENT_SIZE: usize = 4380;

// Offsets within the UHID_CREATE2 payload (linux/uhid.h's struct
// uhid_create2_req), relative to the start of the event payload.
const CREATE2_NAME_OFFSET: usize = 0;
const CREATE2_NAME_MAX: usize = 127;
const CREATE2_RD_SIZE_OFFSET: usize = 256;
const CREATE2_BUS_OFFSET: usize = 258;
const CREATE2_VENDOR_OFFSET: usize = 260;
const CREATE2_PRODUCT_OFFSET: usize = 264;
const CREATE2_VERSION_OFFSET: usize = 268;
const CREATE2_RD_DATA_OFFSET: usize = 276;
/// Bus type reported for the virtual device (linux/input.h BUS_USB).
const BUS_USB: u16 = 0x03;
/// pid.codes community vendor/product IDs for this project.
const VENDOR_ID: u32 = 0x1209;
const PRODUCT_ID: u32 = 0x5042;
const DEVICE_VERSION: u32 = 1;

/// UHID_OUTPUT carries the report in a fixed 4096-byte buffer.
const OUTPUT_DATA_SIZE: usize = 4096;
/// hidraw reports include a leading report-ID byte that UHID does not.
const REPORT_ID_LEN: usize = 1;
/// Data bytes in the FIDO HID report descriptor below.
const FIDO_REPORT_SIZE: usize = 64;

// Offsets within a UHID_INPUT2 event: a little-endian u16 length, then data.
const INPUT2_SIZE_OFFSET: usize = 4;
const INPUT2_DATA_OFFSET: usize = 6;

/// FIDO U2FHID report descriptor (U2FHID spec 4.3): usage page 0xF1D0,
/// 64-byte input and output reports.
pub const FIDO_REPORT_DESCRIPTOR: [u8; 34] = [
    0x06, 0xd0, 0xf1, 0x09, 0x01, 0xa1, 0x01, 0x09, 0x20, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08,
    0x95, 0x40, 0x81, 0x02, 0x09, 0x21, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x40, 0x91,
    0x02, 0xc0,
];

#[derive(Debug)]
pub enum UhidEvent {
    Start,
    Stop,
    Open,
    Close,
    Output(Vec<u8>),
    Other(u32),
}

pub struct UhidDevice {
    fd: AsyncFd<File>,
}

impl UhidDevice {
    pub fn create(name: &str, report_descriptor: &[u8]) -> io::Result<Self> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open("/dev/uhid")?;
        let dev = Self {
            fd: AsyncFd::new(file)?,
        };
        dev.write_create(name, report_descriptor)?;
        Ok(dev)
    }

    fn write_create(&self, name: &str, report_descriptor: &[u8]) -> io::Result<()> {
        let mut event = [0u8; UHID_EVENT_SIZE];
        event[..4].copy_from_slice(&UHID_CREATE2.to_le_bytes());

        let u = &mut event[4..];
        let name_bytes = name.as_bytes();
        let name_len = name_bytes.len().min(CREATE2_NAME_MAX);
        u[CREATE2_NAME_OFFSET..CREATE2_NAME_OFFSET + name_len]
            .copy_from_slice(&name_bytes[..name_len]);

        let rd_size = report_descriptor.len() as u16;
        u[CREATE2_RD_SIZE_OFFSET..CREATE2_RD_SIZE_OFFSET + 2]
            .copy_from_slice(&rd_size.to_le_bytes());
        u[CREATE2_BUS_OFFSET..CREATE2_BUS_OFFSET + 2].copy_from_slice(&BUS_USB.to_le_bytes());
        u[CREATE2_VENDOR_OFFSET..CREATE2_VENDOR_OFFSET + 4]
            .copy_from_slice(&VENDOR_ID.to_le_bytes());
        u[CREATE2_PRODUCT_OFFSET..CREATE2_PRODUCT_OFFSET + 4]
            .copy_from_slice(&PRODUCT_ID.to_le_bytes());
        u[CREATE2_VERSION_OFFSET..CREATE2_VERSION_OFFSET + 4]
            .copy_from_slice(&DEVICE_VERSION.to_le_bytes());
        u[CREATE2_RD_DATA_OFFSET..CREATE2_RD_DATA_OFFSET + report_descriptor.len()]
            .copy_from_slice(report_descriptor);

        write_all(self.fd.get_ref(), &event)
    }

    pub async fn next_event(&self) -> io::Result<UhidEvent> {
        let mut buf = [0u8; UHID_EVENT_SIZE];
        loop {
            let mut guard = self.fd.readable().await?;
            // The fd is O_NONBLOCK, so a WouldBlock error tells try_io to
            // keep waiting rather than fail the read.
            let n = match guard.try_io(|inner| inner.get_ref().read(&mut buf)) {
                Ok(result) => result?,
                Err(_would_block) => continue,
            };
            if n < 8 {
                continue;
            }
            let ev_type = u32::from_le_bytes(buf[..4].try_into().unwrap());
            return Ok(match ev_type {
                UHID_START => UhidEvent::Start,
                UHID_STOP => UhidEvent::Stop,
                UHID_OPEN => UhidEvent::Open,
                UHID_CLOSE => UhidEvent::Close,
                UHID_OUTPUT => {
                    // uhid_output_req: `data[OUTPUT_DATA_SIZE]` followed by a
                    // little-endian u16 size. Read both bytes; a single byte
                    // would truncate reports larger than 255.
                    let size = u16::from_le_bytes([
                        buf[4 + OUTPUT_DATA_SIZE],
                        buf[4 + OUTPUT_DATA_SIZE + 1],
                    ]) as usize;
                    let mut data = buf[4..4 + size.min(OUTPUT_DATA_SIZE)].to_vec();
                    // hidraw writes carry a leading report-ID byte (0 when the
                    // descriptor defines no report IDs); strip it.
                    if data.len() == FIDO_REPORT_SIZE + REPORT_ID_LEN && data[0] == 0 {
                        data.remove(0);
                    }
                    UhidEvent::Output(data)
                }
                other => UhidEvent::Other(other),
            });
        }
    }

    pub async fn write_input(&self, report: &[u8]) -> io::Result<()> {
        // `uhid_input2_req`: a little-endian u16 size followed by the report
        // data. The kernel consumes the first data byte as the report ID even
        // for descriptors without numbered reports, so it is counted here.
        let mut event = [0u8; UHID_EVENT_SIZE];
        event[..4].copy_from_slice(&UHID_INPUT2.to_le_bytes());
        let size = (report.len() + REPORT_ID_LEN) as u16;
        event[INPUT2_SIZE_OFFSET..INPUT2_SIZE_OFFSET + 2].copy_from_slice(&size.to_le_bytes());
        event[INPUT2_DATA_OFFSET..INPUT2_DATA_OFFSET + report.len()].copy_from_slice(report);

        let mut guard = self.fd.writable().await?;
        match guard.try_io(|inner| write_all(inner.get_ref(), &event)) {
            Ok(result) => result,
            Err(_would_block) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "uhid write would block",
            )),
        }
    }
}

/// Write the whole event. WouldBlock surfaces as an error: callers either
/// fail fast (create) or report it (input reports), matching the previous
/// raw-write behaviour.
fn write_all(mut file: &File, data: &[u8]) -> io::Result<()> {
    file.write_all(data)
}

const _ASSERT_UNION_FITS: () = assert!(UHID_EVENT_SIZE >= 4376);
