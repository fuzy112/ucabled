// SPDX-License-Identifier: GPL-3.0-or-later

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const EID_KEY_SIZE: usize = 64;
pub const ADVERT_SIZE: usize = 20;
pub const EID_PLAINTEXT_SIZE: usize = 16;
pub const NONCE_SIZE: usize = 10;
pub const ROUTING_ID_SIZE: usize = 3;

/// EID key layout: AES-256 key followed by an HMAC-SHA256 key.
const KEY_HALF: usize = EID_KEY_SIZE / 2;

/// AES block size, used for both the EID block and the advert tag input.
const AES_BLOCK_SIZE: usize = 16;
/// Truncated HMAC-SHA256 tag appended to the encrypted EID block.
const ADVERT_TAG_SIZE: usize = 4;

/// Field offsets within the 16-byte EID plaintext: reserved flags, nonce,
/// routing ID, tunnel server domain (little-endian).
const FLAGS_OFFSET: usize = 0;
const NONCE_OFFSET: usize = FLAGS_OFFSET + 1;
const ROUTING_ID_OFFSET: usize = NONCE_OFFSET + NONCE_SIZE;
const TUNNEL_DOMAIN_OFFSET: usize = ROUTING_ID_OFFSET + ROUTING_ID_SIZE;

#[derive(Debug, Clone)]
pub struct EidComponents {
    pub nonce: [u8; NONCE_SIZE],
    pub routing_id: [u8; ROUTING_ID_SIZE],
    pub tunnel_server_domain: u16,
}

pub fn plaintext_from_components(c: &EidComponents) -> [u8; EID_PLAINTEXT_SIZE] {
    let mut eid = [0u8; EID_PLAINTEXT_SIZE];
    eid[NONCE_OFFSET..NONCE_OFFSET + NONCE_SIZE].copy_from_slice(&c.nonce);
    eid[ROUTING_ID_OFFSET..ROUTING_ID_OFFSET + ROUTING_ID_SIZE].copy_from_slice(&c.routing_id);
    eid[TUNNEL_DOMAIN_OFFSET..].copy_from_slice(&c.tunnel_server_domain.to_le_bytes());
    eid
}

pub fn to_components(eid: &[u8; EID_PLAINTEXT_SIZE]) -> EidComponents {
    let mut nonce = [0u8; NONCE_SIZE];
    nonce.copy_from_slice(&eid[NONCE_OFFSET..NONCE_OFFSET + NONCE_SIZE]);
    let mut routing_id = [0u8; ROUTING_ID_SIZE];
    routing_id.copy_from_slice(&eid[ROUTING_ID_OFFSET..ROUTING_ID_OFFSET + ROUTING_ID_SIZE]);
    let tunnel_server_domain = u16::from_le_bytes(
        eid[TUNNEL_DOMAIN_OFFSET..TUNNEL_DOMAIN_OFFSET + 2]
            .try_into()
            .unwrap(),
    );
    EidComponents {
        nonce,
        routing_id,
        tunnel_server_domain,
    }
}

pub fn encrypt(eid: &[u8; EID_PLAINTEXT_SIZE], key: &[u8; EID_KEY_SIZE]) -> [u8; ADVERT_SIZE] {
    assert_eq!(eid[FLAGS_OFFSET], 0);
    let cipher = Aes256::new((&key[..KEY_HALF]).into());
    let mut block = (*eid).into();
    cipher.encrypt_block(&mut block);

    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key[KEY_HALF..]).unwrap();
    mac.update(&block);
    let tag = mac.finalize().into_bytes();

    let mut ret = [0u8; ADVERT_SIZE];
    ret[..AES_BLOCK_SIZE].copy_from_slice(&block);
    ret[AES_BLOCK_SIZE..].copy_from_slice(&tag[..ADVERT_TAG_SIZE]);
    ret
}

pub fn decrypt(advert: &[u8], key: &[u8; EID_KEY_SIZE]) -> Option<[u8; EID_PLAINTEXT_SIZE]> {
    if advert.len() != ADVERT_SIZE {
        return None;
    }
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key[KEY_HALF..]).unwrap();
    mac.update(&advert[..AES_BLOCK_SIZE]);
    // verify_truncated_left compares in constant time and accepts the
    // 4-byte prefix we put on the wire; a != on the tag would early-exit
    // on the first mismatching byte.
    if mac
        .verify_truncated_left(&advert[AES_BLOCK_SIZE..])
        .is_err()
    {
        return None;
    }

    let cipher = Aes256::new((&key[..KEY_HALF]).into());
    let block_bytes: [u8; AES_BLOCK_SIZE] = advert[..AES_BLOCK_SIZE].try_into().unwrap();
    let mut block = aes::Block::from(block_bytes);
    cipher.decrypt_block(&mut block);
    let plaintext: [u8; EID_PLAINTEXT_SIZE] = block.into();
    if plaintext[FLAGS_OFFSET] != 0 {
        return None;
    }
    Some(plaintext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(seed: u8) -> [u8; EID_KEY_SIZE] {
        let mut k = [0u8; EID_KEY_SIZE];
        for (i, b) in k.iter_mut().enumerate() {
            *b = seed.wrapping_add(i as u8);
        }
        k
    }

    fn sample() -> EidComponents {
        EidComponents {
            nonce: [7; NONCE_SIZE],
            routing_id: [1, 2, 3],
            tunnel_server_domain: 0x0102,
        }
    }

    #[test]
    fn components_roundtrip() {
        let eid = sample();
        let pt = plaintext_from_components(&eid);
        assert_eq!(pt[0], 0);
        let back = to_components(&pt);
        assert_eq!(back.nonce, eid.nonce);
        assert_eq!(back.routing_id, eid.routing_id);
        assert_eq!(back.tunnel_server_domain, 0x0102);
    }

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let k = key(9);
        let pt = plaintext_from_components(&sample());
        let adv = encrypt(&pt, &k);
        assert_eq!(adv.len(), ADVERT_SIZE);
        assert_eq!(decrypt(&adv, &k), Some(pt));
    }

    #[test]
    fn decrypt_rejects_wrong_key_and_tampering() {
        let k = key(1);
        let pt = plaintext_from_components(&sample());
        let mut adv = encrypt(&pt, &k);
        assert_eq!(decrypt(&adv, &key(2)), None);
        adv[3] ^= 0x40; // corrupt ciphertext -> MAC mismatch
        assert_eq!(decrypt(&adv, &k), None);
        adv[3] ^= 0x40;
        adv[18] ^= 0x01; // corrupt tag
        assert_eq!(decrypt(&adv, &k), None);
    }

    #[test]
    fn decrypt_rejects_bad_length() {
        let k = key(1);
        assert_eq!(decrypt(&[0u8; ADVERT_SIZE - 1], &k), None);
        assert_eq!(decrypt(&[0u8; ADVERT_SIZE + 1], &k), None);
    }
}
