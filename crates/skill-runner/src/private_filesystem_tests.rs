use std::fs;
use std::os::unix::fs::DirBuilderExt;

use super::*;

fn request_value() -> Value {
    serde_json::json!({
        "request_id": "private-policy-fixture",
        "user_id": 1,
        "chat_id": 1,
        "skill_name": "fixture",
        "expected_skill_version": "1.0.0",
        "expected_manifest_digest": "a".repeat(64),
        "expected_receipt_digest": "b".repeat(64),
        "expected_registry_generation": 1,
        "args": {},
        "context": null,
        "host_private_directories": [],
    })
}

fn installed_launch(workspace: &Path, private: &[PathBuf]) -> ChildLaunch {
    let mut launch = ChildLaunch::legacy("/bin/sh");
    launch.installed = true;
    launch.strict_protocol = true;
    launch.working_directory = Some(workspace.to_path_buf());
    launch.host_private_directories = private.to_vec();
    launch
}

#[test]
fn host_policy_is_required_typed_and_never_taken_from_skill_args_or_context() {
    let value = request_value();
    let request: SkillRequest = serde_json::from_value(value.clone()).unwrap();
    assert!(request.host_private_directories.is_empty());
    for invalid in [Value::Null, serde_json::json!({}), serde_json::json!([1])] {
        let mut value = value.clone();
        value["host_private_directories"] = invalid;
        assert!(serde_json::from_value::<SkillRequest>(value).is_err());
    }
    let mut value = value;
    value
        .as_object_mut()
        .unwrap()
        .remove("host_private_directories");
    value["args"] = serde_json::json!({"host_private_directories": []});
    value["context"] = serde_json::json!({"host_private_directories": []});
    assert!(serde_json::from_value::<SkillRequest>(value.clone()).is_err());
    value["host_private_directories"] = serde_json::json!(["/host-owned-fixture"]);
    let request: SkillRequest = serde_json::from_value(value).unwrap();
    assert_eq!(
        request.host_private_directories,
        [PathBuf::from("/host-owned-fixture")]
    );
}

#[test]
fn child_protocol_projects_only_skill_fields_without_host_policy_or_binding() {
    let mut value = request_value();
    value["host_private_directories"] = serde_json::json!(["/host-owned-fixture"]);
    value["args"] = serde_json::json!({"host_private_directories": ["user-data-not-policy"]});
    let request: SkillRequest = serde_json::from_value(value).unwrap();
    let payload = child_request_payload(&request);
    assert_eq!(payload.as_object().unwrap().len(), 6);
    for name in [
        "request_id",
        "args",
        "context",
        "user_id",
        "chat_id",
        "user_key",
    ] {
        assert!(payload.get(name).is_some());
    }
    assert!(payload.get("host_private_directories").is_none());
    assert!(payload.get("expected_receipt_digest").is_none());
    assert!(!payload.to_string().contains("/host-owned-fixture"));
    assert_eq!(payload["args"], request.args);
}

#[tokio::test]
async fn installed_child_rebuilds_exclusions_per_invocation_with_real_access_controls() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    fs::create_dir(&workspace).unwrap();
    fs::write(workspace.join("public"), b"public-marker").unwrap();
    let private: Vec<_> = ["first", "second"]
        .map(|name| {
            let path = root.path().join(name);
            fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
            fs::write(path.join("marker"), b"non-secret-marker").unwrap();
            path.canonicalize().unwrap()
        })
        .into();
    for index in 0..2 {
        let mut launch = installed_launch(&workspace, &[private[index].clone()]);
        launch.args = vec!["-c".into(), "set -eu; test \"$(cat \"$1/public\")\" = public-marker; if cat \"$2/marker\"; then exit 42; fi; test \"$(cat \"$3/marker\")\" = non-secret-marker; printf '%s' visible-control".into(), "fixture".into(), workspace.display().to_string(), private[index].display().to_string(), private[1-index].display().to_string()];
        let output = child_process_command_for_parent(&launch, None, None)
            .unwrap()
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"visible-control");
    }
    let mut launch = installed_launch(&workspace, &[]);
    launch.program = PathBuf::from("/bin/cat");
    launch.args = vec![private[0].join("marker").display().to_string()];
    let output = child_process_command_for_parent(&launch, None, None)
        .unwrap()
        .output()
        .await
        .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"non-secret-marker");
}

#[test]
fn inherited_boundary_requires_matching_policy_and_never_allows_direct_private_execution() {
    let root = tempfile::tempdir().unwrap();
    let launch = installed_launch(root.path(), &[root.path().join("private")]);
    let digest = skill_sdk::private_directories_digest(&launch.host_private_directories).unwrap();
    for backend in ["bubblewrap", "macos_seatbelt"] {
        for digest in [None, Some("wrong-policy")] {
            assert_eq!(
                child_process_command_for_parent(&launch, Some(backend), digest).unwrap_err(),
                "sandbox_parent_private_directories_mismatch"
            );
        }
        let command =
            child_process_command_for_parent(&launch, Some(backend), Some(&digest)).unwrap();
        assert_eq!(command.as_std().get_program(), launch.program.as_os_str());
    }
    assert_eq!(
        child_process_command_for_parent(&launch, Some("direct"), Some(&digest)).unwrap_err(),
        "sandbox_private_directories_require_isolation"
    );
    let mut uninstalled = launch;
    uninstalled.installed = false;
    assert_eq!(
        child_process_command_for_parent(&uninstalled, None, None).unwrap_err(),
        "sandbox_private_directories_require_isolation"
    );
}

#[test]
fn inherited_boundary_rejects_a_pre_anchor_policy_claim_before_child_preparation() {
    let root = tempfile::tempdir().unwrap();
    // Fixed v1 vector from the SDK's independently calculated regression.
    // This tests claim consistency only, not actual filesystem admission.
    let launch = installed_launch(
        root.path(),
        &[PathBuf::from("/owned-policy-fixture/private")],
    );
    let old = "016a924c5268ac972c2774d8cb5d6cf9ec8db66c2ac13ea1035d5b6f3b1f66c2";
    for backend in ["bubblewrap", "macos_seatbelt"] {
        assert_eq!(
            child_process_command_for_parent(&launch, Some(backend), Some(old)).unwrap_err(),
            "sandbox_parent_private_directories_mismatch"
        );
    }
}

#[tokio::test]
async fn package_environment_cannot_forward_host_boundary_claims() {
    let root = tempfile::tempdir().unwrap();
    let mut launch = installed_launch(root.path(), &[]);
    for name in [
        PARENT_PRIVATE_DIRECTORIES_DIGEST_ENV,
        PARENT_SANDBOX_BACKEND_ENV,
    ] {
        launch
            .environment
            .insert(name.into(), "package-claim".into());
        launch.environment_allowlist.push(name.into());
    }
    launch.args = vec!["-c".into(), "test -z \"${APP_PARENT_PRIVATE_DIRECTORIES_DIGEST+set}\" && test -z \"${APP_PARENT_SANDBOX_BACKEND+set}\" && printf claims-absent".into()];
    let mut command = child_process_command_for_parent(&launch, Some("direct"), None).unwrap();
    let environment: BTreeMap<_, _> = command.as_std().get_envs().collect();
    for name in [
        PARENT_PRIVATE_DIRECTORIES_DIGEST_ENV,
        PARENT_SANDBOX_BACKEND_ENV,
    ] {
        assert!(environment
            .get(std::ffi::OsStr::new(name))
            .is_none_or(|value| value.is_none()));
    }
    let output = command.output().await.unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, b"claims-absent");
}
