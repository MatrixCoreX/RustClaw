use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use super::{
    admitted_extra_field_exists, build_auto_sudo_retry_args,
    contains_unresolved_runtime_template_arg, contract_matrix_action_policy_error,
    contract_matrix_arg_policy_error, handle_skill_step_failure, handle_skill_step_success,
    merge_isolation_artifact_refs, preflight_failure_metadata, record_subagent_step_execution,
    skill_extra_requests_user_input, structured_extra_evidence_output,
    structured_observation_path_argument_error, try_auto_sudo_retry_after_permission_denied,
    unresolved_runtime_template_argument_error, validate_skill_output_contract,
    AgentLoopGuardPolicy, LoopState,
};
use crate::agent_engine::support::{
    AnswerVerifierRequiredEvidenceScope, RegistryIdempotencyGuardScope, SemanticRouteAuthority,
};
use crate::{
    AgentRuntimeConfig, AppState, ClaimedTask, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID,
};
use claw_core::config::{AgentConfig, ToolsConfig};
use claw_core::skill_registry::SkillsRegistry;
use rusqlite::params;

pub(super) fn test_state() -> AppState {
    let db_pool = crate::db_init::test_pool();
    {
        let db = db_pool.get().expect("get db conn");
        db.execute_batch(
            r#"
            CREATE TABLE tasks (
                task_id TEXT PRIMARY KEY,
                status TEXT NOT NULL,
                result_json TEXT,
                updated_at INTEGER
            );
            INSERT INTO tasks (task_id, status, result_json, updated_at)
            VALUES ('task-skill-exec', 'running', NULL, 0);
            "#,
        )
        .expect("seed tasks");
    }
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            db: db_pool,
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
            locator_scan_max_depth: 3,
            locator_scan_max_files: 200,
            tools_policy: Arc::new(
                ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy"),
            ),
            ..crate::SkillRuntime::test_default()
        },
        policy: crate::PolicyConfig::test_default(),
        worker: crate::WorkerConfig::test_default(),
        metrics: crate::TaskMetricsRegistry::default(),
        channels: crate::ChannelConfig::default(),
        reload_ctx: crate::ReloadContext::default(),
        ask_states: crate::AskStateRegistry::default(),
    }
}

fn test_task() -> ClaimedTask {
    ClaimedTask {
        task_id: "task-skill-exec".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "telegram".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn insert_auth_key(state: &AppState, user_key: &str, role: &str) {
    let db = state.core.db.get().expect("db pool");
    db.execute_batch(crate::KEY_AUTH_UPGRADE_SQL)
        .expect("create auth schema");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
         VALUES (?1, ?2, 1, '123', NULL)",
        params![user_key, role],
    )
    .expect("insert auth key");
}

fn enable_test_skills(state: &AppState, skills: &[&str]) {
    let set = skills
        .iter()
        .map(|skill| skill.to_string())
        .collect::<HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("write skill snapshot") = Arc::new(SkillViewsSnapshot {
        registry: None,
        skills_list: Arc::new(set),
    });
}

pub(super) fn install_test_registry(state: &AppState, raw: &str, skills: &[&str]) {
    let path = std::env::temp_dir().join(format!(
        "rustclaw-skill-execution-test-{}-{}-{}.toml",
        std::process::id(),
        skills.join("-"),
        unique_suffix()
    ));
    fs::write(&path, raw).expect("write registry fixture");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry fixture"));
    let set = skills
        .iter()
        .map(|skill| skill.to_string())
        .collect::<HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("write skill snapshot") = Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(set),
    });
}

fn install_agent_guard_workspace(name: &str, raw: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-skill-exec-agent-guard-{}-{name}",
        std::process::id()
    ));
    let config_dir = root.join("configs");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::write(config_dir.join("agent_guard.toml"), raw).expect("write agent guard");
    root
}

fn admin_task() -> ClaimedTask {
    let mut task = test_task();
    task.user_key = Some("rk-admin".to_string());
    task
}

fn test_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_steps: 16,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 4,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        semantic_route_authority: SemanticRouteAuthority::Legacy,
        agent_loop_canary_bucket: "none".to_string(),
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        structured_evidence_required_for_selected_contracts: false,
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    }
}

#[test]
fn subagent_step_execution_promotes_runtime_observation_to_step_output() {
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.task_observations.push(serde_json::json!({
        "schema_version": 1,
        "owner_layer": "subagent_runtime",
        "status": "accepted",
        "execution_mode": "bounded_parallel_readonly_child_runs",
        "aggregation": {
            "status": "completed",
            "finding_refs": [
                "subagent-batch:2:3:1:explorer",
                "subagent-batch:2:3:2:verifier"
            ]
        },
        "global_step": 7,
        "step_in_round": 3,
        "round_no": 2
    }));

    record_subagent_step_execution(&task, &mut loop_state, 7, 3, "call_tool", None);

    assert_eq!(loop_state.executed_step_results.len(), 1);
    let step = &loop_state.executed_step_results[0];
    assert!(step.is_ok());
    assert_eq!(step.skill, "subagent");
    let output = step.output.as_deref().expect("subagent output");
    let parsed: serde_json::Value = serde_json::from_str(output).expect("machine json");
    assert_eq!(parsed["output_format"], "machine_json");
    assert_eq!(parsed["owner_layer"], "subagent_runtime");
    assert_eq!(
        parsed["execution_mode"],
        "bounded_parallel_readonly_child_runs"
    );
    assert_eq!(
        parsed["aggregation"]["finding_refs"]
            .as_array()
            .expect("finding refs")
            .len(),
        2
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("skill.subagent.last_output")
            .map(String::as_str),
        Some(output)
    );
}

