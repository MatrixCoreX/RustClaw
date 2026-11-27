use super::*;
use std::os::unix::fs::{symlink, PermissionsExt};

#[test]
fn personal_skill_config_is_a_single_reviewed_file_and_rejects_links() {
    let tmp = tempfile::tempdir().unwrap();
    let mut host = AppState::test_default_with_fixture_provider();
    host.skill_rt.workspace_root = tmp.path().to_owned();
    host.skill_rt.principal_workspace = Some(tmp.path().join("own"));
    let mut config = claw_core::config::ToolsConfig::default();
    config.operator_skill_configs = [("fixture_reader".into(), "configs/feeds.toml".into())].into();
    host.skill_rt.tools_policy =
        std::sync::Arc::new(crate::ToolsPolicy::from_config(&config).unwrap());
    let configs = tmp.path().join("configs");
    std::fs::create_dir(&configs).unwrap();
    std::fs::write(configs.join("feeds.toml"), "[feeds]").unwrap();
    assert_eq!(
        host.granted_operator_config("fixture_reader").unwrap(),
        Some(configs.join("feeds.toml").canonicalize().unwrap())
    );
    assert!(host.granted_operator_config("ungranted").unwrap().is_none());
    std::fs::remove_file(configs.join("feeds.toml")).unwrap();
    std::fs::write(tmp.path().join("private"), "fixture").unwrap();
    symlink(tmp.path().join("private"), configs.join("feeds.toml")).unwrap();
    assert!(host.granted_operator_config("fixture_reader").is_err());
}

#[test]
fn authenticated_accounts_get_distinct_workspaces_with_stable_ownership() {
    let tmp = tempfile::tempdir().unwrap();
    let mut host = AppState::test_default_with_fixture_provider();
    host.skill_rt.workspace_root = tmp.path().to_owned();
    for (key, role) in [
        ("account-first", "user"),
        ("account-second", "guest"),
        ("account-admin", "admin"),
    ] {
        host.seed_test_auth_identity(key, role);
    }
    let task = |key: &str| ClaimedTask {
        claim_attempt: 1,
        task_id: "workspace-test".into(),
        user_id: 7,
        chat_id: 9,
        user_key: Some(key.into()),
        channel: "ui".into(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".into(),
        payload_json: "{}".into(),
    };
    let first = host
        .scope_task_workspace(&task("account-first"))
        .unwrap()
        .unwrap();
    let second = host
        .scope_task_workspace(&task("account-second"))
        .unwrap()
        .unwrap();
    let document = first.skill_rt.files_root().join("report.txt");
    std::fs::write(&document, "owned").unwrap();
    assert!(first
        .skill_rt
        .host_filesystem
        .open_file(Some(first.skill_rt.files_root()), &document)
        .is_ok());
    assert!(second
        .skill_rt
        .host_filesystem
        .open_file(None, &document)
        .is_err());
    assert!(first.scope_task_workspace(&task("account-second")).is_err());
    assert!(host
        .scope_task_workspace(&task("account-admin"))
        .unwrap()
        .is_none());
    assert!(host
        .scope_task_workspace(&task("account-no-longer-active"))
        .is_err());
}

#[cfg(target_os = "linux")]
#[test]
fn command_sees_only_its_workspace_and_approved_runtime_files() {
    let tmp = tempfile::tempdir().unwrap();
    let first = directory(tmp.path(), "first").unwrap();
    let second = directory(tmp.path(), "second").unwrap();
    std::fs::write(second.join("report.txt"), "other").unwrap();
    let approved = tmp.path().join("settings.txt");
    std::fs::write(&approved, "approved").unwrap();
    let grants = [approved.clone()];
    let prepared = crate::process_sandbox::prepare_process_invocation(
        "/bin/sh",
        crate::process_sandbox::ProcessSandboxRequest {
            principal_readonly: Some(&grants),
            workspace_root: &first,
            execution_root: &first,
            mode: claw_core::config::ToolSandboxMode::DangerFull,
            backend: claw_core::config::ToolSandboxBackend::Auto,
            network: crate::process_sandbox::ProcessNetworkPolicy::Deny,
            additional_writable_paths: &[],
            host_private_directories: &[],
            browser_subprocess: false,
        },
    )
    .unwrap();
    let status = prepared
        .invocation
        .into_command()
        .args([
            "-c",
            "test ! -e \"$1\" && test -r \"$2\" && printf mine > report.txt",
            "check",
        ])
        .arg(second.join("report.txt"))
        .arg(&approved)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(std::fs::read(first.join("report.txt")).unwrap(), b"mine");
    assert_eq!(std::fs::read(second.join("report.txt")).unwrap(), b"other");
}

#[test]
fn workspace_is_stable_private_and_never_resolves_a_redirected_directory() {
    let root = tempfile::tempdir().unwrap();
    let first = directory(root.path(), "one").unwrap();
    let second = directory(root.path(), "../two").unwrap();
    assert_ne!(first, second);
    assert_eq!(first, directory(root.path(), "one").unwrap());
    assert_eq!(
        first.metadata().unwrap().permissions().mode() & 0o777,
        0o700
    );
    std::fs::write(first.join("a.txt"), b"one").unwrap();
    assert!(!second.join("a.txt").exists());
    std::fs::remove_dir(&second).unwrap();
    symlink(&first, &second).unwrap();
    assert!(directory(root.path(), "../two").is_err());
}
