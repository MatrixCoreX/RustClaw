use super::*;
use serde_json::Value;
use sha2::Digest;

#[tokio::test]
async fn builtin_write_file_delivery_reaches_real_task_artifact_storage() {
    let root = TempDirGuard::new("builtin_generated_attachment");
    let mut state = test_state("zh-CN");
    install_real_registry(&mut state);
    state.skill_rt.workspace_root = root.path().to_path_buf();
    let task = test_task(json!({"kind":"run_skill"}));
    let write = crate::skills::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({
            "path":"验收结果.txt","content":"exact UTF-8 内容\n","deliver_to_user":true
        }),
    )
    .await
    .expect("write and publish");
    let extra = write.extra.unwrap();
    assert_eq!(extra["artifacts"][0]["owner_task_id"], task.task_id);
    let result = json!({"task_journal":{"trace":{"capability_results":[{"status":"ok","data":{"extra":extra}}]}}});
    let materialized = crate::task_artifacts::materialize_task_result_artifacts(
        root.path(),
        &task.task_id,
        &result.to_string(),
    )
    .unwrap();
    let value: Value = serde_json::from_str(&materialized).unwrap();
    let artifacts = value["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0]["filename"], "验收结果.txt");
    assert_eq!(
        artifacts[0]["sha256"],
        format!(
            "{:x}",
            sha2::Sha256::digest("exact UTF-8 内容\n".as_bytes())
        )
    );
}

#[tokio::test]
async fn builtin_write_file_local_temp_workspace_executes_in_isolation_root() {
    let root = TempDirGuard::new("builtin_write_file_local_temp_isolation");
    let mut state = test_state("en");
    state.skill_rt.workspace_root = root.path().to_path_buf();
    install_registry_from_toml(
        &mut state,
        root.path(),
        r#"
[[skills]]
name = "write_file"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
input_schema = { type = "object", properties = { path = { type = "string" }, content = { type = "string" } } }
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", isolation_profile = "local_temp_workspace", network_access = false, filesystem_write = true, external_publish = false, credential_access = false },
]
"#,
        &["write_file"],
    );
    let task = test_task(json!({"kind": "run_skill"}));

    let outcome = crate::skills::run_skill_with_runner_outcome(
        &state,
        &task,
        "write_file",
        json!({"path": "tmp/out.txt", "content": "isolated\n", "deliver_to_user": true}),
    )
    .await
    .expect("isolated write_file outcome");

    assert!(
        !root.path().join("tmp/out.txt").exists(),
        "write_file should not modify the primary workspace"
    );
    let extra = outcome.extra.expect("write_file extra");
    let refs = extra
        .get("artifact_refs")
        .and_then(serde_json::Value::as_array)
        .expect("isolation artifact refs");
    let execution_root = refs[0]
        .get("execution_root")
        .and_then(serde_json::Value::as_str)
        .expect("execution root");
    assert_eq!(refs[0]["profile"], "local_temp_workspace");
    assert_eq!(refs[0]["requires_cleanup"], true);
    assert!(
        Path::new(execution_root).join("tmp/out.txt").exists(),
        "write_file should write inside the isolation root"
    );
    assert!(
        extra
            .get("resolved_path")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|path| path.starts_with(execution_root)),
        "extra: {extra}"
    );
    let artifact = extra["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| {
            artifact
                .pointer("/metadata/producer")
                .and_then(Value::as_str)
                == Some("filesystem_text_delivery")
        })
        .unwrap();
    let snapshot = Path::new(artifact["path"].as_str().unwrap());
    assert!(snapshot.starts_with(root.path().join(".agent-runtime/artifacts")));
    assert!(!snapshot.starts_with(execution_root));
    fs::remove_dir_all(execution_root).unwrap();
    assert_eq!(fs::read_to_string(snapshot).unwrap(), "isolated\n");
    let result = json!({"task_journal":{"trace":{"capability_results":[{"status":"ok","data":{"extra":extra}}]}}});
    let delivered = crate::task_artifacts::materialize_task_result_artifacts(
        root.path(),
        &task.task_id,
        &result.to_string(),
    )
    .unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&delivered).unwrap()["artifacts"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
