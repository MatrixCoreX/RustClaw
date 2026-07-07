use super::*;

#[test]
fn error_extra_merges_machine_contract_and_details() {
    let extra = error_extra_with_details(
        "not_found",
        Some(json!({
            "path": "/tmp/missing.zip",
            "role": "archive"
        })),
    );

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "not_found");
    assert_eq!(extra["message_key"], "skill.archive_basic.not_found");
    assert_eq!(extra["retryable"], false);
    assert_eq!(extra["path"], "/tmp/missing.zip");
    assert_eq!(extra["role"], "archive");
}

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
fn list_zip_archive_returns_structured_member_entries() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
        .canonicalize()
        .expect("fixture archive exists");

    let listing = list_archive(&fixture).expect("list fixture archive");

    assert_eq!(
        listing.entries,
        vec!["notes.txt".to_string(), "nested/config.ini".to_string()]
    );
    assert!(listing.output.contains("notes.txt"));
    assert!(listing.output.contains("nested/config.ini"));
}

#[test]
fn execute_list_projects_member_count_and_members() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
        .canonicalize()
        .expect("fixture archive exists");

    let (_text, extra) = execute(json!({
        "action": "list",
        "archive": fixture.display().to_string()
    }))
    .expect("execute list");

    assert_eq!(extra.get("member_count").and_then(Value::as_u64), Some(2));
    assert_eq!(
        extra.pointer("/members/0").and_then(Value::as_str),
        Some("notes.txt")
    );
    assert_eq!(
        extra
            .pointer("/field_value/member_count")
            .and_then(Value::as_u64),
        Some(2)
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

#[test]
fn execute_read_projects_member_path_and_content_excerpt() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip")
        .canonicalize()
        .expect("fixture archive exists");

    let (_text, extra) = execute(json!({
        "action": "read",
        "archive": fixture.display().to_string(),
        "member": "notes.txt"
    }))
    .expect("execute read");

    assert_eq!(extra.get("path").and_then(Value::as_str), Some("notes.txt"));
    assert_eq!(
        extra.get("member_path").and_then(Value::as_str),
        Some("notes.txt")
    );
    assert_eq!(
        extra.get("content_excerpt").and_then(Value::as_str),
        Some("fixture archive notes")
    );
    assert_eq!(
        extra
            .pointer("/field_value/content_excerpt")
            .and_then(Value::as_str),
        Some("fixture archive notes")
    );
}
