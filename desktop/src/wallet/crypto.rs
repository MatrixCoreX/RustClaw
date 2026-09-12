use super::secure_memory::{LockedBytes, LockedKey};
use crate::Result;
use argon2::{Algorithm, Argon2, Block, Params, Version};
use chacha20poly1305::{
    aead::{Aead, AeadInPlace, KeyInit, Payload},
    XChaCha20Poly1305, XNonce,
};
use serde::{Deserialize, Serialize};

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

fn derive(password: &str, salt: &[u8]) -> Result<LockedKey> {
    derive_with(password, salt, 65_536, 3, 4)
}
pub(super) fn derive_with(
    password: &str,
    salt: &[u8],
    memory: u32,
    iterations: u32,
    lanes: u32,
) -> Result<LockedKey> {
    password_valid(password)?;
    if salt.len() != 16 {
        return Err("wallet_data_invalid".into());
    }
    let mut key = LockedKey::zeroed()?;
    let params =
        Params::new(memory, iterations, lanes, Some(32)).map_err(|_| "wallet_kdf_unavailable")?;
    // The allocating Argon2 entry point does not wipe its full workspace.
    // Own and erase it. This workspace is not locked: 256 MiB exceeds normal
    // unprivileged mlock limits. Output keys use mandatory locked allocations.
    let mut blocks = zeroize::Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(params.block_count())
        .map_err(|_| "wallet_kdf_unavailable")?;
    blocks.resize(params.block_count(), Block::default());
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into_with_memory(
            password.as_bytes(),
            salt,
            key.as_mut(),
            blocks.as_mut_slice(),
        )
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

pub fn open(key: &[u8; 32], aad: &[u8], sealed: &Sealed) -> Result<LockedBytes> {
    if sealed.nonce.len() != 48
        || sealed.ciphertext.len() < 32
        || sealed.ciphertext.len() > 2_000_000
        || sealed.ciphertext.len() % 2 != 0
    {
        return Err("wallet_data_invalid".into());
    }
    let nonce = hex::decode(&sealed.nonce).map_err(|_| "wallet_data_invalid")?;
    let split = sealed.ciphertext.len() - 32;
    let mut bytes = LockedBytes::new(split / 2)?;
    hex::decode_to_slice(&sealed.ciphertext[..split], bytes.as_mut())
        .map_err(|_| "wallet_data_invalid")?;
    let mut tag = [0; 16];
    hex::decode_to_slice(&sealed.ciphertext[split..], &mut tag)
        .map_err(|_| "wallet_data_invalid")?;
    XChaCha20Poly1305::new(key.into())
        .decrypt_in_place_detached(
            XNonce::from_slice(&nonce),
            aad,
            bytes.as_mut(),
            (&tag).into(),
        )
        .map_err(|_| "wallet_unlock_failed")?;
    Ok(bytes)
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

pub fn unwrap(password: &str, aad: &[u8], wrapped: &Wrapped) -> Result<LockedBytes> {
    if wrapped.version != 1 || wrapped.salt.len() != 32 {
        return Err("wallet_data_invalid".into());
    }
    let salt = hex::decode(&wrapped.salt).map_err(|_| "wallet_data_invalid")?;
    open(&*derive(password, &salt)?, aad, &wrapped.sealed)
}
