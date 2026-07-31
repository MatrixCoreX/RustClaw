use super::*;

#[test]
fn rejects_empty_and_oversized_files_before_upload() {
    let dir = std::env::temp_dir().join(format!("channel-media-limit-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, []).expect("write empty file");
    assert!(validate_local_media_file(&empty, "test", "文件", 10)
        .unwrap_err()
        .contains("空文件"));

    let large = dir.join("large.bin");
    let file = std::fs::File::create(&large).expect("create sparse file");
    file.set_len(11).expect("set sparse length");
    let error = validate_local_media_file(&large, "test", "视频", 10).unwrap_err();
    assert!(error.contains("过大"));
    assert!(error.contains("UI"));

    let _ = std::fs::remove_dir_all(dir);
}
