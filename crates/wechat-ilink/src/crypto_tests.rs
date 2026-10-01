use super::*;

fn temporary_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "agent-runtime-aes-{label}-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ))
}

#[tokio::test]
async fn file_stream_round_trip_matches_in_memory_contract() {
    let key = [7_u8; 16];
    for plaintext in [
        Vec::new(),
        b"short".to_vec(),
        vec![3_u8; 16],
        vec![9_u8; 16 * 1024 + 7],
    ] {
        let input = temporary_path("input");
        let encrypted = temporary_path("encrypted");
        let decrypted = temporary_path("decrypted");
        tokio::fs::write(&input, &plaintext).await.unwrap();
        let (raw_size, encrypted_size) = encrypt_aes_128_ecb_file(&input, &encrypted, &key)
            .await
            .unwrap();
        assert_eq!(raw_size, plaintext.len() as u64);
        assert_eq!(encrypted_size, aes_ecb_padded_size(plaintext.len()) as u64);
        assert_eq!(
            tokio::fs::read(&encrypted).await.unwrap(),
            encrypt_aes_128_ecb(&plaintext, &key).unwrap()
        );
        let decrypted_size = decrypt_aes_128_ecb_file(&encrypted, &decrypted, &key)
            .await
            .unwrap();
        assert_eq!(decrypted_size, plaintext.len() as u64);
        assert_eq!(tokio::fs::read(&decrypted).await.unwrap(), plaintext);
        for path in [input, encrypted, decrypted] {
            let _ = tokio::fs::remove_file(path).await;
        }
    }
}

#[tokio::test]
async fn file_stream_decrypt_rejects_invalid_padding() {
    let encrypted = temporary_path("invalid-encrypted");
    let decrypted = temporary_path("invalid-decrypted");
    tokio::fs::write(&encrypted, [0_u8; 16]).await.unwrap();
    let error = decrypt_aes_128_ecb_file(&encrypted, &decrypted, &[0_u8; 16])
        .await
        .unwrap_err();
    assert!(error.starts_with("pkcs7:"));
    let _ = tokio::fs::remove_file(encrypted).await;
    let _ = tokio::fs::remove_file(decrypted).await;
}
