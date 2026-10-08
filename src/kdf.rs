// SPDX-License-Identifier: GPL-3.0-or-later

use hkdf::Hkdf;
use sha2::Sha256;

#[derive(Clone, Copy)]
#[repr(u32)]
pub enum Purpose {
    EidKey = 1,
    TunnelId = 2,
    Psk = 3,
    #[allow(dead_code)]
    PairedSecret = 4,
    #[allow(dead_code)]
    IdentityKeySeed = 5,
    #[allow(dead_code)]
    PerContactIdSecret = 6,
}

/// Derive `out` bytes from `secret` for `purpose` (HKDF-SHA256).
///
/// # Panics
///
/// Panics when `out` exceeds the HKDF-SHA256 limit of 8160 bytes
/// (255 × 32). All callers derive fixed-size keys far below that.
pub fn derive(secret: &[u8], salt: &[u8], purpose: Purpose, out: &mut [u8]) {
    let info = (purpose as u32).to_le_bytes();
    let hk = Hkdf::<Sha256>::new(Some(salt), secret);
    hk.expand(&info, out)
        .expect("HKDF-SHA256 output is at most 8160 bytes");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_and_purpose_separated() {
        let mut a = [0u8; 32];
        let mut b = [0u8; 32];
        derive(b"secret", b"salt", Purpose::Psk, &mut a);
        derive(b"secret", b"salt", Purpose::Psk, &mut b);
        assert_eq!(a, b);

        let mut other_purpose = [0u8; 32];
        derive(b"secret", b"salt", Purpose::TunnelId, &mut other_purpose);
        assert_ne!(a, other_purpose);

        let mut other_salt = [0u8; 32];
        derive(b"secret", b"other", Purpose::Psk, &mut other_salt);
        assert_ne!(a, other_salt);
    }

    #[test]
    fn empty_and_long_outputs() {
        let mut empty = [0u8; 0];
        derive(b"secret", b"", Purpose::Psk, &mut empty);
        let mut long = [0u8; 96];
        derive(b"secret", b"", Purpose::Psk, &mut long);
        // Prefix-stability is not guaranteed by HKDF, but output must be
        // filled and not all zero for this input.
        assert!(long.iter().any(|&b| b != 0));
    }
}
