// SPDX-License-Identifier: GPL-3.0-or-later

//! Minimal systemd readiness and watchdog notifications.
//!
//! systemd hands the daemon a datagram socket path in `$NOTIFY_SOCKET` and,
//! when the unit sets `WatchdogSec=`, the expected interval in
//! `$WATCHDOG_USEC`.  The protocol is one `sendto` per state change, so it is
//! implemented directly instead of pulling in an `sd_notify` dependency.  All
//! functions are no-ops when systemd (or a watchdog) is not in play.

use std::ffi::OsStr;
use std::io;
use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::time::Duration;

/// Send one state string to `address`.
fn send_to(address: &OsStr, state: &str) -> io::Result<()> {
    let socket = UnixDatagram::unbound()?;
    match address.as_bytes().strip_prefix(b"@") {
        // Abstract namespace: systemd encodes it with a leading '@' where the
        // kernel wants a leading NUL.
        Some(name) => socket.connect_addr(&SocketAddr::from_abstract_name(name)?)?,
        None => socket.connect(address)?,
    }
    socket.send(state.as_bytes())?;
    Ok(())
}

fn notify(state: &str) {
    let Some(address) = std::env::var_os("NOTIFY_SOCKET") else {
        return;
    };
    if address.is_empty() {
        return;
    }
    if let Err(e) = send_to(&address, state) {
        tracing::debug!("systemd notify ({state}) failed: {e}");
    }
}

/// Tell systemd the daemon is ready to serve (`READY=1`).
pub fn ready() {
    notify("READY=1");
}

/// A watchdog keepalive (`WATCHDOG=1`).
pub fn watchdog() {
    notify("WATCHDOG=1");
}

/// `WATCHDOG_USEC / 2`, the interval systemd asks us to ping at.
fn interval_from(usec: Option<&str>) -> Option<Duration> {
    let usec: u64 = usec?.trim().parse().ok()?;
    Some(Duration::from_micros(usec / 2))
}

/// The watchdog keepalive interval, or `None` when no watchdog is configured.
pub fn watchdog_interval() -> Option<Duration> {
    // systemd sets WATCHDOG_PID for the process it is watching; ignore a
    // stale value that was inherited by a child.
    if let Ok(pid) = std::env::var("WATCHDOG_PID") {
        if pid.parse::<u32>().ok() != Some(std::process::id()) {
            return None;
        }
    }
    interval_from(std::env::var("WATCHDOG_USEC").ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datagram_reaches_a_pathname_socket() {
        let path = std::env::temp_dir().join(format!("ucabled-notify-{}", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let receiver = UnixDatagram::bind(&path).unwrap();

        send_to(path.as_os_str(), "READY=1").unwrap();

        let mut buf = [0u8; 32];
        let n = receiver.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"READY=1");

        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn watchdog_interval_is_half_the_announced_period() {
        assert_eq!(interval_from(None), None);
        assert_eq!(interval_from(Some("not a number")), None);
        assert_eq!(
            interval_from(Some("30000000")),
            Some(Duration::from_secs(15))
        );
        // systemd may pad the value.
        assert_eq!(
            interval_from(Some(" 60000000\n")),
            Some(Duration::from_secs(30))
        );
    }
}
