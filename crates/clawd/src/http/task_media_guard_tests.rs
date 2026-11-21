use super::*;
#[tokio::test]
async fn same_media_shares_one_lock_and_cancellation_releases_it() {
    let first = key_lock(Path::new("/test/a")).unwrap();
    let second = key_lock(Path::new("/test/a")).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    let held = first.lock().await;
    assert!(second.try_lock().is_err());
    drop(held);
    assert!(second.try_lock().is_ok());
}
#[test]
fn temporary_output_is_removed_on_drop() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("partial.tmp");
    std::fs::write(&path, b"partial").unwrap();
    {
        let _guard = TempFile(path.clone());
    }
    assert!(!path.exists());
}
