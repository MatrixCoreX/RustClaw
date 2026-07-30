use std::fs;

use super::{
    attachment_payload, extract_path_references, inspect_attachment, merge_attachment,
    RequestedAttachmentKind,
};
use crate::chat_session::working_directory_identity;

fn workspace(name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-clawcli-attachment-{name}-{}",
        uuid::Uuid::new_v4()
    ));
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn at_paths_are_lexical_and_skip_quotes_and_code_spans() {
    let paths = extract_path_references(
        "inspect @src/lib.rs and @\"docs/with space.md\" but not `@secret.env` or \"@quoted\"",
    )
    .unwrap();
    assert_eq!(
        paths,
        [
            std::path::PathBuf::from("src/lib.rs"),
            std::path::PathBuf::from("docs/with space.md")
        ]
    );
    assert!(extract_path_references("```\n@inside.rs\n``` @outside.rs")
        .unwrap()
        .ends_with(&[std::path::PathBuf::from("outside.rs")]));
}

#[test]
fn text_attachment_is_confined_hashed_and_encoded() {
    let root = workspace("text");
    fs::create_dir_all(root.join("docs")).unwrap();
    fs::write(root.join("docs/说明.md"), "hello").unwrap();
    let identity = working_directory_identity(&root).unwrap();
    let attachment = inspect_attachment(
        &identity,
        std::path::Path::new("docs/说明.md"),
        RequestedAttachmentKind::File,
    )
    .unwrap();
    assert_eq!(attachment.display_path, "docs/说明.md");
    assert_eq!(attachment.mime_type, "text/markdown");
    assert_eq!(attachment.materialization, "bounded_text_context");
    assert_eq!(attachment.sha256.len(), 64);
    let payload = attachment_payload(&[attachment]).unwrap();
    assert!(payload[0]["base64"]
        .as_str()
        .unwrap()
        .starts_with("data:text/markdown;base64,"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn image_mode_uses_content_signature_not_only_extension() {
    let root = workspace("image");
    fs::write(root.join("frame.bin"), b"\x89PNG\r\n\x1a\npayload").unwrap();
    fs::write(root.join("fake.png"), b"plain text").unwrap();
    let identity = working_directory_identity(&root).unwrap();
    let image = inspect_attachment(
        &identity,
        std::path::Path::new("frame.bin"),
        RequestedAttachmentKind::Image,
    )
    .unwrap();
    assert_eq!(image.kind, "image");
    assert_eq!(image.mime_type, "image/png");
    assert!(inspect_attachment(
        &identity,
        std::path::Path::new("fake.png"),
        RequestedAttachmentKind::Image
    )
    .is_err());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn duplicate_content_is_deduplicated() {
    let root = workspace("dedupe");
    fs::write(root.join("a.txt"), "same").unwrap();
    fs::write(root.join("b.txt"), "same").unwrap();
    let identity = working_directory_identity(&root).unwrap();
    let mut attachments = Vec::new();
    for path in ["a.txt", "b.txt"] {
        let attachment = inspect_attachment(
            &identity,
            std::path::Path::new(path),
            RequestedAttachmentKind::File,
        )
        .unwrap();
        merge_attachment(&mut attachments, attachment).unwrap();
    }
    assert_eq!(attachments.len(), 1);
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn symlink_and_sensitive_paths_are_denied() {
    use std::os::unix::fs::symlink;

    let root = workspace("denied");
    fs::write(root.join("target.txt"), "data").unwrap();
    fs::write(root.join(".env"), "TOKEN=value").unwrap();
    symlink(root.join("target.txt"), root.join("link.txt")).unwrap();
    let identity = working_directory_identity(&root).unwrap();
    assert!(inspect_attachment(
        &identity,
        std::path::Path::new("link.txt"),
        RequestedAttachmentKind::File
    )
    .is_err());
    assert!(inspect_attachment(
        &identity,
        std::path::Path::new(".env"),
        RequestedAttachmentKind::File
    )
    .is_err());
    let _ = fs::remove_dir_all(root);
}