#[test]
fn unresolved_runtime_template_arg_is_detected_structurally() {
    let args = serde_json::json!({
        "action": "stat_paths",
        "paths": "{{s1.paths}}"
    });

    assert!(contains_unresolved_runtime_template_arg(&args));
    let err = unresolved_runtime_template_argument_error("fs_basic", &args, &args)
        .expect("unresolved template should be rejected");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("preflight error should be structured");
    assert_eq!(parsed.error_kind, "invalid_args");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason"))
            .and_then(|value| value.as_str()),
        Some("unresolved_runtime_placeholder")
    );
    assert!(
        !parsed.error_text.contains("{{"),
        "user-facing error text must not leak unresolved templates"
    );
}

#[test]
fn contract_matrix_preflight_rejects_disallowed_action_for_structured_task() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({"command": "ls"});

    let err = contract_matrix_action_policy_error(&state, &loop_state, "run_cmd", &args)
        .expect("contract matrix should reject run_cmd for file_names");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("contract policy error should be structured");

    assert_eq!(parsed.error_kind, "contract_action_rejected");
    assert!(parsed
        .error_text
        .contains("prefer action(s): fs_basic.list_dir"));
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!("contract_action_rejected"))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("decision")),
        Some(&serde_json::json!("rejected_not_allowed"))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("failure_attribution")),
        Some(&serde_json::json!("contract_gap"))
    );
    let permission = parsed
        .extra
        .as_ref()
        .and_then(|extra| extra.get("permission_decision"))
        .expect("permission_decision");
    assert_eq!(permission["allowed"], false);
    assert_eq!(permission["decision"], serde_json::json!("deny"));
    assert_eq!(permission["denied_by_policy"], true);
    assert_eq!(permission["needs_confirmation"], false);
    assert_eq!(permission["dry_run_required"], false);
    assert_eq!(permission["external_provider_blocked"], false);
    assert_eq!(
        permission["owner_layer"],
        serde_json::json!("contract_matrix_preflight")
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("original_action_ref")),
        Some(&serde_json::json!("run_cmd"))
    );
    let metadata = preflight_failure_metadata(&err);
    assert_eq!(metadata.reason, "contract_action_rejected");
    assert_eq!(metadata.error_kind, "contract_action_rejected");
    assert!(metadata
        .retry_instruction
        .contains("contract_policy_decision=rejected_not_allowed"));
    assert!(metadata.retry_instruction.contains("fs_basic.list_dir"));
}

#[test]
fn contract_matrix_preflight_allows_runtime_async_job_start_marker() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "command": "sleep 2 && echo RUSTCLAW_ASYNC_SMOKE",
        "async_start": true,
        "poll_after_seconds": 2,
        "expires_in_seconds": 600,
        super::super::CLAWD_RUNTIME_ASYNC_JOB_START_ARG: "async_job_protocol"
    });

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "run_cmd", &args).is_none(),
        "runtime async job starts are classified by the machine contract before execution"
    );
}

#[test]
fn contract_matrix_preflight_allows_bounded_planner_async_start_without_runtime_marker() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "command": "sleep 2 && echo RUSTCLAW_ASYNC_SMOKE",
        "async_start": true,
        "poll_after_seconds": 2,
        "expires_in_seconds": 600
    });

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "run_cmd", &args).is_none(),
        "complete planner async-start machine fields should keep agent-loop authority even when the normalizer route was generic"
    );
}

#[test]
fn contract_matrix_preflight_rejects_unbounded_async_start_without_runtime_marker() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "command": "sleep 2 && echo RUSTCLAW_ASYNC_SMOKE",
        "async_start": true,
        "poll_after_seconds": 2
    });

    let err = contract_matrix_action_policy_error(&state, &loop_state, "run_cmd", &args)
        .expect("unbounded async starts still need an explicit runtime contract");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("contract policy error should be structured");
    assert_eq!(parsed.error_kind, "contract_action_rejected");
}

#[test]
fn contract_matrix_preflight_allows_registry_observe_config_preview_for_summary() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "config_edit"
enabled = true
kind = "runner"
planner_kind = "skill"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "config.plan_change", action = "plan_config_change", effect = "observe", required = ["field_path", "value"], risk_level = "low", preferred = true, idempotent = true, dedup_scope = "args" },
  { name = "config.apply_change", action = "apply_config_change", effect = "mutate", required = ["field_path", "value"], risk_level = "high", preferred = true, once_per_task = true, idempotent = false, dedup_scope = "action" },
]
"#,
        &["config_edit"],
    );
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::CommandOutputSummary,
        requires_content_evidence: true,
        ..crate::IntentOutputContract::default()
    });
    let preview_args = serde_json::json!({
        "action": "plan_config_change",
        "path": "configs/config.toml",
        "field_path": "llm.selected_vendor",
        "value": "minimax"
    });
    let apply_args = serde_json::json!({
        "action": "apply_config_change",
        "path": "configs/config.toml",
        "field_path": "llm.selected_vendor",
        "value": "minimax"
    });

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "config_edit", &preview_args)
            .is_none()
    );
    let err = contract_matrix_action_policy_error(&state, &loop_state, "config_edit", &apply_args)
        .expect("mutating config apply must still be rejected");
    let parsed =
        crate::skills::parse_structured_skill_error(&err).expect("preflight error is structured");
    assert_eq!(parsed.error_kind, "contract_action_rejected");
}

