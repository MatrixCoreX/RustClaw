use p256::ecdsa::{signature::hazmat::PrehashVerifier, Signature, VerifyingKey};
use sha2::{Digest, Sha256};

use crate::store::normalize_device_pubkey;

pub fn verify_enrollment_signature(
    device_pubkey: &str,
    challenge: &str,
    signature_hex: &str,
) -> bool {
    let Ok(device_pubkey) = normalize_device_pubkey(device_pubkey) else {
        return false;
    };
    let signature_hex = signature_hex.trim().to_ascii_lowercase();
    if signature_hex.len() != 128 || !signature_hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return false;
    }
    let Ok(signature_bytes) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(signature) = Signature::from_slice(&signature_bytes) else {
        return false;
    };
    let Ok(public_key_bytes) = hex::decode(device_pubkey) else {
        return false;
    };
    let mut sec1 = Vec::with_capacity(65);
    sec1.push(0x04);
    sec1.extend_from_slice(&public_key_bytes);
    let Ok(verifying_key) = VerifyingKey::from_sec1_bytes(&sec1) else {
        return false;
    };
    let digest = Sha256::digest(challenge.as_bytes());
    verifying_key.verify_prehash(&digest, &signature).is_ok()
}

pub fn canonical_enrollment_challenge(
    challenge_id: &str,
    device_pubkey: &str,
    expires_at_epoch: i64,
) -> String {
    format!(
        "relay-device-enrollment-v1\n{}\n{}\n{}",
        challenge_id, device_pubkey, expires_at_epoch
    )
}
