//! AES-128-ECB + PKCS#7 (aligned with OpenClaw weixin `aes-ecb.ts` / `pic-decrypt.ts`).

use aes::cipher::{BlockDecrypt, BlockEncrypt, KeyInit};
use aes::Aes128;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cipher::generic_array::GenericArray;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::Path;

const FILE_CRYPTO_CHUNK_BYTES: usize = 256 * 1024;

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

pub async fn encrypt_aes_128_ecb_file(
    input_path: &Path,
    output_path: &Path,
    key: &[u8; 16],
) -> Result<(u64, u64), String> {
    let input_path = input_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let key = *key;
    tokio::task::spawn_blocking(move || {
        encrypt_aes_128_ecb_file_blocking(&input_path, &output_path, &key)
    })
    .await
    .map_err(|error| format!("aes encrypt worker: {error}"))?
}

fn encrypt_aes_128_ecb_file_blocking(
    input_path: &Path,
    output_path: &Path,
    key: &[u8; 16],
) -> Result<(u64, u64), String> {
    let plaintext_size = std::fs::metadata(input_path)
        .map_err(|error| format!("aes input metadata: {error}"))?
        .len();
    let cipher = Aes128::new_from_slice(key).map_err(|_| "aes: bad key len")?;
    let input =
        std::fs::File::open(input_path).map_err(|error| format!("aes input open: {error}"))?;
    let output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output_path)
        .map_err(|error| format!("aes output create: {error}"))?;
    let mut input = BufReader::with_capacity(FILE_CRYPTO_CHUNK_BYTES, input);
    let mut output = BufWriter::with_capacity(FILE_CRYPTO_CHUNK_BYTES, output);
    let full_blocks = plaintext_size / 16;
    let remainder = (plaintext_size % 16) as usize;
    let mut remaining_full_bytes = full_blocks.saturating_mul(16);
    let mut buffer = vec![0_u8; FILE_CRYPTO_CHUNK_BYTES];
    while remaining_full_bytes > 0 {
        let read_len = usize::try_from(remaining_full_bytes.min(buffer.len() as u64))
            .map_err(|_| "aes input chunk exceeds platform address space".to_string())?;
        input
            .read_exact(&mut buffer[..read_len])
            .map_err(|error| format!("aes input read: {error}"))?;
        for chunk in buffer[..read_len].chunks_exact_mut(16) {
            let mut block = GenericArray::clone_from_slice(chunk);
            cipher.encrypt_block(&mut block);
            chunk.copy_from_slice(&block);
        }
        output
            .write_all(&buffer[..read_len])
            .map_err(|error| format!("aes output write: {error}"))?;
        remaining_full_bytes = remaining_full_bytes.saturating_sub(read_len as u64);
    }
    let mut block_bytes = [0_u8; 16];
    let pad = 16 - remainder;
    block_bytes.fill(pad as u8);
    if remainder > 0 {
        input
            .read_exact(&mut block_bytes[..remainder])
            .map_err(|error| format!("aes input tail read: {error}"))?;
    }
    let mut block = GenericArray::clone_from_slice(&block_bytes);
    cipher.encrypt_block(&mut block);
    output
        .write_all(&block)
        .map_err(|error| format!("aes output tail write: {error}"))?;
    let mut unexpected = [0_u8; 1];
    if input
        .read(&mut unexpected)
        .map_err(|error| format!("aes input final read: {error}"))?
        != 0
    {
        return Err("aes input changed while reading".to_string());
    }
    output
        .flush()
        .map_err(|error| format!("aes output flush: {error}"))?;
    let plaintext_size_usize = usize::try_from(plaintext_size)
        .map_err(|_| "aes input exceeds platform address space".to_string())?;
    Ok((
        plaintext_size,
        aes_ecb_padded_size(plaintext_size_usize) as u64,
    ))
}

pub async fn decrypt_aes_128_ecb_file(
    input_path: &Path,
    output_path: &Path,
    key: &[u8; 16],
) -> Result<u64, String> {
    let input_path = input_path.to_path_buf();
    let output_path = output_path.to_path_buf();
    let key = *key;
    tokio::task::spawn_blocking(move || {
        decrypt_aes_128_ecb_file_blocking(&input_path, &output_path, &key)
    })
    .await
    .map_err(|error| format!("aes decrypt worker: {error}"))?
}

fn decrypt_aes_128_ecb_file_blocking(
    input_path: &Path,
    output_path: &Path,
    key: &[u8; 16],
) -> Result<u64, String> {
    let ciphertext_size = std::fs::metadata(input_path)
        .map_err(|error| format!("aes ciphertext metadata: {error}"))?
        .len();
    if ciphertext_size == 0 || ciphertext_size % 16 != 0 {
        return Err("aes ecb decrypt: ciphertext not a non-empty multiple of 16".to_string());
    }
    let cipher = Aes128::new_from_slice(key).map_err(|_| "aes: bad key len")?;
    let input =
        std::fs::File::open(input_path).map_err(|error| format!("aes ciphertext open: {error}"))?;
    let output = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(output_path)
        .map_err(|error| format!("aes plaintext create: {error}"))?;
    let mut input = BufReader::with_capacity(FILE_CRYPTO_CHUNK_BYTES, input);
    let mut output = BufWriter::with_capacity(FILE_CRYPTO_CHUNK_BYTES, output);
    let mut remaining_body_bytes = ciphertext_size.saturating_sub(16);
    let mut buffer = vec![0_u8; FILE_CRYPTO_CHUNK_BYTES];
    while remaining_body_bytes > 0 {
        let read_len = usize::try_from(remaining_body_bytes.min(buffer.len() as u64))
            .map_err(|_| "aes ciphertext chunk exceeds platform address space".to_string())?;
        input
            .read_exact(&mut buffer[..read_len])
            .map_err(|error| format!("aes ciphertext read: {error}"))?;
        for chunk in buffer[..read_len].chunks_exact_mut(16) {
            let mut block = GenericArray::clone_from_slice(chunk);
            cipher.decrypt_block(&mut block);
            chunk.copy_from_slice(&block);
        }
        output
            .write_all(&buffer[..read_len])
            .map_err(|error| format!("aes plaintext write: {error}"))?;
        remaining_body_bytes = remaining_body_bytes.saturating_sub(read_len as u64);
    }
    let mut final_block_bytes = [0_u8; 16];
    input
        .read_exact(&mut final_block_bytes)
        .map_err(|error| format!("aes ciphertext read: {error}"))?;
    let mut final_block = GenericArray::clone_from_slice(&final_block_bytes);
    cipher.decrypt_block(&mut final_block);
    let pad = final_block[15] as usize;
    if pad == 0
        || pad > 16
        || !final_block[16 - pad..]
            .iter()
            .all(|byte| *byte as usize == pad)
    {
        return Err("pkcs7: inconsistent padding".to_string());
    }
    output
        .write_all(&final_block[..16 - pad])
        .map_err(|error| format!("aes plaintext write: {error}"))?;
    output
        .flush()
        .map_err(|error| format!("aes plaintext flush: {error}"))?;
    Ok(ciphertext_size.saturating_sub(pad as u64))
}

#[cfg(test)]
#[path = "crypto_tests.rs"]
mod tests;