#[test]
fn contract_matrix_preflight_rejects_generated_media_run_cmd() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::GeneratedFilePathReport,
        requires_content_evidence: true,
        response_shape: crate::OutputResponseShape::Scalar,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "document/rust_icon_pixel_smoke.png".to_string(),
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({"command": "python3 -c 'create image'"});

    let err = contract_matrix_action_policy_error(&state, &loop_state, "run_cmd", &args)
        .expect("media path run_cmd should be rejected");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("contract policy error should be structured");

    assert_eq!(parsed.error_kind, "contract_action_rejected");
    assert_eq!(parsed.error_text, "media_artifact_requires_media_skill");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!("media_artifact_requires_media_skill"))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("policy_decision")),
        Some(&serde_json::json!("deny"))
    );
    let permission = parsed
        .extra
        .as_ref()
        .and_then(|extra| extra.get("permission_decision"))
        .expect("permission_decision");
    assert_eq!(permission["allowed"], false);
    assert_eq!(permission["decision"], serde_json::json!("deny"));
    assert_eq!(permission["denied_by_policy"], true);
    assert_eq!(
        permission["owner_layer"],
        serde_json::json!("run_cmd_media_artifact_preflight")
    );
    assert_eq!(
        permission["reason_code"],
        serde_json::json!("media_artifact_requires_media_skill")
    );
    assert_eq!(
        permission.pointer("/command_policy/policy_authority"),
        Some(&serde_json::json!("planner_structured_args"))
    );
    assert_eq!(
        permission.pointer("/command_policy/literal_command_token"),
        Some(&serde_json::json!(false))
    );
    assert_eq!(
        permission.pointer("/command_policy/command_arg_present"),
        Some(&serde_json::json!(true))
    );
    let metadata = preflight_failure_metadata(&err);
    assert_eq!(metadata.reason, "contract_action_rejected");
    assert!(metadata.retry_instruction.contains("policy_decision=deny"));
    assert!(metadata.retry_instruction.contains("image_edit"));
}

#[test]
fn contract_matrix_preflight_does_not_block_literal_media_run_cmd() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::GeneratedFilePathReport,
        requires_content_evidence: true,
        response_shape: crate::OutputResponseShape::Scalar,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "document/rust_icon_pixel_smoke.png".to_string(),
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "command": "python3 -c 'create image'",
        super::super::CLAWD_LITERAL_COMMAND_ARG: true
    });

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "run_cmd", &args).is_none(),
        "literal user commands should preserve the explicit command policy boundary"
    );
}

#[test]
fn contract_matrix_preflight_permission_decision_uses_registry_policy() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "system.run_command", effect = "external", required = ["command"], risk_level = "high", once_per_task = true, idempotent = false, dedup_scope = "action" },
]
"#,
        &["run_cmd"],
    );
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({"command": "ls"});

    let err = contract_matrix_action_policy_error(&state, &loop_state, "run_cmd", &args)
        .expect("contract matrix should reject run_cmd for file_names");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("contract policy error should be structured");
    let permission = parsed
        .extra
        .as_ref()
        .and_then(|extra| extra.get("permission_decision"))
        .expect("permission_decision");

    assert_eq!(permission["risk_level"], serde_json::json!("high"));
    assert_eq!(permission["decision"], serde_json::json!("deny"));
    assert_eq!(permission["needs_confirmation"], true);
    assert_eq!(permission["action_effect"], serde_json::json!("observe"));
    assert_eq!(permission["canonical_skill"], serde_json::json!("run_cmd"));
    assert_eq!(
        permission.pointer("/command_policy/policy_authority"),
        Some(&serde_json::json!("planner_structured_args"))
    );
    assert_eq!(
        permission.pointer("/command_policy/effect"),
        Some(&serde_json::json!("observe"))
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/available")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/once_per_task")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/dedup_scope")
            .and_then(serde_json::Value::as_str),
        Some("action")
    );
    assert_eq!(
        permission
            .pointer("/registry_policy/idempotent")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/isolation_profile")
            .and_then(serde_json::Value::as_str),
        Some("remote_executor")
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/network_access")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/filesystem_write")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/external_publish")
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert_eq!(
        permission
            .pointer("/capability_policy/credential_access")
            .and_then(serde_json::Value::as_bool),
        Some(false)
    );
}

#[test]
fn contract_matrix_preflight_marks_package_dry_run_as_low_risk_observe() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "package_manager"
enabled = true
kind = "runner"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
confirmation_exempt_when = [
  { action = "smart_install", dry_run = true },
]
planner_capabilities = [
  { name = "package.smart_install_preview", action = "smart_install", effect = "mutate", required = ["package|packages"], optional = ["dry_run"], risk_level = "high", once_per_task = true, idempotent = false, dedup_scope = "action" },
]
"#,
        &["package_manager"],
    );
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "action": "smart_install",
        "packages": ["jq"],
        "dry_run": true
    });

    let err = contract_matrix_action_policy_error(&state, &loop_state, "package_manager", &args)
        .expect("file_names contract should reject package dry-run and expose permission");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("contract policy error should be structured");
    let permission = parsed
        .extra
        .as_ref()
        .and_then(|extra| extra.get("permission_decision"))
        .expect("permission_decision");

    assert_eq!(permission["risk_level"], serde_json::json!("low"));
    assert_eq!(permission["decision"], serde_json::json!("deny"));
    assert_eq!(permission["needs_confirmation"], false);
    assert_eq!(permission["action_effect"], serde_json::json!("observe"));
    assert_eq!(
        permission["canonical_skill"],
        serde_json::json!("package_manager")
    );
}

#[test]
fn contract_matrix_preflight_allows_user_named_output_path_marker() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::RawCommandOutput,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::None,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "path": "pwd_line_abs.txt",
        "content": "/home/guagua/rustclaw\n",
        "_clawd_user_named_output_path": true
    });

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "write_file", &args).is_none(),
        "planner-marked user named output writes must survive execution preflight"
    );
}

#[test]
fn active_ops_recipe_preflight_allows_backing_mutation_despite_summary_contract() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::CommandOutputSummary,
        requires_content_evidence: true,
        response_shape: crate::OutputResponseShape::Scalar,
        ..crate::IntentOutputContract::default()
    });
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::CurrentRepo,
        validation_required: true,
        saw_inspect: true,
        ..Default::default()
    };
    let args = serde_json::json!({
        "path": "document/ops-repair/index.html",
        "content": "ops-repair-ok\n"
    });

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "write_file", &args).is_none(),
        "active ops recipe mutations must not be rejected after virtual tool rewrite"
    );
}

