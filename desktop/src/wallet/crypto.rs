use crate::Result;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sealed {
    pub nonce: String,
    pub ciphertext: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wrapped {
    pub version: u32,
    pub salt: String,
    pub sealed: Sealed,
}

pub fn random<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0; N];
    getrandom::getrandom(&mut bytes).map_err(|_| "wallet_random_unavailable")?;
    Ok(bytes)
}

pub fn password_valid(password: &str) -> Result<()> {
    if password.chars().count() < 12 || password.len() > 1024 {
        return Err("wallet_password_length".into());
    }
    Ok(())
}

fn derive(password: &str, salt: &[u8]) -> Result<Zeroizing<[u8; 32]>> {
    password_valid(password)?;
    if salt.len() != 16 {
        return Err("wallet_data_invalid".into());
    }
    let mut key = Zeroizing::new([0; 32]);
    let params = Params::new(65_536, 3, 4, Some(32)).map_err(|_| "wallet_kdf_unavailable")?;
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, key.as_mut())
        .map_err(|_| "wallet_kdf_unavailable")?;
    Ok(key)
}

pub fn seal(key: &[u8; 32], aad: &[u8], clear: &[u8]) -> Result<Sealed> {
    let nonce = random::<24>()?;
    let cipher = XChaCha20Poly1305::new(key.into());
    let encrypted = cipher
        .encrypt(XNonce::from_slice(&nonce), Payload { msg: clear, aad })
        .map_err(|_| "wallet_encrypt_failed")?;
    Ok(Sealed {
        nonce: hex::encode(nonce),
        ciphertext: hex::encode(encrypted),
    })
}

pub fn open(key: &[u8; 32], aad: &[u8], sealed: &Sealed) -> Result<Zeroizing<Vec<u8>>> {
    if sealed.nonce.len() != 48 || sealed.ciphertext.len() > 2_000_000 {
        return Err("wallet_data_invalid".into());
    }
    let nonce = hex::decode(&sealed.nonce).map_err(|_| "wallet_data_invalid")?;
    let bytes = hex::decode(&sealed.ciphertext).map_err(|_| "wallet_data_invalid")?;
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(&nonce), Payload { msg: &bytes, aad })
        .map(Zeroizing::new)
        .map_err(|_| "wallet_unlock_failed".into())
}

pub fn wrap(password: &str, aad: &[u8], clear: &[u8]) -> Result<Wrapped> {
    let salt = random::<16>()?;
    let key = derive(password, &salt)?;
    Ok(Wrapped {
        version: 1,
        salt: hex::encode(salt),
        sealed: seal(&key, aad, clear)?,
    })
}

pub fn unwrap(password: &str, aad: &[u8], wrapped: &Wrapped) -> Result<Zeroizing<Vec<u8>>> {
    if wrapped.version != 1 || wrapped.salt.len() != 32 {
        return Err("wallet_data_invalid".into());
    }
    let salt = hex::decode(&wrapped.salt).map_err(|_| "wallet_data_invalid")?;
    open(&*derive(password, &salt)?, aad, &wrapped.sealed)
}
