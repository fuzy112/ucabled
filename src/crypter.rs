// SPDX-License-Identifier: GPL-3.0-or-later

use aes_gcm::Aes256Gcm;
use aes_gcm::aead::{Aead, KeyInit};
use zeroize::Zeroize;

const PADDING_GRANULARITY: usize = 32;
/// AES-GCM authentication tag appended to every ciphertext.
const TAG_SIZE: usize = 16;
const MAX_SEQUENCE: u32 = (1 << 24) - 1;

/// Plaintext size after zero padding to the granularity: one length byte is
/// always added, so even an empty message occupies one whole block.
fn padded_size(message_len: usize) -> usize {
    (message_len + 1 + PADDING_GRANULARITY - 1) & !(PADDING_GRANULARITY - 1)
}

/// The ciphers wipe their expanded AES round keys on drop (the `zeroize`
/// feature of the `aes` crate), so the raw keys must not be stored: they
/// are wiped right after the ciphers are built.  The GHASH `H` element
/// inside `Aes256Gcm` is not wipeable (polyval has no zeroize support),
/// which is acceptable: recovering the AES key from H = E_K(0) is a
/// known-plaintext attack on AES itself.
pub struct Crypter {
    read_cipher: Aes256Gcm,
    write_cipher: Aes256Gcm,
    read_seq: u32,
    write_seq: u32,
}

impl Crypter {
    pub fn new(mut read_key: [u8; 32], mut write_key: [u8; 32]) -> Self {
        let read_cipher = Aes256Gcm::new((&read_key).into());
        let write_cipher = Aes256Gcm::new((&write_key).into());
        read_key.zeroize();
        write_key.zeroize();
        Self {
            read_cipher,
            write_cipher,
            read_seq: 0,
            write_seq: 0,
        }
    }

    /// 96-bit AES-GCM nonce: the 32-bit big-endian counter goes in the *last*
    /// four bytes. This matches caBLE v2 message encryption and differs from
    /// the Noise handshake, which puts the counter first (see `noise.rs`). Do
    /// not change it without a test vector.
    fn nonce(seq: u32) -> Option<[u8; 12]> {
        if seq > MAX_SEQUENCE {
            return None;
        }
        let mut nonce = [0u8; 12];
        nonce[8..].copy_from_slice(&seq.to_be_bytes());
        Some(nonce)
    }

    /// The ciphertext length [`Crypter::encrypt`] produces for a plaintext of
    /// `message_len` bytes.  Used to check a message against the L2CAP send
    /// MTU before committing to that channel.
    pub fn encrypted_size(message_len: usize) -> usize {
        padded_size(message_len) + TAG_SIZE
    }

    pub fn encrypt(&mut self, message: &[u8]) -> Option<Vec<u8>> {
        let padded_size = padded_size(message.len());
        let num_zeros = padded_size - message.len() - 1;
        let mut padded = vec![0u8; padded_size];
        padded[..message.len()].copy_from_slice(message);
        padded[padded_size - 1] = num_zeros as u8;

        let nonce = Self::nonce(self.write_seq)?;
        self.write_seq += 1;
        self.write_cipher
            .encrypt((&nonce).into(), padded.as_ref())
            .ok()
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        let nonce = Self::nonce(self.read_seq)?;
        let mut plaintext = self.read_cipher.decrypt((&nonce).into(), ciphertext).ok()?;
        self.read_seq += 1;

        if plaintext.is_empty() {
            return None;
        }
        let padding_len = *plaintext.last().unwrap() as usize;
        if padding_len + 1 > plaintext.len() {
            return None;
        }
        // Strip the padding in place instead of copying the message out.
        plaintext.truncate(plaintext.len() - padding_len - 1);
        Some(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (a, b) where a writes with key 1 and b reads with key 1, and vice versa.
    fn pair() -> (Crypter, Crypter) {
        (
            Crypter::new([2u8; 32], [1u8; 32]),
            Crypter::new([1u8; 32], [2u8; 32]),
        )
    }

    #[test]
    fn roundtrip_at_padding_boundaries() {
        let (mut a, mut b) = pair();
        for len in [0usize, 1, 30, 31, 32, 33, 63, 64, 65] {
            let msg: Vec<u8> = (0..len).map(|i| i as u8).collect();
            let ct = a.encrypt(&msg).unwrap();
            assert_eq!((ct.len() - 16) % PADDING_GRANULARITY, 0, "len {len}");
            assert_eq!(b.decrypt(&ct).unwrap(), msg, "len {len}");
        }
    }

    #[test]
    fn sequence_number_must_match() {
        let (mut a, mut b) = pair();
        let _first = a.encrypt(b"one").unwrap();
        let second = a.encrypt(b"two").unwrap();
        // A message encrypted with sequence 1 does not decrypt under sequence 0.
        assert!(b.decrypt(&second).is_none());
    }

    #[test]
    fn nonce_counter_is_in_the_last_four_bytes() {
        assert_eq!(
            Crypter::nonce(0).unwrap(),
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
        );
        assert_eq!(
            Crypter::nonce(1).unwrap(),
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
        );
    }

    #[test]
    fn encrypted_size_matches_the_ciphertext() {
        let (mut a, _b) = pair();
        for len in [0usize, 1, 30, 31, 32, 33, 63, 64, 65, 7609] {
            let msg = vec![0u8; len];
            assert_eq!(
                a.encrypt(&msg).unwrap().len(),
                Crypter::encrypted_size(len),
                "len {len}"
            );
        }
    }

    #[test]
    fn tampering_fails() {
        let (mut a, mut b) = pair();
        let mut ct = a.encrypt(b"payload").unwrap();
        ct[0] ^= 1;
        assert!(b.decrypt(&ct).is_none());
    }
}