#[test]
fn contract_matrix_preflight_allows_internal_synthesis_actions() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::RecentArtifactsJudgment,
        requires_content_evidence: true,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({"evidence_refs": ["last_output"]});

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "synthesize_answer", &args)
            .is_none(),
        "internal synthesis must not be rejected by observation allowed_actions"
    );
    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "respond", &serde_json::json!({}))
            .is_none(),
        "internal respond must not be rejected by observation allowed_actions"
    );
}

#[test]
fn contract_matrix_preflight_allows_virtual_find_entries_backing_action() {
    let state = test_state();
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "action": "find_ext",
        "root": "plan",
        "ext": "md",
    });

    assert!(
        contract_matrix_action_policy_error(&state, &loop_state, "fs_search", &args).is_none(),
        "runtime backing fs_search calls should be admitted through their planner-facing fs_basic.find_entries contract"
    );
}

#[test]
fn contract_matrix_preflight_rejects_missing_bound_target_arg() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "action": "read_text_range",
        "start_line": 1,
        "end_line": 20
    });

    let err = contract_matrix_arg_policy_error(&loop_state, "fs_basic", &args)
        .expect("contract arg policy should reject missing path");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("contract arg policy error should be structured");

    assert_eq!(parsed.error_kind, "contract_arg_rejected");
    assert!(parsed.error_text.contains("expected target arg(s): path"));
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason_code")),
        Some(&serde_json::json!("contract_arg_rejected"))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("decision")),
        Some(&serde_json::json!("missing_target_binding"))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("policy_decision")),
        Some(&serde_json::json!("deny"))
    );
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("failure_attribution")),
        Some(&serde_json::json!("model_error"))
    );
    let metadata = preflight_failure_metadata(&err);
    assert_eq!(metadata.reason, "contract_arg_rejected");
    assert_eq!(metadata.error_kind, "contract_arg_rejected");
    assert!(metadata
        .retry_instruction
        .contains("contract_policy_decision=missing_target_binding"));
    assert!(metadata.retry_instruction.contains("policy_decision=deny"));
    assert!(metadata.retry_instruction.contains("path"));
}

#[test]
fn contract_matrix_preflight_defers_template_target_to_runtime_placeholder_check() {
    let mut loop_state = LoopState::new(2);
    loop_state.output_contract = Some(crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        ..crate::IntentOutputContract::default()
    });
    let args = serde_json::json!({
        "action": "read_text_range",
        "path": "{{s1.path}}"
    });

    assert!(contract_matrix_arg_policy_error(&loop_state, "fs_basic", &args).is_none());
    assert!(unresolved_runtime_template_argument_error("fs_basic", &args, &args).is_some());
}

#[test]
fn structured_runtime_observation_path_arg_is_rejected_structurally() {
    let args = serde_json::json!({
        "action": "schema_version",
        "db_path": r#"{"action":"find_ext","count":5,"results":["data/app.db","data/claw.db"]}"#,
    });

    let err = structured_observation_path_argument_error("db_basic", &args)
        .expect("structured observation in path should be rejected");
    let parsed = crate::skills::parse_structured_skill_error(&err)
        .expect("preflight error should be structured");
    assert_eq!(parsed.error_kind, "invalid_args");
    assert_eq!(
        parsed
            .extra
            .as_ref()
            .and_then(|extra| extra.get("reason"))
            .and_then(|value| value.as_str()),
        Some("structured_observation_embedded_in_path_arg")
    );
}

#[test]
fn scalar_json_like_filename_path_arg_is_not_rejected() {
    let args = serde_json::json!({
        "action": "read_text_range",
        "path": "docs/{draft}.md",
    });

    assert!(structured_observation_path_argument_error("fs_basic", &args).is_none());
}

#[test]
fn literal_run_cmd_keeps_user_supplied_handlebars_text() {
    let exec_args = serde_json::json!({
        "command": "printf '%s\\n' '{{literal_template}}'",
    });
    let classification_args = serde_json::json!({
        "command": "printf '%s\\n' '{{literal_template}}'",
        "_clawd_literal_command": true,
    });

    assert!(contains_unresolved_runtime_template_arg(&exec_args));
    assert!(unresolved_runtime_template_argument_error(
        "run_cmd",
        &exec_args,
        &classification_args,
    )
    .is_none());
}

#[test]
fn generated_run_cmd_with_unresolved_placeholder_is_rejected() {
    let args = serde_json::json!({
        "command": "wc -c {{s1.path}}",
    });

    assert!(
        unresolved_runtime_template_argument_error("run_cmd", &args, &args).is_some(),
        "generated commands must not execute unresolved runtime placeholders"
    );
}

#[test]
fn auto_sudo_retry_builds_structured_read_range_retry_for_admin_permission_denied() {
    let mut state = test_state();
    state.policy.allow_sudo = true;
    state.skill_rt.workspace_root = PathBuf::from("/tmp/rustclaw-auto-sudo-workspace");
    enable_test_skills(&state, &["run_cmd", "system_basic"]);
    insert_auth_key(&state, "rk-admin", "admin");
    let task = admin_task();
    let restricted_path = "/tmp/rustclaw-auto-sudo-workspace/restricted.log";

    let retry = build_auto_sudo_retry_args(
        &state,
        &task,
        "system_basic",
        Some(&serde_json::json!({
            "action": "read_range",
            "path": restricted_path,
            "n": 1
        })),
        &crate::skills::structured_skill_error_from_parts(
            "system_basic",
            "permission_denied",
            "read_range failed for restricted.log",
            Some("linux"),
            Some(serde_json::json!({
                "operation": "metadata",
                "path": restricted_path
            })),
        ),
    )
    .expect("admin permission denial should trigger sudo retry");

    let command = retry
        .get("command")
        .and_then(|value| value.as_str())
        .expect("retry command");
    assert!(command.starts_with("sudo -n "), "got: {command}");
    assert!(command.contains(restricted_path), "got: {command}");
    assert!(command.contains("sed"), "got: {command}");
    assert!(!command.contains(" -- "), "got: {command}");
    assert!(!command.contains("-printf"), "got: {command}");
}

