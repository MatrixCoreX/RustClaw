use super::*;

#[test]
fn list_missing_archive_returns_structured_not_found() {
    let path = std::env::temp_dir().join(format!(
        "rustclaw_missing_archive_{}_{}.zip",
        std::process::id(),
        "unit"
    ));
    let _ = std::fs::remove_file(&path);

    let err = list_archive(&path).expect_err("missing archive should fail");

    assert_eq!(err.kind, "not_found");
    assert!(err.text.contains("archive not found"));
    let expected_path = path.display().to_string();
    assert_eq!(
        err.extra
            .as_ref()
            .and_then(|extra| extra.get("path"))
            .and_then(Value::as_str),
        Some(expected_path.as_str())
    );
    assert_eq!(
        err.extra
            .as_ref()
            .and_then(|extra| extra.get("role"))
            .and_then(Value::as_str),
        Some("archive")
    );
}

#[test]
fn archive_member_rejects_traversal() {
    let err = normalize_archive_member("../secret.txt").expect_err("reject traversal");
    assert_eq!(err.kind, "invalid_input");
    assert!(err.text.contains(".."));
}

#[test]
fn read_archive_member_returns_member_content() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_archive_read_{}_{}",
        std::process::id(),
        "unit"
    ));
    let archive = root.join("bundle.tar.gz");
    std::fs::create_dir_all(&root).expect("create temp archive test dir");
    std::fs::write(root.join("notes.txt"), "fixture archive notes\n")
        .expect("write archive member fixture");
    let status = Command::new("tar")
        .args(["-czf", archive.to_str().unwrap(), "notes.txt"])
        .current_dir(&root)
        .status()
        .expect("create tar fixture");
    assert!(status.success(), "tar fixture creation failed");

    let content = read_archive_member(&archive, "notes.txt").expect("read member");

    assert_eq!(content, "fixture archive notes\n");
    let _ = std::fs::remove_dir_all(&root);
}
