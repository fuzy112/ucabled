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