#[test]
fn auto_sudo_retry_uses_posix_directory_listing_for_cross_platform_hosts() {
    let mut state = test_state();
    state.policy.allow_sudo = true;
    state.skill_rt.workspace_root = PathBuf::from("/tmp/rustclaw-auto-sudo-workspace");
    enable_test_skills(&state, &["run_cmd", "system_basic"]);
    insert_auth_key(&state, "rk-admin", "admin");
    let task = admin_task();
    let restricted_path = "/tmp/rustclaw-auto-sudo-workspace/var-log";

    let retry = build_auto_sudo_retry_args(
        &state,
        &task,
        "system_basic",
        Some(&serde_json::json!({
            "action": "inventory_dir",
            "path": restricted_path,
            "max_entries": 5
        })),
        &crate::skills::structured_skill_error_from_parts(
            "system_basic",
            "permission_denied",
            "read_dir failed for restricted dir",
            Some("linux"),
            Some(serde_json::json!({
                "operation": "read_dir",
                "path": restricted_path
            })),
        ),
    )
    .expect("admin permission denial should trigger sudo retry");

    let command = retry
        .get("command")
        .and_then(|value| value.as_str())
        .expect("retry command");
    assert!(command.starts_with("sudo -n sh -c "), "got: {command}");
    assert!(command.contains("basename"), "got: {command}");
    assert!(command.contains(restricted_path), "got: {command}");
    assert!(!command.contains("-printf"), "got: {command}");
    assert!(!command.contains("-maxdepth"), "got: {command}");
}

#[test]
fn auto_sudo_retry_skips_structured_reads_outside_workspace() {
    let mut state = test_state();
    state.policy.allow_sudo = true;
    state.skill_rt.workspace_root = PathBuf::from("/home/guagua/rustclaw");
    enable_test_skills(&state, &["run_cmd", "system_basic"]);
    insert_auth_key(&state, "rk-admin", "admin");
    let task = admin_task();

    let retry = build_auto_sudo_retry_args(
        &state,
        &task,
        "system_basic",
        Some(&serde_json::json!({
            "action": "read_range",
            "path": "/etc/shadow",
            "n": 1
        })),
        &crate::skills::structured_skill_error_from_parts(
            "system_basic",
            "permission_denied",
            "read_range failed for /etc/shadow",
            Some("linux"),
            Some(serde_json::json!({
                "operation": "read_file",
                "path": "/etc/shadow"
            })),
        ),
    );

    assert!(
        retry.is_none(),
        "structured reads outside workspace must not auto-escalate to sudo"
    );
}

#[test]
fn auto_sudo_retry_does_not_trigger_for_non_admin_or_existing_sudo() {
    let mut state = test_state();
    state.policy.allow_sudo = true;
    enable_test_skills(&state, &["run_cmd"]);
    insert_auth_key(&state, "rk-user", "user");
    let mut user_task = test_task();
    user_task.user_key = Some("rk-user".to_string());
    let err = "Command failed with exit code 1\nstderr:\nPermission denied";

    assert!(build_auto_sudo_retry_args(
        &state,
        &user_task,
        "run_cmd",
        Some(&serde_json::json!({"command": "cat /root/secret"})),
        err,
    )
    .is_none());

    insert_auth_key(&state, "rk-admin", "admin");
    let task = admin_task();
    assert!(build_auto_sudo_retry_args(
        &state,
        &task,
        "run_cmd",
        Some(&serde_json::json!({"command": "sudo cat /root/secret"})),
        err,
    )
    .is_none());
}

#[tokio::test]
async fn auto_sudo_retry_obeys_pre_tool_hook_policy() {
    let mut state = test_state();
    state.policy.allow_sudo = true;
    state.skill_rt.workspace_root = install_agent_guard_workspace(
        "blocked-run-cmd",
        r#"
[agent.hooks]
blocked_tools = ["run_cmd"]
"#,
    );
    enable_test_skills(&state, &["run_cmd", "system_basic"]);
    insert_auth_key(&state, "rk-admin", "admin");
    let task = admin_task();
    let restricted_path = state
        .skill_rt
        .workspace_root
        .join("restricted.log")
        .to_string_lossy()
        .to_string();
    let err = crate::skills::structured_skill_error_from_parts(
        "system_basic",
        "permission_denied",
        "read_range failed for restricted.log",
        Some("linux"),
        Some(serde_json::json!({
            "operation": "metadata",
            "path": restricted_path.clone()
        })),
    );
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;

    let stop = try_auto_sudo_retry_after_permission_denied(
        &state,
        &task,
        &mut loop_state,
        1,
        1,
        "inspect restricted log",
        "inspect restricted log",
        "system_basic",
        Some(&serde_json::json!({
            "action": "read_range",
            "path": restricted_path,
            "n": 1
        })),
        &err,
    )
    .await
    .expect("auto sudo retry preflight should not error");

    assert_eq!(
        stop.as_ref().and_then(|value| value.as_deref()),
        Some("recoverable_failure_continue_round")
    );
    assert!(loop_state.task_observations.iter().any(|observation| {
        observation.get("stage").and_then(serde_json::Value::as_str) == Some("pre_tool_use")
            && observation
                .get("decision")
                .and_then(serde_json::Value::as_str)
                == Some("deny")
            && observation
                .get("action_ref")
                .and_then(serde_json::Value::as_str)
                == Some("run_cmd")
    }));
    assert!(loop_state
        .executed_step_results
        .iter()
        .all(|step| step.output.is_none()));
    let _ = fs::remove_dir_all(&state.skill_rt.workspace_root);
}

