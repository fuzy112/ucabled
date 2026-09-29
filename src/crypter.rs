use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::Aes256Gcm;

const PADDING_GRANULARITY: usize = 32;
const MAX_SEQUENCE: u32 = (1 << 24) - 1;

pub struct Crypter {
    read_key: [u8; 32],
    write_key: [u8; 32],
    read_seq: u32,
    write_seq: u32,
}

impl Crypter {
    pub fn new(read_key: [u8; 32], write_key: [u8; 32]) -> Self {
        Self {
            read_key,
            write_key,
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

    pub fn encrypt(&mut self, message: &[u8]) -> Option<Vec<u8>> {
        let padded_size =
            (message.len() + 1 + PADDING_GRANULARITY - 1) & !(PADDING_GRANULARITY - 1);
        let num_zeros = padded_size - message.len() - 1;
        let mut padded = vec![0u8; padded_size];
        padded[..message.len()].copy_from_slice(message);
        padded[padded_size - 1] = num_zeros as u8;

        let nonce = Self::nonce(self.write_seq)?;
        self.write_seq += 1;
        let cipher = Aes256Gcm::new((&self.write_key).into());
        cipher.encrypt((&nonce).into(), padded.as_ref()).ok()
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        let nonce = Self::nonce(self.read_seq)?;
        let cipher = Aes256Gcm::new((&self.read_key).into());
        let plaintext = cipher.decrypt((&nonce).into(), ciphertext).ok()?;
        self.read_seq += 1;

        if plaintext.is_empty() {
            return None;
        }
        let padding_len = *plaintext.last().unwrap() as usize;
        if padding_len + 1 > plaintext.len() {
            return None;
        }
        Some(plaintext[..plaintext.len() - padding_len - 1].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (a, b) where a writes with key 1 and b reads with key 1, and vice versa.
    fn pair() -> (Crypter, Crypter) {
        (Crypter::new([2u8; 32], [1u8; 32]), Crypter::new([1u8; 32], [2u8; 32]))
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
    fn tampering_fails() {
        let (mut a, mut b) = pair();
        let mut ct = a.encrypt(b"payload").unwrap();
        ct[0] ^= 1;
        assert!(b.decrypt(&ct).is_none());
    }
}
