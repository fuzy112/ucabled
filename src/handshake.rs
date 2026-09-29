// SPDX-License-Identifier: GPL-3.0-or-later

use p256::ecdh::diffie_hellman;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::{PublicKey, SecretKey};
use rand::rngs::OsRng;

use crate::crypter::Crypter;
use crate::noise::{Noise, KN_PSK0_PROTOCOL_NAME, NK_PSK0_PROTOCOL_NAME};

pub const P256_X962_LENGTH: usize = 65;
pub const RESPONSE_SIZE: usize = P256_X962_LENGTH + 16;

fn uncompressed_point(key: &PublicKey) -> Vec<u8> {
    key.to_encoded_point(false).as_bytes().to_vec()
}

fn parse_point(bytes: &[u8]) -> Option<PublicKey> {
    PublicKey::from_sec1_bytes(bytes).ok()
}

fn ecdh(secret: &SecretKey, peer_bytes: &[u8]) -> Option<[u8; 32]> {
    let peer = parse_point(peer_bytes)?;
    let shared = diffie_hellman(secret.to_nonzero_scalar(), peer.as_affine());
    let x: &[u8] = shared.raw_secret_bytes();
    x.try_into().ok()
}

pub struct HandshakeInitiator {
    noise: Noise,
    identity: Option<SecretKey>,
    #[allow(dead_code)]
    peer_identity: Option<Vec<u8>>,
    ephemeral: Option<SecretKey>,
    ephemeral_public: Vec<u8>,
}

impl HandshakeInitiator {
    /// QR handshake (KNpsk0): local identity key whose public key is in the QR code.
    pub fn new_qr(psk: &[u8; 32], identity: &SecretKey) -> Self {
        let mut noise = Noise::new(KN_PSK0_PROTOCOL_NAME);
        noise.mix_hash(&[1]);
        noise.mix_hash(&uncompressed_point(&identity.public_key()));
        noise.mix_key_and_hash(psk);

        let ephemeral = SecretKey::random(&mut OsRng);
        let ephemeral_public = uncompressed_point(&ephemeral.public_key());
        noise.mix_hash(&ephemeral_public);
        noise.mix_key(&ephemeral_public);

        Self {
            noise,
            identity: Some(identity.clone()),
            peer_identity: None,
            ephemeral: Some(ephemeral),
            ephemeral_public,
        }
    }

    /// Paired handshake (NKpsk0): peer identity known from linking info.
    #[allow(dead_code)]
    pub fn new_paired(psk: &[u8; 32], peer_identity_x962: &[u8]) -> Self {
        let mut noise = Noise::new(NK_PSK0_PROTOCOL_NAME);
        noise.mix_hash(&[0]);
        noise.mix_hash(peer_identity_x962);
        noise.mix_key_and_hash(psk);

        let ephemeral = SecretKey::random(&mut OsRng);
        let ephemeral_public = uncompressed_point(&ephemeral.public_key());
        noise.mix_hash(&ephemeral_public);
        noise.mix_key(&ephemeral_public);

        if let Some(es) = ecdh(&ephemeral, peer_identity_x962) {
            noise.mix_key(&es);
        }

        Self {
            noise,
            identity: None,
            peer_identity: Some(peer_identity_x962.to_vec()),
            ephemeral: Some(ephemeral),
            ephemeral_public,
        }
    }

    pub fn build_initial_message(&mut self) -> Vec<u8> {
        let ct = self.noise.encrypt_and_hash(&[]);
        let mut msg = self.ephemeral_public.clone();
        msg.extend_from_slice(&ct);
        msg
    }

