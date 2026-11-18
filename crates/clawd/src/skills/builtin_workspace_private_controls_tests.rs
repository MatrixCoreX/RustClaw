use super::*;
use std::os::unix::fs::symlink;

#[tokio::test]
async fn host_private_mutation_public_workflow_preserves_append_modes_and_rewind() {
    let root = TempDirGuard::new("private_mutation_public_controls");
    let private = private_directory(&root.path);
    let state = private_state(&root.path, &private);
    fs::write(root.path.join("public"), b"before").unwrap();
    fs::set_permissions(root.path.join("public"), fs::Permissions::from_mode(0o750)).unwrap();
    let appended = execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path":"./public", "content":"after\n", "append":true, "create_parents":false
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        fs::read(root.path.join("public")).unwrap(),
        b"before\nafter\n"
    );
    assert_eq!(
        fs::metadata(root.path.join("public"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o750
    );
    let checkpoint: Value = serde_json::from_str(&appended).unwrap();
    execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({
            "action":"rewind", "checkpoint_id":checkpoint["checkpoint_id"]
        }),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(root.path.join("public")).unwrap(), b"before");
    assert!(execute_builtin_skill(
        &state,
        "write_file",
        &json!({
            "path":"absent/file", "content":"new", "create_parents":false
        })
    )
    .await
    .is_err());
    assert!(!root.path.join("absent").exists());
    execute_builtin_skill(&state, "make_dir", &json!({"path":"."}))
        .await
        .unwrap();
    execute_builtin_skill(
        &state,
        "make_dir",
        &json!({"path":"new/nested", "parents":true}),
    )
    .await
    .unwrap();
    execute_builtin_skill(
        &state,
        "remove_file",
        &json!({"path":"new", "target_kind":"directory", "recursive":true}),
    )
    .await
    .unwrap();
    assert!(!root.path.join("new").exists());
    let victim = root.path.join("victim");
    fs::write(&victim, b"original").unwrap();
    symlink(&victim, root.path.join("link")).unwrap();
    assert!(execute_builtin_skill(
        &state,
        "write_file",
        &json!({"path":"./link", "content":"no"})
    )
    .await
    .is_err());
    assert_eq!(fs::read(victim).unwrap(), b"original");
}

#[tokio::test]
async fn host_private_mutation_directory_dispatch_refuses_private_descendants() {
    let root = TempDirGuard::new("private_mutation_directory_controls");
    let parent = root.path.join("parent");
    fs::create_dir(&parent).unwrap();
    let private = private_directory(&parent);
    fs::write(parent.join("public"), b"public").unwrap();
    fs::write(private.join("marker"), b"owned").unwrap();
    let state = private_state(&root.path, &private);
    for args in [
        json!({"path":"parent/private"}),
        json!({"path":"parent/private/new", "parents":true}),
    ] {
        assert!(execute_builtin_skill(&state, "make_dir", &args)
            .await
            .is_err());
    }
    assert!(execute_builtin_skill(
        &state,
        "remove_file",
        &json!({
            "path":"parent", "target_kind":"directory", "recursive":true
        })
    )
    .await
    .is_err());
    assert_eq!(fs::read(parent.join("public")).unwrap(), b"public");
    assert_eq!(fs::read(private.join("marker")).unwrap(), b"owned");
    assert!(!private.join("new").exists());
}

#[tokio::test]
async fn host_private_mutation_outside_write_requires_admin_and_still_refuses_private_data() {
    let root = TempDirGuard::new("private_mutation_outside_controls");
    let outside = TempDirGuard::new("private_mutation_outside_targets");
    let private = private_directory(&outside.path);
    let mut state = private_state(&root.path, &private);
    state.policy.allow_path_outside_workspace = true;
    let path = outside.path.join("public/new");
    assert!(
        execute_builtin_skill(&state, "write_file", &json!({"path":path, "content":"no"}))
            .await
            .is_err()
    );
    let key = "rk-native-mutation-fixture";
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
        task_id: "native-mutation-fixture".into(),
        user_id: identity.user_id,
        chat_id: identity.chat_id,
        user_key: Some(key.into()),
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: payload.to_string(),
    };
    super::super::super::execute_builtin_skill_for_task(
        &state,
        &task,
        "write_file",
        &json!({
            "path":path, "content":"outside", "create_parents":true
        }),
    )
    .await
    .unwrap();
    assert_eq!(fs::read(&path).unwrap(), b"outside");
    assert!(super::super::super::execute_builtin_skill_for_task(
        &state,
        &task,
        "write_file",
        &json!({
            "path":private.join("marker"), "content":"no", "create_parents":true
        })
    )
    .await
    .is_err());
    assert!(!private.join("marker").exists());
}

