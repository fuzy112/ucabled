// SPDX-License-Identifier: GPL-3.0-or-later

//! Persistent desktop identity key for caBLE transactions.
//!
//! Chromium keeps a long-term identity key per device; ucabled originally
//! generated a fresh random key for every QR transaction. Using a persistent
//! identity aligns with Chromium and is a prerequisite for caBLE linking
//! (see `docs/linking.md`): the phone's linking signature is verified with
//! `ECDH(identity, phone public key)`, and a linked phone may later recognise
//! the desktop by this key.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use p256::SecretKey;
use rand::rngs::OsRng;

pub const IDENTITY_FILE_NAME: &str = "identity.key";

/// Fallback state directory when systemd's `$STATE_DIRECTORY` is unset
/// (dist installs, manual runs).
pub const DEFAULT_STATE_DIR: &str = "/var/lib/ucabled";

/// Directory holding the identity key: `$STATE_DIRECTORY` when the daemon
/// runs under systemd with `StateDirectory=ucabled`, otherwise the dist
/// default.
pub fn state_dir() -> PathBuf {
    match std::env::var_os("STATE_DIRECTORY") {
        // systemd may pass a colon-separated list; we declare exactly one.
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(DEFAULT_STATE_DIR),
    }
}

/// Load the persistent identity key from `dir`, creating and storing a fresh
/// random one when none exists. A corrupt or unreadable existing file is an
/// error, not a silent regeneration: losing the key is visible, accidentally
/// replacing it must not be.
pub fn load_or_create(dir: &Path) -> Result<SecretKey> {
    let path = dir.join(IDENTITY_FILE_NAME);
    match fs::read(&path) {
        Ok(bytes) => SecretKey::from_slice(&bytes)
            .with_context(|| format!("invalid identity key in {}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let key = SecretKey::random(&mut OsRng);
            fs::create_dir_all(dir)
                .with_context(|| format!("creating state directory {}", dir.display()))?;
            let bytes = key.to_bytes();
            write_private(&path, &bytes)
                .with_context(|| format!("writing identity key to {}", path.display()))?;
            tracing::info!(path = %path.display(), "generated a new persistent identity key");
            Ok(key)
        }
        Err(e) => Err(e).with_context(|| format!("reading identity key from {}", path.display())),
    }
}

/// Write the key atomically (rename into place) with owner-only permissions;
/// the daemon's UMask is 0077, but do not rely on it.
#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let tmp = path.with_extension("tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    fs::write(path, bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_then_reloads_the_same_key() {
        let dir = std::env::temp_dir().join(format!("ucabled-identity-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);

        let first = load_or_create(&dir).unwrap();
        let second = load_or_create(&dir).unwrap();
        assert_eq!(first.to_bytes(), second.to_bytes());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(dir.join(IDENTITY_FILE_NAME))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_a_corrupt_key_file() {
        let dir =
            std::env::temp_dir().join(format!("ucabled-identity-corrupt-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(IDENTITY_FILE_NAME), b"not a key").unwrap();

        assert!(load_or_create(&dir).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }
}
