use super::*;
use crate::crypto::{aes_ecb_padded_size, encrypt_aes_128_ecb};
use futures_util::{StreamExt, TryStreamExt};
use std::path::PathBuf;

struct Input(PathBuf);
impl Input {
    async fn new(bytes: &[u8]) -> Self {
        let path = std::env::temp_dir().join(format!(
            "media-upload-input-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        tokio::fs::write(&path, bytes).await.unwrap();
        Self(path)
    }
}
impl Drop for Input {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Md5::digest(bytes))
}

#[tokio::test]
async fn bounded_upload_stream_matches_existing_aes_wire_bytes_at_chunk_and_padding_boundaries() {
    for size in [
        0,
        1,
        15,
        16,
        17,
        FILE_CRYPTO_CHUNK_BYTES - 1,
        FILE_CRYPTO_CHUNK_BYTES,
        FILE_CRYPTO_CHUNK_BYTES + 1,
        FILE_CRYPTO_CHUNK_BYTES * 2 + 7,
    ] {
        let bytes = (0..size).map(|i| (i % 251) as u8).collect::<Vec<_>>();
        let input = Input::new(&bytes).await;
        let valid = Arc::new(AtomicBool::new(false));
        let stream = encrypted_file_stream(
            &input.0,
            size as u64,
            &digest(&bytes),
            &[7; 16],
            valid.clone(),
        )
        .await
        .unwrap();
        let chunks: Vec<Vec<u8>> = stream.try_collect().await.unwrap();
        assert!(chunks
            .iter()
            .all(|c| !c.is_empty() && c.len() <= FILE_CRYPTO_CHUNK_BYTES && c.len() % 16 == 0));
        let encrypted = chunks.concat();
        assert_eq!(encrypted.len(), aes_ecb_padded_size(size));
        assert_eq!(encrypted, encrypt_aes_128_ecb(&bytes, &[7; 16]).unwrap());
        assert!(valid.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn upload_rejects_growth_shrink_and_same_length_mutation_before_final_padding() {
    use tokio::io::{AsyncSeekExt, AsyncWriteExt};
    for change in ["grow", "shrink", "overwrite"] {
        let bytes = vec![11; FILE_CRYPTO_CHUNK_BYTES + 17];
        let input = Input::new(&bytes).await;
        let valid = Arc::new(AtomicBool::new(false));
        let stream = encrypted_file_stream(
            &input.0,
            bytes.len() as u64,
            &digest(&bytes),
            &[7; 16],
            valid.clone(),
        )
        .await
        .unwrap();
        tokio::pin!(stream);
        assert_eq!(
            stream.next().await.unwrap().unwrap().len(),
            FILE_CRYPTO_CHUNK_BYTES
        );
        let mut writer = tokio::fs::OpenOptions::new()
            .write(true)
            .open(&input.0)
            .await
            .unwrap();
        match change {
            "grow" => writer.set_len(bytes.len() as u64 + 1).await.unwrap(),
            "shrink" => writer
                .set_len(FILE_CRYPTO_CHUNK_BYTES as u64 + 1)
                .await
                .unwrap(),
            _ => {
                writer
                    .seek(std::io::SeekFrom::Start(
                        FILE_CRYPTO_CHUNK_BYTES as u64 + 16,
                    ))
                    .await
                    .unwrap();
                writer.write_all(&[99]).await.unwrap();
                writer.flush().await.unwrap();
            }
        }
        drop(writer);
        let mut failure = None;
        while let Some(item) = stream.next().await {
            if let Err(error) = item {
                failure = Some(error);
                break;
            }
        }
        assert_eq!(
            failure.unwrap().error_code(),
            "channel.media.local_input_changed"
        );
        assert!(!valid.load(Ordering::Acquire));
    }
}

#[tokio::test]
async fn local_read_failure_is_typed_before_any_body_is_created() {
    let input = Input::new(b"temporary").await;
    tokio::fs::remove_file(&input.0).await.unwrap();
    let result = encrypted_file_stream(
        &input.0,
        9,
        &digest(b"temporary"),
        &[7; 16],
        Arc::new(AtomicBool::new(false)),
    )
    .await;
    let error = match result {
        Err(e) => e,
        Ok(_) => panic!("missing input accepted"),
    };
    assert_eq!(error.error_code(), "channel.media.local_unavailable");
}

#[tokio::test]
async fn upload_stream_does_not_require_a_usable_system_temporary_directory() {
    const CHILD: &str = "APP_TEST_MEDIA_UPLOAD_INPUT";
    if let Some(path) = std::env::var_os(CHILD) {
        let bytes = vec![23; 65537];
        let stream = encrypted_file_stream(
            std::path::Path::new(&path),
            bytes.len() as u64,
            &digest(&bytes),
            &[7; 16],
            Arc::new(AtomicBool::new(false)),
        )
        .await
        .unwrap();
        let chunks: Vec<Vec<u8>> = stream.try_collect().await.unwrap();
        assert_eq!(
            chunks.concat(),
            encrypt_aes_128_ecb(&bytes, &[7; 16]).unwrap()
        );
        return;
    }
    let input = Input::new(&vec![23; 65537]).await;
    // A regular file cannot be used as a temporary directory on any supported OS.
    let unusable = Input::new(b"not a directory").await;
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact").arg("upload_crypto::tests::upload_stream_does_not_require_a_usable_system_temporary_directory")
        .arg("--nocapture").env_clear().env(CHILD, &input.0)
        .env("TMPDIR",&unusable.0).env("TMP",&unusable.0).env("TEMP",&unusable.0)
        .output().unwrap();
    assert!(
        result.status.success(),
        "child stdout={} stderr={}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}
