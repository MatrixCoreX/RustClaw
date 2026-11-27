use super::*;
use crate::channel_ingress::ChannelIngressAttachment;

#[test]
fn received_file_is_private_and_replay_cannot_replace_different_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let item = ChannelIngressAttachment {
        kind: "file".into(),
        path: "inbox/note.txt".into(),
        mime_type: None,
        size: Some(4),
    };
    std::fs::create_dir(root.join("inbox")).unwrap();
    std::fs::write(root.join(&item.path), b"note").unwrap();
    let own = private_directory(root, "first").unwrap();
    let other = private_directory(root, "second").unwrap();
    for _ in 0..2 {
        stage_inbox_files(root, &own, std::slice::from_ref(&item)).unwrap();
    }
    assert!(!other.join(&item.path).exists());
    std::fs::write(root.join(&item.path), b"edit").unwrap();
    assert!(stage_inbox_files(root, &own, std::slice::from_ref(&item)).is_err());
    assert_eq!(std::fs::read(own.join(&item.path)).unwrap(), b"note");
}
