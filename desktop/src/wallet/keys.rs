use crate::Result;
use k256::ecdsa::SigningKey;
#[cfg(any(feature = "gui", test))]
use k256::ecdsa::{signature::Signer, Signature};
use ripemd::{Digest, Ripemd160};
use zeroize::Zeroizing;

pub fn generate() -> Result<Zeroizing<[u8; 32]>> {
    for _ in 0..16 {
        let secret = Zeroizing::new(super::crypto::random()?);
        if SigningKey::from_bytes((&*secret).into()).is_ok() {
            return Ok(secret);
        }
    }
    Err("wallet_random_unavailable".into())
}

pub fn public(secret: &[u8; 32]) -> Result<String> {
    let key = SigningKey::from_bytes(secret.into()).map_err(|_| "wallet_key_invalid")?;
    let point = key.verifying_key().to_encoded_point(true);
    let bytes = point.as_bytes();
    let checksum = Ripemd160::digest([bytes, b"K1"].concat());
    Ok(bs58::encode([bytes, &checksum[..4]].concat()).into_string())
}

pub fn validate_public(public: &str) -> Result<()> {
    if public.len() < 40 || public.len() > 60 {
        return Err("wallet_public_key_invalid".into());
    }
    let bytes = bs58::decode(public)
        .into_vec()
        .map_err(|_| "wallet_public_key_invalid")?;
    if bytes.len() != 37 {
        return Err("wallet_public_key_invalid".into());
    }
    let checksum = Ripemd160::digest([&bytes[..33], b"K1"].concat());
    if bytes[33..] != checksum[..4]
        || !matches!(bytes[0], 2 | 3)
        || k256::PublicKey::from_sec1_bytes(&bytes[..33]).is_err()
    {
        return Err("wallet_public_key_invalid".into());
    }
    Ok(())
}

// SigningKey::sign hashes the original UTF-8 payload once with SHA-256.
// Signatures are compact r||s, normalized low-S, matching the browser K1 contract.
#[cfg(any(feature = "gui", test))]
pub(super) fn sign(secret: &[u8; 32], payload: &[u8]) -> Result<String> {
    let key = SigningKey::from_bytes(secret.into()).map_err(|_| "wallet_key_invalid")?;
    let signature: Signature = key.sign(payload);
    Ok(hex::encode(
        signature.normalize_s().unwrap_or(signature).to_bytes(),
    ))
}
