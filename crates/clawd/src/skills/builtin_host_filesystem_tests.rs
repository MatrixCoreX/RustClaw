use super::*;
use crate::runtime::host_filesystem::HostFilesystemPolicy;
use std::os::unix::fs::DirBuilderExt;

#[tokio::test]
async fn host_private_listing_dispatch_refuses_private_entries_and_preserves_public_paging() {
    let root = TempDirGuard::new("host_private_listing");
    let private = root.path.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("private-marker"), b"owned-private-marker").unwrap();
    fs::create_dir(root.path.join("public")).unwrap();
    fs::write(root.path.join("public/a"), b"public-a").unwrap();
    fs::write(root.path.join("public/b"), b"public-b").unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private.clone()], &root.path).unwrap());
    let public = execute_builtin_skill(
        &state,
        "list_dir",
        &json!({"path":"public", "max_entries":1}),
    )
    .await
    .unwrap();
    let public: Value = serde_json::from_str(&public).unwrap();
    assert_eq!(public["names"], json!(["a"]));
    assert_eq!(public["next_cursor"], 1);
    let result = execute_builtin_skill(&state, "list_dir", &json!({"path":"private"})).await;
    eprintln!(
        "HOST_PRIVATE_LIST actual_builtin=true refused={} llm_calls=0",
        result.is_err()
    );
    assert!(
        result.is_err(),
        "native listing must not enumerate private entries"
    );
    fs::rename(&private, root.path.join("moved-private")).unwrap();
    let moved = execute_builtin_skill(&state, "list_dir", &json!({"path":"moved-private"})).await;
    assert!(
        moved.is_err(),
        "renaming a private directory cannot grant listing access"
    );
}

#[tokio::test]
async fn host_private_listing_preserves_public_aliases_and_symlink_entry_metadata() {
    use std::os::unix::fs::symlink;
    let root = TempDirGuard::new("host_private_listing_alias");
    let private = root.path.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::create_dir(root.path.join("public")).unwrap();
    fs::write(root.path.join("public/file"), b"abc").unwrap();
    symlink("not-present", root.path.join("public/link")).unwrap();
    symlink("public", root.path.join("alias")).unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    let output = execute_builtin_skill(&state, "list_dir", &json!({"path":"alias"}))
        .await
        .unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["names"], json!(["file", "link"]));
    assert_eq!(
        value["canonical_path"],
        root.path.join("public").display().to_string()
    );
    assert_eq!(value["entries"][0]["size_bytes"], 3);
    assert_eq!(value["entries"][1]["is_symlink"], true);
    assert_eq!(value["entries"][1]["size_bytes"], Value::Null);
}

#[tokio::test]
async fn host_private_read_dispatch_refuses_private_data_and_preserves_public_reads() {
    let root = TempDirGuard::new("host_private_read");
    let private = root.path.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    fs::write(root.path.join("public"), b"public-marker").unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    let public = execute_builtin_skill(&state, "read_file", &json!({"path":"public"}))
        .await
        .unwrap();
    let public: Value = serde_json::from_str(&public).unwrap();
    assert_eq!(public["content"], "public-marker");
    let private =
        execute_builtin_skill(&state, "read_file", &json!({"path":"private/marker"})).await;
    eprintln!(
        "HOST_PRIVATE_READ actual_builtin=true refused={} llm_calls=0",
        private.is_err()
    );
    assert!(
        private.is_err(),
        "native dispatch must enforce the host's directory policy"
    );
}

