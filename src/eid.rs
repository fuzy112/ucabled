use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes256;
use hmac::{Hmac, Mac};
use sha2::Sha256;

pub const EID_KEY_SIZE: usize = 64;
pub const ADVERT_SIZE: usize = 20;
pub const EID_PLAINTEXT_SIZE: usize = 16;
pub const NONCE_SIZE: usize = 10;
pub const ROUTING_ID_SIZE: usize = 3;

#[derive(Debug, Clone)]
pub struct EidComponents {
    pub nonce: [u8; NONCE_SIZE],
    pub routing_id: [u8; ROUTING_ID_SIZE],
    pub tunnel_server_domain: u16,
}

pub fn plaintext_from_components(c: &EidComponents) -> [u8; EID_PLAINTEXT_SIZE] {
    let mut eid = [0u8; EID_PLAINTEXT_SIZE];
    eid[1..1 + NONCE_SIZE].copy_from_slice(&c.nonce);
    eid[11..11 + ROUTING_ID_SIZE].copy_from_slice(&c.routing_id);
    eid[14..16].copy_from_slice(&c.tunnel_server_domain.to_le_bytes());
    eid
}

pub fn to_components(eid: &[u8; EID_PLAINTEXT_SIZE]) -> EidComponents {
    let mut nonce = [0u8; NONCE_SIZE];
    nonce.copy_from_slice(&eid[1..11]);
    let mut routing_id = [0u8; ROUTING_ID_SIZE];
    routing_id.copy_from_slice(&eid[11..14]);
    let tunnel_server_domain = u16::from_le_bytes([eid[14], eid[15]]);
    EidComponents {
        nonce,
        routing_id,
        tunnel_server_domain,
    }
}

pub fn encrypt(eid: &[u8; EID_PLAINTEXT_SIZE], key: &[u8; EID_KEY_SIZE]) -> [u8; ADVERT_SIZE] {
    assert_eq!(eid[0], 0);
    let cipher = Aes256::new((&key[..32]).into());
    let mut block = (*eid).into();
    cipher.encrypt_block(&mut block);

    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key[32..]).unwrap();
    mac.update(&block);
    let tag = mac.finalize().into_bytes();

    let mut ret = [0u8; ADVERT_SIZE];
    ret[..16].copy_from_slice(&block);
    ret[16..].copy_from_slice(&tag[..4]);
    ret
}

pub fn decrypt(advert: &[u8], key: &[u8; EID_KEY_SIZE]) -> Option<[u8; EID_PLAINTEXT_SIZE]> {
    if advert.len() != ADVERT_SIZE {
        return None;
    }
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key[32..]).unwrap();
    mac.update(&advert[..16]);
    let tag = mac.finalize().into_bytes();
    if tag[..4] != advert[16..] {
        return None;
    }

    let cipher = Aes256::new((&key[..32]).into());
    let block_bytes: [u8; 16] = advert[..16].try_into().unwrap();
    let mut block = aes::Block::from(block_bytes);
    cipher.decrypt_block(&mut block);
    let plaintext: [u8; EID_PLAINTEXT_SIZE] = block.into();
    if plaintext[0] != 0 {
        return None;
    }
    Some(plaintext)
}