fn ok_step(step_id: &str, skill: &str, output: &str) -> crate::executor::StepExecutionResult {
    crate::executor::StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 0,
        finished_at: 0,
    }
}

#[test]
fn skill_extra_user_input_signal_uses_generic_protocol_fields() {
    assert!(skill_extra_requests_user_input(Some(
        &serde_json::json!({"requires_user_input": true})
    )));
    assert!(skill_extra_requests_user_input(Some(
        &serde_json::json!({"needs_user_input": true})
    )));
    assert!(!skill_extra_requests_user_input(Some(
        &serde_json::json!({"needs_directory": true})
    )));
    assert!(!skill_extra_requests_user_input(None));
}

#[test]
fn structured_extra_evidence_output_wraps_text_and_extra_for_journal() {
    let extra = serde_json::json!({
        "outputs": [{
            "type": "image_file",
            "path": "/tmp/rustclaw-image.png"
        }]
    });

    let output = structured_extra_evidence_output("FILE:/tmp/rustclaw-image.png", Some(&extra))
        .expect("journal evidence output");
    let value: serde_json::Value = serde_json::from_str(&output).expect("json output");

    assert_eq!(
        value.get("text").and_then(serde_json::Value::as_str),
        Some("FILE:/tmp/rustclaw-image.png")
    );
    assert_eq!(
        value
            .pointer("/extra/outputs/0/path")
            .and_then(serde_json::Value::as_str),
        Some("/tmp/rustclaw-image.png")
    );
}

#[test]
fn isolation_artifacts_merge_into_journal_evidence() {
    let artifact_ref = serde_json::json!({
        "kind": "execution_isolation_workspace",
        "profile": "local_temp_workspace",
        "creation_kind": "create_local_temp_workspace",
        "artifact_path": "/tmp/rustclaw-isolated-task",
        "cleanup_ref": "isolation:temp:task",
        "requires_cleanup": true
    });
    let output = structured_extra_evidence_output(
        "ok",
        Some(&serde_json::json!({
            "status_code": "ok"
        })),
    );

    let merged =
        merge_isolation_artifact_refs(output, "ok", &[artifact_ref]).expect("merged evidence");
    let value: serde_json::Value = serde_json::from_str(&merged).expect("json evidence");

    assert_eq!(
        value.pointer("/artifacts/0/artifact_path"),
        Some(&serde_json::json!("/tmp/rustclaw-isolated-task"))
    );
    assert_eq!(
        value.pointer("/artifact_refs/0/cleanup_ref"),
        Some(&serde_json::json!("isolation:temp:task"))
    );
}

#[test]
fn image_output_kind_accepts_skill_text_protocol_schema() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "image_edit"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "image"
side_effect = true
auto_invocable = true
input_schema = { type = "object", properties = { instruction = { type = "string" } } }
output_schema = { type = "object", required = ["text"], properties = { text = { type = "string" }, extra = { type = "object" } } }
"#,
        &["image_edit"],
    );

    assert!(validate_skill_output_contract(
        &state,
        "image_edit",
        "Edited successfully and saved: /tmp/rustclaw-image.png"
    )
    .is_ok());
}

#[test]
fn admitted_extra_field_exists_checks_machine_field_paths() {
    let extra = serde_json::json!({
        "action": "count",
        "count": 3,
        "nested": {
            "path": "/tmp/out.txt"
        },
        "missing": null
    });

    assert!(admitted_extra_field_exists(&extra, "extra.count"));
    assert!(admitted_extra_field_exists(&extra, "extra.nested.path"));
    assert!(admitted_extra_field_exists(&extra, "count"));
    assert!(!admitted_extra_field_exists(&extra, "extra.missing"));
    assert!(!admitted_extra_field_exists(&extra, "extra.nope"));
}

fn failed_step(step_id: &str, skill: &str, error: &str) -> crate::executor::StepExecutionResult {
    crate::executor::StepExecutionResult {
        step_id: step_id.to_string(),
        skill: skill.to_string(),
        status: crate::executor::StepExecutionStatus::Error,
        output: None,
        error: Some(error.to_string()),
        started_at: 0,
        finished_at: 0,
    }
}

pub(super) fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos()
}

#[tokio::test]
async fn policy_block_failure_appends_user_visible_delivery() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    let err = crate::skills::policy_block_error(
        "path_outside_workspace",
        vec!["denied_path: /etc/shadow".to_string()],
        vec!["Do not access paths outside workspace.".to_string()],
    );
    let step = failed_step("step_1", "read_file", &err);

    let stop = handle_skill_step_failure(
        &state,
        &task,
        &step,
        &[],
        &["skill(read_file)".to_string()],
        &mut loop_state,
        0,
        1,
        1,
        "Read the first line of /etc/shadow.",
        "Read the first line of /etc/shadow",
        &test_policy(),
        "read_file",
        Some(&serde_json::json!({"path": "/etc/shadow"})),
        &err,
        "skill",
    )
    .await
    .expect("policy block should be converted to delivery");

    assert_eq!(stop.as_deref(), Some("policy_block_user_visible"));
    assert_eq!(loop_state.delivery_messages.len(), 1);
    assert!(loop_state.delivery_messages[0].contains("/etc/shadow"));
    assert!(loop_state.delivery_messages[0].contains("workspace"));
    assert!(loop_state
        .output_vars
        .get("failed_step.error")
        .is_some_and(|value| value.contains("path_outside_workspace")));
}

