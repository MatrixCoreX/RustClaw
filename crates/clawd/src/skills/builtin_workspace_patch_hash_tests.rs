use super::*;

#[test]
fn patch_hash_streams_large_sparse_files_and_the_final_partial_chunk() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let path = root.join("sparse.bin");
    let size = 8 * 1024 * 1024 + 37;
    fs::File::create(&path).unwrap().set_len(size).unwrap();
    let mut expected = Sha256::new();
    let zeros = [0; 4096];
    let mut remaining = size;
    while remaining != 0 {
        let count = remaining.min(zeros.len() as u64) as usize;
        expected.update(&zeros[..count]);
        remaining -= count as u64;
    }
    assert_eq!(
        hash_existing_file(&root, &path, &filesystem).unwrap(),
        Some(format!("sha256:{:x}", expected.finalize()))
    );
}

#[test]
fn patch_hash_and_optional_reads_preserve_missing_empty_and_binary_semantics() {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let filesystem = HostFilesystemPolicy::without_private_authority();
    let path = root.join("data.bin");
    assert_eq!(hash_existing_file(&root, &path, &filesystem).unwrap(), None);
    assert_eq!(
        read_optional_regular_file(&root, &path, &filesystem).unwrap(),
        None
    );
    fs::write(&path, []).unwrap();
    assert_eq!(
        hash_existing_file(&root, &path, &filesystem).unwrap(),
        Some(sha256_label(b""))
    );
    let bytes = b"\x00\xffline\n";
    fs::write(&path, bytes).unwrap();
    assert_eq!(
        hash_existing_file(&root, &path, &filesystem).unwrap(),
        Some(sha256_label(bytes))
    );
    assert_eq!(
        read_optional_regular_file(&root, &path, &filesystem).unwrap(),
        Some(bytes.to_vec())
    );
}

#[test]
fn patch_hash_keeps_private_path_and_nonregular_file_admission() {
    use std::os::unix::fs::{symlink, DirBuilderExt};
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().canonicalize().unwrap();
    let private = root.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("data.bin"), b"owned private fixture").unwrap();
    fs::write(root.join("public.bin"), b"public fixture").unwrap();
    let filesystem = HostFilesystemPolicy::pin_directories(&[private.clone()], &root).unwrap();
    let error = hash_existing_file(&root, &private.join("data.bin"), &filesystem).unwrap_err();
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "host_private_directory_access_denied"
    );
    assert!(hash_existing_file(&root, &private, &filesystem).is_err());
    fs::create_dir(root.join("directory")).unwrap();
    assert!(hash_existing_file(&root, &root.join("directory"), &filesystem).is_err());
    symlink(root.join("public.bin"), root.join("alias.bin")).unwrap();
    assert!(hash_existing_file(&root, &root.join("alias.bin"), &filesystem).is_err());
    assert_eq!(
        hash_existing_file(&root, &root.join("public.bin"), &filesystem).unwrap(),
        Some(sha256_label(b"public fixture"))
    );
}
