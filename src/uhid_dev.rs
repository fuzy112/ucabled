// SPDX-License-Identifier: GPL-3.0-or-later

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;

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
            .open("/dev/uhid")?;
        set_nonblocking(&file)?;
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
        let name_len = name_bytes.len().min(127);
        u[..name_len].copy_from_slice(&name_bytes[..name_len]);

        let rd_size = report_descriptor.len() as u16;
        u[256..258].copy_from_slice(&rd_size.to_le_bytes());
        // bus = BUS_USB (0x03)
        u[258..260].copy_from_slice(&3u16.to_le_bytes());
        // vendor/product: pid.codes community space
        u[260..264].copy_from_slice(&0x1209u32.to_le_bytes());
        u[264..268].copy_from_slice(&0x5042u32.to_le_bytes());
        u[268..272].copy_from_slice(&1u32.to_le_bytes());
        // rd_data starts at offset 276 within create2 payload
        u[276..276 + report_descriptor.len()].copy_from_slice(report_descriptor);

        write_all_blocking(self.fd.get_ref(), &event)
    }

    pub async fn next_event(&self) -> io::Result<UhidEvent> {
        let mut buf = [0u8; UHID_EVENT_SIZE];
        loop {
            let mut guard = self.fd.readable().await?;
            let n = match guard.try_io(|inner| {
                let raw = unsafe {
                    libc::read(
                        inner.as_raw_fd(),
                        buf.as_mut_ptr() as *mut libc::c_void,
                        buf.len(),
                    )
                };
                if raw < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(raw as usize)
                }
            }) {
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
                    // uhid_output_req: `data[4096]` followed by a little-endian
                    // u16 size. Read both bytes; a single byte would truncate
                    // reports larger than 255.
                    let size = u16::from_le_bytes([buf[4 + 4096], buf[4 + 4096 + 1]]) as usize;
                    let mut data = buf[4..4 + size.min(4096)].to_vec();
                    // hidraw writes carry a leading report-ID byte (0 when the
                    // descriptor defines no report IDs); strip it.
                    if data.len() == 65 && data[0] == 0 {
                        data.remove(0);
                    }
                    UhidEvent::Output(data)
                }
                other => UhidEvent::Other(other),
            });
        }
    }

    pub async fn write_input(&self, report: &[u8]) -> io::Result<()> {
        // The kernel consumes the first byte as the report ID even for
        // descriptors without numbered reports, so prepend 0.
        let mut event = [0u8; UHID_EVENT_SIZE];
        event[..4].copy_from_slice(&UHID_INPUT2.to_le_bytes());
        event[4] = (report.len() + 1) as u8;
        event[5] = 0;
        event[6..6 + report.len()].copy_from_slice(report);

        let mut guard = self.fd.writable().await?;
        match guard.try_io(|inner| write_all_blocking(inner.get_ref(), &event)) {
            Ok(result) => result,
            Err(_would_block) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "uhid write would block",
            )),
        }
    }
}

fn set_nonblocking(file: &File) -> io::Result<()> {
    let fd = file.as_raw_fd();
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        if flags < 0 {
            return Err(io::Error::last_os_error());
        }
        if libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn write_all_blocking(file: &File, mut data: &[u8]) -> io::Result<()> {
    while !data.is_empty() {
        let n = unsafe {
            libc::write(
                file.as_raw_fd(),
                data.as_ptr() as *const libc::c_void,
                data.len(),
            )
        };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        data = &data[n as usize..];
    }
    Ok(())
}

const _ASSERT_UNION_FITS: () = assert!(UHID_EVENT_SIZE >= 4376);
