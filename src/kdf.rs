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

pub fn derive(secret: &[u8], salt: &[u8], purpose: Purpose, out: &mut [u8]) {
    let info = (purpose as u32).to_le_bytes();
    let hk = Hkdf::<Sha256>::new(Some(salt), secret);
    hk.expand(&info, out).expect("HKDF expand failed");
}