#[tokio::test]
async fn host_private_mutation_rewind_rejects_corrupt_backups_before_deleting_current_data() {
    let root = TempDirGuard::new("private_mutation_backup_controls");
    let private = private_directory(&root.path);
    let state = private_state(&root.path, &private);
    fs::write(root.path.join("public"), b"before").unwrap();
    let applied = execute_builtin_skill(
        &state,
        "write_file",
        &json!({"path":"./public", "content":"after"}),
    )
    .await
    .unwrap();
    let checkpoint: Value = serde_json::from_str(&applied).unwrap();
    let directory = claw_core::workspace_state::workspace_state_root(&root.path)
        .join("checkpoints")
        .join(checkpoint["checkpoint_id"].as_str().unwrap());
    fs::write(directory.join("before/0000.bin"), b"corrupt-owned-backup").unwrap();
    let error = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({"action":"rewind", "checkpoint_id":checkpoint["checkpoint_id"]}),
    )
    .await
    .unwrap_err();
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "checkpoint_backup_invalid"
    );
    assert_eq!(fs::read(root.path.join("public")).unwrap(), b"after");
}

#[tokio::test]
async fn host_private_mutation_moved_target_and_transaction_policy_are_not_bypassed() {
    let root = TempDirGuard::new("private_mutation_moved_controls");
    let private = private_directory(&root.path);
    fs::write(private.join("marker"), b"before\n").unwrap();
    let state = private_state(&root.path, &private);
    let moved = root.path.join("moved");
    fs::rename(&private, &moved).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&private).unwrap();
    assert!(execute_builtin_skill(&state, "workspace_patch", &json!({
        "action":"preview_replace_text", "path":"moved/marker", "old_text":"before", "new_text":"after"
    })).await.is_err());
    assert!(execute_builtin_skill(
        &state,
        "write_file",
        &json!({"path":"moved/marker", "content":"no"})
    )
    .await
    .is_err());
    assert_eq!(fs::read(moved.join("marker")).unwrap(), b"before\n");

    let other = TempDirGuard::new("private_transaction_controls");
    let state_root = claw_core::workspace_state::workspace_state_root(&other.path);
    fs::create_dir_all(&state_root).unwrap();
    fs::set_permissions(&state_root, fs::Permissions::from_mode(0o700)).unwrap();
    let state = private_state(&other.path, &state_root);
    let result = execute_builtin_skill(&state, "workspace_patch", &json!({
        "action":"apply_patch_shard", "transaction_id":"private-fixture", "shard_index":0, "shard_count":1,
        "patch":"--- /dev/null\n+++ b/new\n@@ -0,0 +1 @@\n+new\n"
    })).await;
    let error = result.unwrap_err();
    assert_eq!(
        crate::skills::parse_structured_skill_error(&error)
            .unwrap()
            .error_code,
        "host_private_directory_access_denied"
    );
    assert!(!other.path.join("new").exists());
    assert_eq!(fs::read_dir(state_root).unwrap().count(), 0);
}

#[tokio::test]
async fn host_private_mutation_non_git_patch_needs_no_process_backend() {
    let root = TempDirGuard::new("private_mutation_native_patch_controls");
    let private = private_directory(&root.path);
    let mut state = private_state(&root.path, &private);
    let mut policy = ToolsPolicy::from_config(&ToolsConfig::default()).unwrap();
    #[cfg(target_os = "linux")]
    {
        policy.sandbox_backend = claw_core::config::ToolSandboxBackend::MacosSeatbelt;
    }
    #[cfg(target_os = "macos")]
    {
        policy.sandbox_backend = claw_core::config::ToolSandboxBackend::Bubblewrap;
    }
    state.skill_rt.tools_policy = Arc::new(policy);
    fs::write(root.path.join("public"), b"before\n").unwrap();
    let result = execute_builtin_skill(
        &state,
        "workspace_patch",
        &json!({
            "action":"apply_patch", "patch":patch("public")
        }),
    )
    .await
    .unwrap();
    let result: Value = serde_json::from_str(&result).unwrap();
    assert_eq!(result["patch_engine"], "pure_rust");
    assert_eq!(fs::read(root.path.join("public")).unwrap(), b"after\n");
}