#[tokio::test]
async fn non_recoverable_failure_preserves_resume_context_and_user_error() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    let err = "planner schema mismatch: missing field `path`";
    let step = failed_step("step_1", "fragile_skill", err);
    let actions = vec![
        crate::AgentAction::CallSkill {
            skill: "fragile_skill".to_string(),
            args: serde_json::json!({}),
        },
        crate::AgentAction::CallSkill {
            skill: "next_skill".to_string(),
            args: serde_json::json!({}),
        },
    ];

    let outcome = handle_skill_step_failure(
        &state,
        &task,
        &step,
        &actions,
        &[
            "skill(fragile_skill)".to_string(),
            "skill(next_skill)".to_string(),
        ],
        &mut loop_state,
        0,
        1,
        1,
        "Run two ordered operations.",
        "Run two ordered operations",
        &test_policy(),
        "fragile_skill",
        Some(&serde_json::json!({})),
        err,
        "skill",
    )
    .await
    .expect_err("non-recoverable failure should return resume context error");

    let (user_error, payload) =
        crate::parse_resume_context_error(&outcome).expect("resume context payload");
    assert!(!user_error.trim().is_empty());
    assert!(payload
        .get("resume_context")
        .and_then(|v| v.get("remaining_steps"))
        .and_then(|v| v.as_array())
        .is_some_and(|steps| steps.len() == 1));
    assert!(payload
        .get("resume_context")
        .and_then(|v| v.get("failed_step"))
        .and_then(|v| v.get("error"))
        .and_then(|v| v.as_str())
        .is_some_and(|value| value.contains("missing field")));
}

#[tokio::test]
async fn missing_target_failure_without_fallback_publishes_failure_only() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "system_basic",
            "error_kind": "not_found",
            "error_text": "path not found: missing.md"
        })
    );
    let step = failed_step("step_1", "system_basic", &err);
    let actions = vec![crate::AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action":"read_range","path":"missing.md"}),
    }];

    let stop = handle_skill_step_failure(
        &state,
        &task,
        &step,
        &actions,
        &["skill(system_basic)".to_string()],
        &mut loop_state,
        0,
        1,
        1,
        "Read missing.md, then recover if needed.",
        "Read missing.md, then recover if needed.",
        &test_policy(),
        "system_basic",
        Some(&serde_json::json!({"action":"read_range","path":"missing.md"})),
        &err,
        "skill",
    )
    .await
    .expect("recoverable skill failure should not raise resume context");

    assert_eq!(stop.as_deref(), Some("recoverable_failure_finalize"));
    assert!(loop_state.has_recoverable_failure_context);
    let failed_error = loop_state
        .output_vars
        .get("failed_step.error")
        .map(String::as_str)
        .unwrap_or_default();
    assert!(
        failed_error.contains("target path was not found"),
        "failed_error={failed_error}"
    );
    assert_eq!(loop_state.progress_messages.len(), 1);
    assert!(loop_state.progress_messages[0].contains("telegram.progress.step_failed"));
    assert!(loop_state.progress_messages[0].contains("system_basic"));
    assert!(!loop_state
        .progress_messages
        .iter()
        .any(|message| message.contains("telegram.progress.retry_")));
}

#[tokio::test]
async fn recoverable_protocol_failure_publishes_replan_progress() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "system_basic",
            "error_kind": "unsupported_action",
            "error_text": "unknown action: check_exists"
        })
    );
    let step = failed_step("step_1", "system_basic", &err);
    let actions = vec![crate::AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action":"check_exists","path":"README.md"}),
    }];

    let stop = handle_skill_step_failure(
        &state,
        &task,
        &step,
        &actions,
        &["skill(system_basic)".to_string()],
        &mut loop_state,
        0,
        1,
        1,
        "Check README.md exists.",
        "Check README.md exists.",
        &test_policy(),
        "system_basic",
        Some(&serde_json::json!({"action":"check_exists","path":"README.md"})),
        &err,
        "skill",
    )
    .await
    .expect("protocol failure should be recoverable");

    assert_eq!(stop.as_deref(), Some("recoverable_failure_continue_round"));
    assert_eq!(loop_state.progress_messages.len(), 2);
    assert!(loop_state.progress_messages[0].contains("telegram.progress.step_failed"));
    assert!(loop_state.progress_messages[0].contains("system_basic"));
    assert!(loop_state.progress_messages[1].contains("telegram.progress.retry_replan"));
}

#[tokio::test]
async fn validation_failure_records_failed_output_and_advances_recipe_repair() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        repair_count: 0,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };

    let detail = "http response missing expected text=ops-repair-ok";
    let output = "status=200\nops-repair-bad\n";
    let outcome = handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:http_basic:{\"action\":\"get\"}",
        &ok_step("step_1", "http_basic", output),
        1,
        1,
        "http_basic",
        "skill",
        "",
        &serde_json::json!({ "action": "get", "url": "http://127.0.0.1:62078/" }),
        output,
        crate::execution_recipe::ActionEffect::validate(),
        crate::execution_recipe::ValidationObservation::Failed(detail.to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert!(!outcome.ended_with_user_visible_output);
    assert!(!outcome.continue_in_round);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
    assert_eq!(loop_state.execution_recipe.repair_count, 1);
    assert!(loop_state.has_tool_or_skill_output);
    assert_eq!(
        loop_state
            .output_vars
            .get("failed_step.error")
            .map(String::as_str),
        Some(detail)
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("skill.http_basic.error")
            .map(String::as_str),
        Some(detail)
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("failed_step.action")
            .map(String::as_str),
        Some("skill(http_basic)")
    );
    assert!(loop_state
        .history_compact
        .iter()
        .any(|line| line.contains("validation_failed")
            && line.contains("http response missing expected text=ops-repair-ok")));
    assert!(loop_state.successful_action_fingerprints.is_empty());
    assert_eq!(loop_state.executed_step_results.len(), 1);
    assert!(
        loop_state.last_recipe_progress_phase
            == Some(crate::execution_recipe::ExecutionRecipePhase::Repair)
    );
    assert!(loop_state
        .subtask_results
        .iter()
        .any(|line| line.contains("subtask#1 skill(http_basic): success")));
}

#[tokio::test]
async fn successful_skill_user_input_signal_finalizes_as_clarify_delivery() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;

    let output = "Please provide the directory path.";
    let outcome = handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:photo_organize:{\"action\":\"prepare\"}",
        &ok_step("step_1", "photo_organize", output),
        1,
        1,
        "photo_organize",
        "skill",
        "",
        &serde_json::json!({ "action": "prepare" }),
        output,
        crate::execution_recipe::ActionEffect::observe(),
        crate::execution_recipe::ValidationObservation::Passed,
        Some(&serde_json::json!({
            "requires_user_input": true,
            "missing_argument": "source_dir"
        })),
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("skill_requires_user_input")
    );
    assert!(outcome.ended_with_user_visible_output);
    assert!(loop_state.pending_user_input_required);
    assert_eq!(loop_state.delivery_messages, vec![output.to_string()]);
}

