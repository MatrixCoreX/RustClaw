//! AES-128-ECB + PKCS#7 (aligned with OpenClaw weixin `aes-ecb.ts` / `pic-decrypt.ts`).

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cipher::generic_array::GenericArray;

/// Parse `CDNMedia.aes_key` (base64): raw 16 bytes or base64 of 32-char hex ASCII.
pub fn parse_aes_key_base64(aes_key_base64: &str, label: &str) -> Result<[u8; 16], String> {
    let decoded = B64
        .decode(aes_key_base64.trim())
        .map_err(|e| format!("{label}: aes_key base64 decode: {e}"))?;
    if decoded.len() == 16 {
        let mut k = [0u8; 16];
        k.copy_from_slice(&decoded);
        return Ok(k);
    }
    if decoded.len() == 32 {
        let s = std::str::from_utf8(&decoded)
            .map_err(|_| format!("{label}: aes_key inner not utf8"))?;
        if s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
            let raw = hex::decode(s).map_err(|e| format!("{label}: aes_key hex: {e}"))?;
            if raw.len() == 16 {
                let mut k = [0u8; 16];
                k.copy_from_slice(&raw);
                return Ok(k);
            }
        }
    }
    Err(format!(
        "{label}: aes_key must be 16 raw bytes or 32-char hex (base64-wrapped), got {} bytes",
        decoded.len()
    ))
}

/// `image_item.aeskey` is often hex (32 chars) — convert to raw 16-byte key.
pub fn parse_aes_key_hex_or_base64_media(
    aeskey_hex: Option<&str>,
    media_aes_key_b64: Option<&str>,
    label: &str,
) -> Result<[u8; 16], String> {
    if let Some(h) = aeskey_hex.map(str::trim).filter(|s| !s.is_empty()) {
        let raw = hex::decode(h).map_err(|e| format!("{label}: image aeskey hex: {e}"))?;
        if raw.len() != 16 {
            return Err(format!("{label}: image aeskey hex len {}", raw.len()));
        }
        let mut k = [0u8; 16];
        k.copy_from_slice(&raw);
        return Ok(k);
    }
    if let Some(b64) = media_aes_key_b64.map(str::trim).filter(|s| !s.is_empty()) {
        return parse_aes_key_base64(b64, label);
    }
    Err(format!("{label}: missing aes key"))
}

pub fn aes_ecb_padded_size(plaintext_len: usize) -> usize {
    let n = plaintext_len + 1;
    ((n + 15) / 16) * 16
}

fn pkcs7_pad(data: &[u8]) -> Vec<u8> {
    let block = 16usize;
    let pad_len = block - (data.len() % block);
    let mut out = data.to_vec();
    out.extend(std::iter::repeat(pad_len as u8).take(pad_len));
    out
}

fn pkcs7_unpad(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.is_empty() || data.len() % 16 != 0 {
        return Err("pkcs7: bad length".to_string());
    }
    let pad = *data.last().unwrap() as usize;
    if pad == 0 || pad > 16 || pad > data.len() {
        return Err("pkcs7: bad padding".to_string());
    }
    if !data[data.len() - pad..].iter().all(|&b| b as usize == pad) {
        return Err("pkcs7: inconsistent padding".to_string());
    }
    Ok(data[..data.len() - pad].to_vec())
}

pub fn encrypt_aes_128_ecb(plaintext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, String> {
    let cipher = Aes128::new_from_slice(key).map_err(|_| "aes: bad key len")?;
    let padded = pkcs7_pad(plaintext);
    let mut out = padded.clone();
    for chunk in out.chunks_mut(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.encrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    Ok(out)
}

pub fn decrypt_aes_128_ecb(ciphertext: &[u8], key: &[u8; 16]) -> Result<Vec<u8>, String> {
    if ciphertext.len() % 16 != 0 {
        return Err("aes ecb decrypt: ciphertext not multiple of 16".to_string());
    }
    let cipher = Aes128::new_from_slice(key).map_err(|_| "aes: bad key len")?;
    let mut out = ciphertext.to_vec();
    for chunk in out.chunks_mut(16) {
        let mut block = GenericArray::clone_from_slice(chunk);
        cipher.decrypt_block(&mut block);
        chunk.copy_from_slice(&block);
    }
    pkcs7_unpad(&out)
}