#[tokio::test]
async fn host_private_read_dispatch_keeps_inode_fencing_after_host_ancestor_replacement() {
    let root = TempDirGuard::new("host_private_moved");
    let parent = root.path.join("parent");
    fs::create_dir(&parent).unwrap();
    let private = parent.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    fs::write(parent.join("public"), b"public-marker").unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    fs::rename(&parent, root.path.join("moved")).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::DirBuilder::new()
        .mode(0o700)
        .create(parent.join("private"))
        .unwrap();
    let result =
        execute_builtin_skill(&state, "read_file", &json!({"path":"moved/private/marker"})).await;
    assert!(result.is_err());
    let public = execute_builtin_skill(&state, "read_file", &json!({"path":"moved/public"}))
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&public).unwrap()["content"],
        "public-marker"
    );
    let command = execute_builtin_skill(
        &state,
        "run_cmd",
        &json!({"command":"touch must-not-launch"}),
    )
    .await
    .unwrap_err();
    assert!(command.contains("host_private_directory_changed"));
    assert!(!root.path.join("must-not-launch").exists());
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn host_private_policy_reaches_the_actual_foreground_command_dispatch() {
    let root = TempDirGuard::new("host_private_command");
    let private = root.path.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    fs::write(root.path.join("public"), b"public-marker").unwrap();
    let mut state = test_state(root.path.clone());
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private.clone()], &root.path).unwrap());
    let output = execute_builtin_skill(&state, "run_cmd", &json!({
        "command":"if cat private/marker >/dev/null 2>&1; then exit 41; fi; if (printf sandbox-only > private/child-output) 2>/dev/null; then exit 42; fi; cat public > observed"
    })).await.unwrap();
    let value: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(value["exit_code"], 0);
    assert_eq!(
        fs::read(root.path.join("observed")).unwrap(),
        b"public-marker"
    );
    assert_eq!(
        fs::read(private.join("marker")).unwrap(),
        b"owned-private-marker"
    );
    assert!(!private.join("child-output").exists());
}

#[tokio::test]
async fn host_private_policy_preserves_configured_outside_workspace_public_reads() {
    use std::os::unix::fs::symlink;
    let root = TempDirGuard::new("host_private_outside");
    let outside = TempDirGuard::new("host_private_outside_public");
    let private = root.path.join("private");
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    fs::write(private.join("marker"), b"owned-private-marker").unwrap();
    let public = outside.path.join("public");
    fs::write(&public, b"outside-public-marker").unwrap();
    let alias = outside.path.join("public-alias");
    symlink(&public, &alias).unwrap();
    let private_alias = outside.path.join("private-alias");
    symlink(&private, &private_alias).unwrap();
    let mut state = test_state(root.path.clone());
    state.policy.allow_path_outside_workspace = true;
    state.skill_rt.host_filesystem =
        Arc::new(HostFilesystemPolicy::pin_directories(&[private], &root.path).unwrap());
    assert!(
        execute_builtin_skill(&state, "read_file", &json!({"path":alias}))
            .await
            .is_err()
    );
    let key = "rk-host-filesystem-admin";
    assert!(
        execute_builtin_skill(&state, "list_dir", &json!({"path":outside.path}))
            .await
            .is_err()
    );
    state.seed_test_auth_identity(key, "admin");
    let identity = crate::resolve_auth_identity_by_key(&state, key)
        .unwrap()
        .unwrap();
    let mut payload = json!({});
    crate::task_execution_policy::stamp_authenticated_submission_policy(
        &mut payload,
        Some(&identity),
        Some("ui"),
        None,
    )
    .unwrap();
    let task = crate::ClaimedTask {
        claim_attempt: 1,
        task_id: "host-filesystem-read".to_string(),
        user_id: identity.user_id,
        chat_id: identity.chat_id,
        user_key: Some(key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    };
    let output = super::super::execute_builtin_skill_for_task(
        &state,
        &task,
        "read_file",
        &json!({"path":alias}),
    )
    .await
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap()["content"],
        "outside-public-marker"
    );
    assert!(super::super::execute_builtin_skill_for_task(
        &state,
        &task,
        "read_file",
        &json!({"path":private_alias.join("marker")})
    )
    .await
    .is_err());
    let listing = super::super::execute_builtin_skill_for_task(
        &state,
        &task,
        "list_dir",
        &json!({"path":outside.path}),
    )
    .await
    .unwrap();
    assert!(serde_json::from_str::<Value>(&listing).unwrap()["names"]
        .as_array()
        .unwrap()
        .contains(&json!("public")));
    assert!(super::super::execute_builtin_skill_for_task(
        &state,
        &task,
        "list_dir",
        &json!({"path":private_alias}),
    )
    .await
    .is_err());
}
