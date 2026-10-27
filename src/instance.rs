// SPDX-License-Identifier: GPL-3.0-or-later

//! Process-level single-instance guard for the daemon.
//!
//! The system-bus name alone is not enough: `--no-ui` never claims it, and
//! startup falls back to the terminal UI when the bus or the UI bridge is
//! unavailable, so a second daemon would still create another virtual HID
//! device and race the first one for the host's requests.  An advisory lock
//! on a file in the state directory makes "one daemon per state directory"
//! true regardless of UI mode, and it is taken before the identity key can be
//! created or the device registered.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const LOCK_FILE_NAME: &str = "instance.lock";

/// Held for the lifetime of the daemon; dropping it releases the lock.
#[derive(Debug)]
pub struct InstanceLock {
    path: PathBuf,
    _file: File,
}

impl InstanceLock {
    /// Path of the lock file, for logging.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Acquire the exclusive instance lock in `dir`, creating the directory when
/// it does not exist yet.
///
/// The lock is non-blocking: a second daemon gets a clear error instead of
/// waiting for the first one to exit.
pub fn acquire(dir: &Path) -> Result<InstanceLock> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating state directory {}", dir.display()))?;
    let path = dir.join(LOCK_FILE_NAME);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&path)
        .with_context(|| format!("opening instance lock {}", path.display()))?;
    // `flock` locks the open file description, so two daemons conflict even
    // when they share a process; `LOCK_NB` turns that conflict into an error
    // rather than a wait.  The kernel drops the lock when the file is closed,
    // including when the daemon is killed.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EWOULDBLOCK) {
            anyhow::bail!(
                "another ucabled is already running (instance lock {} is held)",
                path.display()
            );
        }
        return Err(err).with_context(|| format!("locking {}", path.display()));
    }
    Ok(InstanceLock { path, _file: file })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("ucabled-instance-{}-{name}", std::process::id()))
    }

    #[test]
    fn a_second_lock_is_refused_until_the_first_is_dropped() {
        let dir = test_dir("exclusive");
        let _ = std::fs::remove_dir_all(&dir);
        let first = acquire(&dir).expect("first lock");
        let err = acquire(&dir).expect_err("second lock must be refused");
        assert!(
            format!("{err:#}").contains("already running"),
            "unexpected error: {err:#}"
        );
        drop(first);
        acquire(&dir).expect("the lock is released when the guard is dropped");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn separate_state_directories_do_not_conflict() {
        let a = test_dir("a");
        let b = test_dir("b");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
        let _first = acquire(&a).expect("lock in the first directory");
        acquire(&b).expect("a different directory has its own lock");
        let _ = std::fs::remove_dir_all(&a);
        let _ = std::fs::remove_dir_all(&b);
    }
}