    pub fn process_response(mut self, response: &[u8]) -> Option<(Crypter, [u8; 32])> {
        if response.len() != RESPONSE_SIZE {
            return None;
        }
        let (peer_point_bytes, ciphertext) = response.split_at(P256_X962_LENGTH);

        let ephemeral = self.ephemeral.take()?;
        let shared_ee = ecdh(&ephemeral, peer_point_bytes)?;

        self.noise.mix_hash(peer_point_bytes);
        self.noise.mix_key(peer_point_bytes);
        self.noise.mix_key(&shared_ee);

        if let Some(identity) = &self.identity {
            let shared_se = ecdh(identity, peer_point_bytes)?;
            self.noise.mix_key(&shared_se);
        }

        let plaintext = self.noise.decrypt_and_hash(ciphertext)?;
        if !plaintext.is_empty() {
            return None;
        }

        let (write_key, read_key) = self.noise.traffic_keys();
        Some((
            Crypter::new(read_key, write_key),
            self.noise.handshake_hash(),
        ))
    }
}

/// Phone side: respond to a QR handshake (KNpsk0). Peer identity is the
/// uncompressed public key from the QR code.
pub fn respond_qr(
    psk: &[u8; 32],
    peer_identity_x962: &[u8],
    initial_message: &[u8],
) -> Option<(Vec<u8>, Crypter, [u8; 32])> {
    respond_inner(
        KN_PSK0_PROTOCOL_NAME,
        Some(psk),
        None,
        Some(peer_identity_x962),
        initial_message,
        true,
    )
}

/// Phone side: respond to a paired handshake (NKpsk0) with a local identity.
#[allow(dead_code)]
pub fn respond_paired(
    psk: &[u8; 32],
    identity: &SecretKey,
    initial_message: &[u8],
) -> Option<(Vec<u8>, Crypter, [u8; 32])> {
    respond_inner(
        NK_PSK0_PROTOCOL_NAME,
        Some(psk),
        Some(identity.clone()),
        None,
        initial_message,
        false,
    )
}

fn respond_inner(
    protocol: &[u8],
    psk: Option<&[u8; 32]>,
    identity: Option<SecretKey>,
    peer_identity: Option<&[u8]>,
    initial_message: &[u8],
    qr_prologue: bool,
) -> Option<(Vec<u8>, Crypter, [u8; 32])> {
    if initial_message.len() < P256_X962_LENGTH {
        return None;
    }
    let (peer_point_bytes, ciphertext) = initial_message.split_at(P256_X962_LENGTH);

    let mut noise = Noise::new(protocol);
    noise.mix_hash(&[qr_prologue as u8]);
    if let Some(id) = &identity {
        noise.mix_hash(&uncompressed_point(&id.public_key()));
    } else if let Some(peer) = peer_identity {
        noise.mix_hash(peer);
    }
    if let Some(psk) = psk {
        noise.mix_key_and_hash(psk);
    }

    noise.mix_hash(peer_point_bytes);
    noise.mix_key(peer_point_bytes);

    let ephemeral = SecretKey::random(&mut OsRng);
    parse_point(peer_point_bytes)?;

    if let Some(id) = &identity {
        let es = ecdh(id, peer_point_bytes)?;
        noise.mix_key(&es);
    }

    let plaintext = noise.decrypt_and_hash(ciphertext)?;
    if !plaintext.is_empty() {
        return None;
    }

    let ephemeral_public = uncompressed_point(&ephemeral.public_key());
    noise.mix_hash(&ephemeral_public);
    noise.mix_key(&ephemeral_public);

    let shared_ee = ecdh(&ephemeral, peer_point_bytes)?;
    noise.mix_key(&shared_ee);

    if let Some(peer) = peer_identity {
        let shared_se = ecdh(&ephemeral, peer)?;
        noise.mix_key(&shared_se);
    }

    let my_ciphertext = noise.encrypt_and_hash(&[]);
    let mut response = ephemeral_public;
    response.extend_from_slice(&my_ciphertext);

    let (read_key, write_key) = noise.traffic_keys();
    Some((
        response,
        Crypter::new(read_key, write_key),
        noise.handshake_hash(),
    ))
}
