use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::Aes256Gcm;
use sha2::{Digest, Sha256};

pub const KN_PSK0_PROTOCOL_NAME: &[u8] = b"Noise_KNpsk0_P256_AESGCM_SHA256";
pub const NK_PSK0_PROTOCOL_NAME: &[u8] = b"Noise_NKpsk0_P256_AESGCM_SHA256";

fn hkdf2(ck: &[u8; 32], ikm: &[u8]) -> ([u8; 32], [u8; 32]) {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(ck), ikm);
    let mut out = [0u8; 64];
    hk.expand(&[], &mut out).expect("HKDF expand failed");
    let (a, b) = out.split_at(32);
    (a.try_into().unwrap(), b.try_into().unwrap())
}

fn hkdf3(ck: &[u8; 32], ikm: &[u8]) -> ([u8; 32], [u8; 32], [u8; 32]) {
    let hk = hkdf::Hkdf::<Sha256>::new(Some(ck), ikm);
    let mut out = [0u8; 96];
    hk.expand(&[], &mut out).expect("HKDF expand failed");
    let a = out[0..32].try_into().unwrap();
    let b = out[32..64].try_into().unwrap();
    let c = out[64..96].try_into().unwrap();
    (a, b, c)
}

pub struct Noise {
    ck: [u8; 32],
    h: [u8; 32],
    key: [u8; 32],
    nonce: u32,
}

impl Noise {
    pub fn new(protocol_name: &[u8]) -> Self {
        assert!(protocol_name.len() <= 32);
        let mut ck = [0u8; 32];
        ck[..protocol_name.len()].copy_from_slice(protocol_name);
        Self {
            ck,
            h: ck,
            key: [0u8; 32],
            nonce: 0,
        }
    }

    pub fn mix_hash(&mut self, data: &[u8]) {
        let mut hasher = Sha256::new();
        hasher.update(self.h);
        hasher.update(data);
        self.h.copy_from_slice(&hasher.finalize());
    }

    fn initialize_key(&mut self, key: [u8; 32]) {
        self.key = key;
        self.nonce = 0;
    }

    pub fn mix_key(&mut self, ikm: &[u8]) {
        let (ck, temp_k) = hkdf2(&self.ck, ikm);
        self.ck = ck;
        self.initialize_key(temp_k);
    }

    pub fn mix_key_and_hash(&mut self, ikm: &[u8]) {
        let (ck, temp_h, temp_k) = hkdf3(&self.ck, ikm);
        self.ck = ck;
        self.mix_hash(&temp_h);
        self.initialize_key(temp_k);
    }

    fn next_nonce(&mut self) -> [u8; 12] {
        let mut nonce = [0u8; 12];
        nonce[..4].copy_from_slice(&self.nonce.to_be_bytes());
        self.nonce += 1;
        nonce
    }

    pub fn encrypt_and_hash(&mut self, plaintext: &[u8]) -> Vec<u8> {
        let nonce = self.next_nonce();
        let cipher = Aes256Gcm::new((&self.key).into());
        let ct = cipher
            .encrypt(
                (&nonce).into(),
                Payload {
                    msg: plaintext,
                    aad: &self.h,
                },
            )
            .expect("AEAD encrypt failed");
        self.mix_hash(&ct);
        ct
    }

    pub fn decrypt_and_hash(&mut self, ciphertext: &[u8]) -> Option<Vec<u8>> {
        let nonce = self.next_nonce();
        let cipher = Aes256Gcm::new((&self.key).into());
        let pt = cipher
            .decrypt(
                (&nonce).into(),
                Payload {
                    msg: ciphertext,
                    aad: &self.h,
                },
            )
            .ok()?;
        self.mix_hash(ciphertext);
        Some(pt)
    }

    pub fn traffic_keys(&self) -> ([u8; 32], [u8; 32]) {
        hkdf2(&self.ck, &[])
    }

    pub fn handshake_hash(&self) -> [u8; 32] {
        self.h
    }
}
