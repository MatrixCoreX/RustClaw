use super::*;
fn input(bytes: &[u8]) -> UploadInput {
    UploadInput {
        name: "sample.txt".into(),
        mime_type: "text/plain".into(),
        kind: "file".into(),
        bytes: bytes.into(),
    }
}

#[test]
fn verified_references_rebuild_media_aliases_and_reject_unbound_locators() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let items = commit_upload_batch(root, 1, 2, "message", vec![input(b"original")]).unwrap();
    let mut payload = json!({"attachments":items, "audio":{"path":"private/other.wav"}, "images":[{"path":"private/other.png"}]});
    validate_upload_references(root, 1, 2, "message", &mut payload).unwrap();
    assert!(payload.get("audio").is_none());
    assert!(payload.get("images").is_none());
    assert_eq!(
        payload["files"][0]["path"],
        payload["attachments"][0]["path"]
    );
    let mut unbound = json!({"audio":{"path":"private/other.wav"}});
    assert_eq!(
        validate_upload_references(root, 1, 2, "message", &mut unbound).unwrap_err(),
        "ui_attachment_reference_missing"
    );
}
#[test]
fn batch_replay_is_immutable_and_invalid_later_file_writes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let items = commit_upload_batch(root, 1, 2, "message", vec![input(b"original")]).unwrap();
    assert!(commit_upload_batch(root, 1, 2, "message", vec![input(b"original")]).is_ok());
    assert_eq!(
        commit_upload_batch(root, 1, 2, "message", vec![input(b"changed")]).unwrap_err(),
        "ui_attachment_replay_conflict"
    );
    assert_eq!(
        std::fs::read(root.join(&items[0].rel_path)).unwrap(),
        b"original"
    );
    assert!(commit_upload_batch(
        root,
        1,
        2,
        "invalid",
        vec![
            input(b"first"),
            input(&vec![0; MAX_UI_ATTACHMENT_BYTES + 1])
        ]
    )
    .is_err());
    assert!(!root
        .join(format!(
            "data/ui/1/2/{}",
            hex::encode(Sha256::digest(b"invalid"))
        ))
        .exists());
}
#[cfg(unix)]
#[test]
fn symlinked_parent_cannot_redirect_uploads() {
    let directory = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), directory.path().join("data")).unwrap();
    assert!(
        commit_upload_batch(directory.path(), 1, 2, "message", vec![input(b"private")]).is_err()
    );
    assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
}
