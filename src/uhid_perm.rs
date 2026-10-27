// SPDX-License-Identifier: GPL-3.0-or-later

//! Detection of a too-permissive `/dev/uhid`.
//!
//! Opening `/dev/uhid` lets a process create arbitrary virtual HID devices,
//! including a keyboard, so only the `ucabled` service account should have
//! write access.  The udev rule grants that account rw via an ACL; this module
//! deliberately does *not* tighten the node, it only reports a loose state so
//! the daemon can warn instead of silently rewriting permissions it did not
//! set.
//!
//! The subtle case is a stale `uaccess` tag: logind then re-grants the active
//! seat user on every seat change, which leaves the mode bits looking normal
//! (`0600 root:root`, the devtmpfs default) while an ACL entry grants the
//! human access.  The tag is what udev persists and what drives that
//! mechanism, so we read it back from `/run/udev/data`.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Return a human-readable reason when `path` is writable by a principal other
/// than root/the daemon's account, `Ok(None)` when it looks fine, and an error
/// when the node cannot be inspected at all (which is *not* the same as
/// tight: a transient stat failure must not be reported as a security
/// improvement).
pub fn loose_reason(path: &Path) -> std::io::Result<Option<String>> {
    let md = fs::metadata(path)?;
    Ok(evaluate(md.mode(), &udev_tags(md.rdev())))
}

/// Tags udev persisted for the device, e.g. `uaccess`, read from
/// `/run/udev/data/c<major>:<minor>`.  An inaccessible or absent file simply
/// yields no tags (the check is best-effort).
fn udev_tags(rdev: u64) -> Vec<String> {
    let data = fs::read_to_string(format!(
        "/run/udev/data/c{}:{}",
        libc::major(rdev),
        libc::minor(rdev)
    ));
    let Ok(data) = data else {
        return Vec::new();
    };
    data.lines()
        .filter_map(|line| line.strip_prefix("G:"))
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string)
        .collect()
}

/// The decision, factored out for tests.
///
/// The kernel creates the node `0600 root:root`; group/other bits only ever
/// come from an admin's udev rule (e.g. `0660 root:somegroup`), which grants
/// nothing to the seat user, so group write is not flagged.  What matters is
/// world write and the `uaccess`/`xaccess-` tags that make logind hand the
/// active seat user an ACL.
fn evaluate(mode: u32, tags: &[String]) -> Option<String> {
    let mut reasons = Vec::new();
    if mode & 0o002 != 0 {
        reasons.push("world-writable".to_string());
    }
    for tag in tags {
        if tag == "uaccess" || tag.starts_with("xaccess") {
            reasons.push(format!(
                "udev tag `{tag}` grants the active seat user access"
            ));
        }
    }
    if reasons.is_empty() {
        None
    } else {
        Some(reasons.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_node_is_fine() {
        assert_eq!(evaluate(0o660, &[]), None);
        assert_eq!(
            evaluate(0o600, &["seat0".into(), "power-switch".into()]),
            None
        );
    }

    #[test]
    fn world_writable_is_reported() {
        let r = evaluate(0o666, &[]).unwrap();
        assert!(r.contains("world-writable"), "{r}");
    }

    #[test]
    fn uninspectable_node_is_an_error_not_tight() {
        let path =
            std::env::temp_dir().join(format!("ucabled-missing-uhid-{}", std::process::id()));
        let err = loose_reason(&path).expect_err("a missing node is not a tight one");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
    }

    #[test]
    fn uaccess_tag_is_reported() {
        let r = evaluate(0o660, &["seat".into(), "uaccess".into()]).unwrap();
        assert!(r.contains("uaccess"), "{r}");
        let r = evaluate(0o660, &["xaccess-ucabled".into()]).unwrap();
        assert!(r.contains("xaccess-ucabled"), "{r}");
    }
}