#[tokio::test]
async fn successful_validation_step_records_machine_result_for_closeout() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        saw_inspect: true,
        saw_mutation: true,
        ..Default::default()
    };

    handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:run_cmd:{\"command\":\"cargo check -p clawd\"}",
        &ok_step("step_3", "run_cmd", "validation ok"),
        3,
        2,
        "run_cmd",
        "skill",
        "command=cargo check -p clawd",
        &serde_json::json!({ "command": "cargo check -p clawd" }),
        "validation ok",
        crate::execution_recipe::ActionEffect::validate(),
        crate::execution_recipe::ValidationObservation::Passed,
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    let validation = loop_state
        .latest_validation_result
        .as_ref()
        .expect("validation result");
    assert_eq!(
        validation
            .get("status_code")
            .and_then(serde_json::Value::as_str),
        Some("validation_passed")
    );
    assert_eq!(
        validation.get("skill").and_then(serde_json::Value::as_str),
        Some("run_cmd")
    );
    assert_eq!(
        validation
            .get("global_step")
            .and_then(serde_json::Value::as_u64),
        Some(3)
    );
}

#[tokio::test]
async fn run_cmd_validation_failed_marker_advances_recipe_repair_without_success_fingerprint() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        phase: crate::execution_recipe::ExecutionRecipePhase::Validate,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        repair_count: 0,
        saw_inspect: true,
        saw_mutation: true,
        saw_validation: false,
        ..Default::default()
    };

    let output = "VALIDATION_FAILED\n";
    let outcome = handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:run_cmd:{\"command\":\"curl\"}",
        &ok_step("step_2", "run_cmd", output),
        2,
        1,
        "run_cmd",
        "skill",
        "",
        &serde_json::json!({ "command": "curl -s http://127.0.0.1:62078/" }),
        output,
        crate::execution_recipe::ActionEffect::validate(),
        crate::execution_recipe::ValidationObservation::Failed("VALIDATION_FAILED".to_string()),
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("recoverable_failure_continue_round")
    );
    assert_eq!(
        loop_state.execution_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Repair
    );
    assert_eq!(loop_state.execution_recipe.repair_count, 1);
    assert!(loop_state.successful_action_fingerprints.is_empty());
    assert!(loop_state
        .history_compact
        .iter()
        .any(|line| line.contains("skill=run_cmd")
            && line.contains("validation_failed=VALIDATION_FAILED")));
    assert_eq!(
        loop_state
            .output_vars
            .get("failed_step.error")
            .map(String::as_str),
        Some("VALIDATION_FAILED")
    );
    assert!(loop_state
        .subtask_results
        .iter()
        .any(|line| line.contains("subtask#2 skill(run_cmd): success")));
}

#[tokio::test]
async fn successful_external_workspace_step_records_scope_progress() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::ExternalWorkspace,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: false,
        max_repairs: 2,
        saw_inspect: true,
        ..Default::default()
    };

    handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:read_file:{\"path\":\"/opt/other-project/main.rs\"}",
        &ok_step("step_3", "read_file", "fn main() {}\n"),
        3,
        1,
        "read_file",
        "skill",
        "",
        &serde_json::json!({ "path": "/opt/other-project/main.rs" }),
        "fn main() {}\n",
        crate::execution_recipe::ActionEffect::observe(),
        crate::execution_recipe::ValidationObservation::Passed,
        None,
        None,
        Some("/opt/other-project/main.rs"),
    )
    .await
    .expect("skill step outcome");

    assert!(loop_state.execution_recipe.saw_external_target);
}

#[tokio::test]
async fn successful_greenfield_creation_step_records_scope_progress() {
    let state = test_state();
    let task = test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 1;
    loop_state.execution_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState {
        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
        target_scope: crate::execution_recipe::ExecutionRecipeTargetScope::Greenfield,
        phase: crate::execution_recipe::ExecutionRecipePhase::Apply,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        saw_inspect: true,
        ..Default::default()
    };

    handle_skill_step_success(
        &state,
        &task,
        &mut loop_state,
        "skill:write_file:{\"path\":\"tools/demo/main.rs\"}",
        &ok_step("step_4", "write_file", "ok"),
        4,
        1,
        "write_file",
        "skill",
        "",
        &serde_json::json!({ "path": "tools/demo/main.rs", "content": "fn main() {}\n" }),
        "ok",
        crate::execution_recipe::ActionEffect::mutate(),
        crate::execution_recipe::ValidationObservation::Passed,
        None,
        None,
        None,
    )
    .await
    .expect("skill step outcome");

    assert!(loop_state.execution_recipe.saw_greenfield_creation);
}
