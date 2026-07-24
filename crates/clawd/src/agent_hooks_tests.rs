use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

use super::{
    default_pre_tool_use_outcome, execute_command_handler, lifecycle_hook_event,
    merge_hook_decision, parse_handler_output, pre_tool_hook_event, validate_command_handler,
    HookHandlerConfig, HookStage,
};
use crate::policy_decision::PolicyDecision;

struct TempHookRoot {
    path: PathBuf,
}

impl TempHookRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw-hook-runtime-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(path.join("hooks")).expect("create hook root");
        Self { path }
    }

    fn write_executable(&self, name: &str, body: &str) -> String {
        let path = self.path.join("hooks").join(name);
        std::fs::write(&path, body).expect("write hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = std::fs::metadata(&path)
                .expect("hook metadata")
                .permissions();
            permissions.set_mode(0o700);
            std::fs::set_permissions(&path, permissions).expect("make hook executable");
        }
        sha256_label(body.as_bytes())
    }
}

impl Drop for TempHookRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn sha256_label(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

fn command_handler(path: &Path, content_sha256: &str) -> HookHandlerConfig {
    HookHandlerConfig {
        id: "fixture_guard".to_string(),
        stage: "pre_tool_use".to_string(),
        kind: "command".to_string(),
        enabled: true,
        trusted: true,
        blocking: true,
        path: path.to_string_lossy().to_string(),
        content_sha256: content_sha256.to_string(),
        timeout_ms: 500,
        max_input_bytes: 4096,
        max_output_bytes: 4096,
        max_attempts: 1,
        failure_policy: "deny".to_string(),
        args: Vec::new(),
        ..HookHandlerConfig::default()
    }
}

#[test]
fn pre_tool_use_hook_allows_without_configured_handlers() {
    let outcome = default_pre_tool_use_outcome("filesystem.list_entries");

    assert_eq!(outcome.stage, "pre_tool_use");
    assert_eq!(outcome.decision, "allow");
    assert_eq!(outcome.reason_code, "pre_tool_use_allowed");
    assert_eq!(outcome.action_ref, "filesystem.list_entries");
}

#[test]
fn post_tool_use_hook_records_machine_status_reason() {
    let outcome =
        super::post_tool_use_outcome("video_generate", &json!({"action": "poll"}), "error");

    assert_eq!(outcome.stage, "post_tool_use");
    assert_eq!(outcome.decision, "allow");
    assert_eq!(outcome.reason_code, "post_tool_use_error");
    assert_eq!(outcome.action_ref, "video_generate.poll");
}

#[test]
fn stop_hook_records_final_status_reason() {
    let outcome = super::stop_outcome("resume_failure");

    assert_eq!(outcome.stage, "stop");
    assert_eq!(outcome.decision, "allow");
    assert_eq!(outcome.reason_code, "stop_resume_failure");
    assert_eq!(outcome.action_ref, "agent_loop.stop");
}

#[test]
fn session_and_prompt_hooks_use_machine_action_refs() {
    let session_start = super::session_start_outcome();
    let session_end = super::session_end_outcome("success");
    let prompt = super::user_prompt_submit_outcome();

    assert_eq!(session_start.stage, "session_start");
    assert_eq!(session_start.action_ref, "agent_loop.session_start");
    assert_eq!(session_end.reason_code, "session_end_success");
    assert_eq!(session_end.action_ref, "agent_loop.session_end");
    assert_eq!(prompt.stage, "user_prompt_submit");
    assert_eq!(prompt.action_ref, "agent_loop.user_prompt_submit");
}

#[test]
fn hook_stage_contract_exposes_all_versioned_lifecycle_events() {
    assert_eq!(
        HookStage::all()
            .iter()
            .map(|stage| stage.as_token())
            .collect::<Vec<_>>(),
        vec![
            "session_start",
            "user_prompt_submit",
            "pre_tool_use",
            "permission_request",
            "post_tool_use",
            "pre_compact",
            "post_compact",
            "subagent_start",
            "subagent_stop",
            "stop",
            "session_end",
        ]
    );
}

#[test]
fn pre_tool_event_exposes_machine_shape_without_argument_values() {
    let event = pre_tool_hook_event(
        "task-1",
        "run_cmd",
        &json!({
            "action": "run",
            "command": "secret-command-value",
            "api_key": "secret-token-value",
            "非机器字段": "must-not-appear"
        }),
        "run_cmd.run",
    );
    let raw = event.to_string();

    assert_eq!(event["schema_version"], 1);
    assert_eq!(event["event_type"], "pre_tool_use");
    assert_eq!(event["argument_count"], 4);
    assert_eq!(
        event["argument_fields"],
        json!(["action", "api_key", "command"])
    );
    assert!(!raw.contains("secret-command-value"));
    assert!(!raw.contains("secret-token-value"));
    assert!(!raw.contains("非机器字段"));
}

#[test]
fn lifecycle_event_drops_semantic_and_secret_metadata_fields() {
    let event = lifecycle_hook_event(
        HookStage::SessionStart,
        "task-1",
        "agent_loop.session_start",
        json!({
            "task_kind": "ask",
            "user_prompt": "must-not-appear",
            "final_answer": "must-not-appear",
            "api_key": "must-not-appear",
            "access_token": "must-not-appear",
            "status": "human sentence must not appear",
            "nested": {"safe_key": "also not machine prose"},
            "非机器字段": "must-not-appear"
        }),
    );
    let raw = event.to_string();

    assert_eq!(event["metadata"]["task_kind"], "ask");
    assert!(!raw.contains("must-not-appear"));
    assert!(!raw.contains("human sentence"));
    assert!(!raw.contains("machine prose"));
    assert!(event["metadata"]
        .as_object()
        .is_some_and(|value| value.len() == 1));
}

#[test]
fn blocking_handler_is_limited_to_decision_capable_stages() {
    let mut handler = HookHandlerConfig {
        id: "fixture_session_observer".to_string(),
        stage: "session_start".to_string(),
        kind: "command".to_string(),
        enabled: true,
        trusted: true,
        blocking: true,
        ..HookHandlerConfig::default()
    };
    assert_eq!(
        validate_command_handler(Path::new("."), handler.clone())
            .expect_err("session observer cannot block")
            .1,
        "hook_handler_blocking_stage_invalid"
    );

    handler.blocking = false;
    assert_ne!(
        validate_command_handler(Path::new("."), handler)
            .expect_err("missing command path must still fail")
            .1,
        "hook_handler_blocking_stage_invalid"
    );
}

#[tokio::test]
async fn trusted_hash_bound_command_hook_returns_structured_decision() {
    let root = TempHookRoot::new();
    let body = "#!/bin/sh\nIFS= read -r _event\nprintf '%s\\n' '{\"schema_version\":1,\"decision\":\"require_confirmation\",\"reason_code\":\"fixture_review_required\"}'\n";
    let hash = root.write_executable("guard.sh", body);
    let mut config = command_handler(Path::new("hooks/guard.sh"), &hash);
    config.timeout_ms = 2_000;
    let handler = validate_command_handler(&root.path, config).expect("validated hook");

    let result = execute_command_handler(
        &handler,
        &root.path,
        &json!({"schema_version": 1, "event_type": "pre_tool_use"}),
        CancellationToken::new(),
        claw_core::config::ToolSandboxMode::DangerFull,
        claw_core::config::ToolSandboxBackend::Auto,
    )
    .await;

    assert_eq!(result.status, "ok");
    assert_eq!(result.decision, PolicyDecision::RequireConfirmation);
    assert_eq!(result.reason_code, "fixture_review_required");
    assert!(result.error_code.is_none());
}

#[test]
fn changed_or_untrusted_command_hook_fails_validation_before_execution() {
    let root = TempHookRoot::new();
    let original = "#!/bin/sh\nprintf '%s\\n' '{\"schema_version\":1,\"decision\":\"allow\",\"reason_code\":\"fixture_allowed\"}'\n";
    let hash = root.write_executable("guard.sh", original);
    root.write_executable("guard.sh", &format!("{original}# changed\n"));

    let changed = validate_command_handler(
        &root.path,
        command_handler(Path::new("hooks/guard.sh"), &hash),
    )
    .expect_err("changed hook must fail");
    assert_eq!(changed.1, "hook_handler_hash_mismatch");

    let mut untrusted = command_handler(Path::new("hooks/guard.sh"), &hash);
    untrusted.trusted = false;
    let untrusted =
        validate_command_handler(&root.path, untrusted).expect_err("untrusted hook must fail");
    assert_eq!(untrusted.1, "hook_handler_untrusted");
}

#[tokio::test]
async fn slow_command_hook_times_out_with_fail_closed_decision() {
    let root = TempHookRoot::new();
    let body = "#!/bin/sh\nIFS= read -r _event\nsleep 1\nprintf '%s\\n' '{\"schema_version\":1,\"decision\":\"allow\",\"reason_code\":\"late_allow\"}'\n";
    let hash = root.write_executable("slow.sh", body);
    let mut config = command_handler(Path::new("hooks/slow.sh"), &hash);
    config.timeout_ms = 20;
    let handler = validate_command_handler(&root.path, config).expect("validated hook");

    let result = execute_command_handler(
        &handler,
        &root.path,
        &json!({"schema_version": 1, "event_type": "pre_tool_use"}),
        CancellationToken::new(),
        claw_core::config::ToolSandboxMode::DangerFull,
        claw_core::config::ToolSandboxBackend::Auto,
    )
    .await;

    assert_eq!(result.decision, PolicyDecision::Deny);
    assert_eq!(result.error_code, Some("hook_handler_timeout"));
    assert!(Duration::from_millis(result.duration_ms) < Duration::from_secs(1));
}

#[tokio::test]
async fn command_hook_cancellation_stops_the_child_and_fails_closed() {
    let root = TempHookRoot::new();
    let body = "#!/bin/sh\nIFS= read -r _event\nsleep 1\nprintf '%s\\n' '{\"schema_version\":1,\"decision\":\"allow\",\"reason_code\":\"late_allow\"}'\n";
    let hash = root.write_executable("cancel.sh", body);
    let handler = validate_command_handler(
        &root.path,
        command_handler(Path::new("hooks/cancel.sh"), &hash),
    )
    .expect("validated hook");
    let cancellation = CancellationToken::new();
    let cancel = cancellation.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel();
    });

    let result = execute_command_handler(
        &handler,
        &root.path,
        &json!({"schema_version": 1, "event_type": "pre_tool_use"}),
        cancellation,
        claw_core::config::ToolSandboxMode::DangerFull,
        claw_core::config::ToolSandboxBackend::Auto,
    )
    .await;

    assert_eq!(result.decision, PolicyDecision::Deny);
    assert_eq!(result.error_code, Some("hook_handler_cancelled"));
    assert!(Duration::from_millis(result.duration_ms) < Duration::from_secs(1));
}

#[test]
fn command_hook_output_rejects_semantic_rewrite_fields_and_merges_conservatively() {
    let output = br#"{"schema_version":1,"decision":"allow","reason_code":"fixture_allowed","final_answer":"rewritten"}"#;
    assert_eq!(
        parse_handler_output(output, true).expect_err("extra semantic field must fail"),
        "hook_handler_output_schema_invalid"
    );

    let mut outcome = default_pre_tool_use_outcome("filesystem.read_text");
    merge_hook_decision(
        &mut outcome,
        PolicyDecision::BackgroundWait,
        "fixture_background".to_string(),
    );
    merge_hook_decision(
        &mut outcome,
        PolicyDecision::RequireConfirmation,
        "fixture_confirmation".to_string(),
    );
    merge_hook_decision(
        &mut outcome,
        PolicyDecision::Deny,
        "fixture_denied".to_string(),
    );
    assert_eq!(outcome.decision, "deny");
    assert_eq!(outcome.reason_code, "fixture_denied");
}
