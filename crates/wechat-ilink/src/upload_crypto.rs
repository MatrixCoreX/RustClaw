//! Bounded AES upload stream: no ciphertext scratch file and no whole-file allocation.
use crate::crypto::FILE_CRYPTO_CHUNK_BYTES;
use aes::cipher::{BlockEncrypt, KeyInit};
use aes::Aes128;
use cipher::generic_array::GenericArray;
use claw_core::channel_media_io::{ChannelMediaIoError, MediaIoOperation};
use futures_util::{stream, Stream};
use md5::{Digest, Md5};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::AsyncReadExt;

struct State {
    file: tokio::fs::File,
    cipher: Aes128,
    remaining: u64,
    hash: Md5,
    expected_md5: String,
    validated: Arc<AtomicBool>,
    done: bool,
}

pub(crate) async fn encrypted_file_stream(
    path: &std::path::Path,
    expected_size: u64,
    expected_md5: &str,
    key: &[u8; 16],
    validated: Arc<AtomicBool>,
) -> Result<impl Stream<Item = Result<Vec<u8>, ChannelMediaIoError>> + Send, ChannelMediaIoError> {
    let file = tokio::fs::File::open(path)
        .await
        .map_err(|e| ChannelMediaIoError::from_io(MediaIoOperation::Open, &e))?;
    let metadata = file
        .metadata()
        .await
        .map_err(|e| ChannelMediaIoError::from_io(MediaIoOperation::Inspect, &e))?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(ChannelMediaIoError::input_changed());
    }
    let state = State {
        file,
        cipher: Aes128::new(GenericArray::from_slice(key)),
        remaining: expected_size,
        hash: Md5::new(),
        expected_md5: expected_md5.to_owned(),
        validated,
        done: false,
    };
    Ok(stream::try_unfold(state, |mut state| async move {
        if state.done {
            return Ok(None);
        }
        let full_bytes = (state.remaining / 16) * 16;
        let mut bytes;
        if full_bytes > 0 {
            let count = full_bytes.min(FILE_CRYPTO_CHUNK_BYTES as u64) as usize;
            bytes = vec![0; count];
            state
                .file
                .read_exact(&mut bytes)
                .await
                .map_err(|e| ChannelMediaIoError::from_io(MediaIoOperation::Read, &e))?;
            state.hash.update(&bytes);
            state.remaining -= count as u64;
        } else {
            let remainder = state.remaining as usize;
            bytes = vec![(16 - remainder) as u8; 16];
            state
                .file
                .read_exact(&mut bytes[..remainder])
                .await
                .map_err(|e| ChannelMediaIoError::from_io(MediaIoOperation::Read, &e))?;
            state.hash.update(&bytes[..remainder]);
            let mut unexpected = [0];
            if state
                .file
                .read(&mut unexpected)
                .await
                .map_err(|e| ChannelMediaIoError::from_io(MediaIoOperation::Read, &e))?
                != 0
                || format!("{:x}", state.hash.clone().finalize()) != state.expected_md5
            {
                return Err(ChannelMediaIoError::input_changed());
            }
            state.validated.store(true, Ordering::Release);
            state.done = true;
        }
        for block in bytes.chunks_exact_mut(16) {
            let mut value = GenericArray::clone_from_slice(block);
            state.cipher.encrypt_block(&mut value);
            block.copy_from_slice(&value);
        }
        Ok(Some((bytes, state)))
    }))
}

#[cfg(test)]
#[path = "upload_crypto_tests.rs"]
mod tests;
