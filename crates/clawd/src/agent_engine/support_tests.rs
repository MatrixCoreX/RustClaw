use super::{
    action_fingerprint, action_fingerprint_for_policy, append_delivery_message,
    build_agent_loop_checkpoint_progress_payload,
    build_agent_loop_user_input_checkpoint_progress_payload,
    collect_execution_recipe_progress_hints, execution_recipe_phase_progress_key,
    load_agent_loop_guard_policy, AgentLoopGuardPolicy, AnswerVerifierRequiredEvidenceScope,
    LoopBudgetProfile, LoopRecipeOverrides, RegistryIdempotencyGuardScope,
};
use crate::agent_engine::{seed_loop_state_for_agent_run, AgentRunContext, LoopState};
use crate::execution_recipe::{
    ExecutionRecipeKind, ExecutionRecipePhase, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
    ExecutionRecipeSpec, ExecutionRecipeTargetScope,
};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, SkillViewsSnapshot,
};
use claw_core::skill_registry::SkillsRegistry;
use std::sync::{Arc, RwLock};

fn base_policy() -> AgentLoopGuardPolicy {
    AgentLoopGuardPolicy {
        max_steps: 32,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 1,
        repeat_action_limit: 4,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 2,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        fast_read: LoopRecipeOverrides {
            max_steps: Some(16),
            max_rounds: Some(2),
            max_tool_calls: Some(6),
            repeat_action_limit: Some(3),
            no_progress_limit: Some(1),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        grounded_summary: LoopRecipeOverrides {
            max_steps: Some(40),
            max_rounds: Some(4),
            max_tool_calls: Some(16),
            repeat_action_limit: Some(5),
            no_progress_limit: Some(2),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        multi_step_workspace: LoopRecipeOverrides {
            max_steps: Some(56),
            max_rounds: Some(6),
            max_tool_calls: Some(24),
            repeat_action_limit: Some(6),
            no_progress_limit: Some(2),
            max_repairs: None,
            run_cmd_timeout_seconds: None,
            run_cmd_validation_timeout_seconds: None,
        },
        ops_closed_loop: LoopRecipeOverrides {
            max_steps: Some(48),
            max_rounds: Some(4),
            max_tool_calls: Some(24),
            repeat_action_limit: Some(6),
            no_progress_limit: Some(2),
            max_repairs: Some(3),
            run_cmd_timeout_seconds: Some(180),
            run_cmd_validation_timeout_seconds: Some(90),
        },
    }
}

fn temp_support_workspace(name: &str) -> std::path::PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "rustclaw-support-{name}-{}-{stamp}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create temp support workspace");
    dir
}

fn state_with_registry(toml: &str, skills: &[&str]) -> crate::AppState {
    let root = temp_support_workspace("registry-policy");
    let path = root.join("skills_registry.toml");
    std::fs::write(&path, toml).expect("write registry");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry"));
    let _ = std::fs::remove_dir_all(root);
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(skills.iter().map(|skill| (*skill).to_string()).collect()),
    })));
    state
}

fn support_test_task() -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: "task-support".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn registry_governance_fixture() -> &'static str {
    r#"
[[skills]]
name = "config_edit"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "config.apply", action = "apply_config_change", effect = "mutate", once_per_task = true, dedup_scope = "action", idempotent = false },
]

[[skills]]
name = "fs_basic"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "filesystem.list_entries", action = "list_dir", effect = "observe", idempotent = true, dedup_scope = "args" },
]

[[skills]]
name = "config_basic"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "config.validate", action = "validate_config", effect = "validate" },
]

[[skills]]
name = "run_cmd"
enabled = true
kind = "builtin"
risk_level = "high"
side_effect = true
requires_confirmation = true
planner_capabilities = [
  { name = "system.run_command", effect = "external", required = ["command"], once_per_task = true, idempotent = false, dedup_scope = "action" },
]

[[skills]]
name = "system_basic"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "system.run_command", action = "run_cmd", effect = "external", once_per_task = true, dedup_scope = "action", idempotent = false },
]
"#
}

#[test]
fn soft_budget_checkpoint_payload_records_machine_resume_state() {
    let task = support_test_task();
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 2;
    loop_state.total_steps_executed = 3;
    loop_state.tool_calls_total = 2;
    loop_state
        .progress_messages
        .push("I18N:telegram.progress.step_completed:{}".to_string());
    loop_state.successful_action_fingerprints.insert(
        "skill:config_edit:action:apply_config_change".to_string(),
        1,
    );
    loop_state.last_written_file_path = Some("src/lib.rs".to_string());
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "config_edit".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("{\"status\":\"ok\"}".to_string()),
            error: None,
            started_at: 100,
            finished_at: 102,
        });
    loop_state.last_stop_signal = Some("max_rounds".to_string());

    let payload = build_agent_loop_checkpoint_progress_payload(
        &task,
        &loop_state,
        "agent_loop_max_rounds",
        1_781_800_000,
        1_781_800_060,
    );

    assert_eq!(payload["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        payload["task_lifecycle"]["resume_reason"],
        "agent_loop_max_rounds"
    );
    assert_eq!(
        payload["task_lifecycle"]["message_key"],
        "clawd.task.agent_loop_max_rounds"
    );
    assert_eq!(payload["task_lifecycle"]["next_check_after"], 1_781_800_060);
    assert_eq!(
        payload["task_checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
    assert_eq!(
        payload["task_lifecycle"]["context_compaction_trigger"]["trigger_kind"],
        "before_background_checkpoint"
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["context_compaction_trigger"]
            ["resume_reason"],
        "agent_loop_max_rounds"
    );
    assert_eq!(payload["task_checkpoint"]["budget"]["round"], 2);
    assert_eq!(payload["task_checkpoint"]["budget"]["step"], 3);
    assert_eq!(payload["task_checkpoint"]["budget"]["tool_calls"], 2);
    assert_eq!(
        payload["task_checkpoint"]["budget"]["tool_elapsed_ms"],
        2000
    );
    assert_eq!(payload["task_lifecycle"]["budget"]["tool_elapsed_ms"], 2000);
    assert_eq!(
        payload["task_checkpoint"]["completed_side_effect_refs"][0],
        "skill:config_edit:action:apply_config_change"
    );
    assert_eq!(
        payload["task_checkpoint"]["artifact_refs"][0],
        "changed_file:src/lib.rs"
    );
    assert_eq!(
        payload["task_checkpoint"]["observations"][0]["step_id"],
        "step_1"
    );
    assert_eq!(
        payload["task_checkpoint"]["attempt_ledger"][0]["tool_or_skill"],
        "config_edit"
    );
    assert_eq!(
        payload["task_checkpoint"]["attempt_ledger"][0]["status"],
        "ok"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["signal"],
        "max_rounds"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_class"],
        "checkpoint_resume_repair"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["checkpoint_id"],
        payload["task_checkpoint"]["checkpoint_id"]
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["resume_entrypoint"],
        "next_planner_round"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["stop_reason_code"],
        "max_rounds"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["round_no"],
        2
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["repair_attempt"],
        2
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["max_attempts"],
        2
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["no_progress_count"],
        0
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["budget_exhausted"],
        true
    );
}

#[test]
fn provider_blocker_checkpoint_payload_records_background_resume_contract() {
    let task = support_test_task();
    let mut loop_state = LoopState::new(3);
    loop_state.round_no = 2;
    loop_state.last_stop_signal = Some("recoverable_failure_continue_round".to_string());
    let err = crate::skills::structured_skill_error_from_parts(
        "image_generate",
        "provider_retryable_response",
        "provider retryable response",
        None,
        Some(serde_json::json!({
            "provider": "minimax",
            "provider_error_class": "rate_limited",
            "external_provider_blocked": true,
            "retry_after_seconds": 60
        })),
    );
    crate::agent_engine::attempt_ledger::record_attempt(
        &mut loop_state,
        "image_generate",
        "action=generate",
        crate::executor::StepExecutionStatus::Error,
        "",
        None,
        &err,
    );

    let payload = build_agent_loop_checkpoint_progress_payload(
        &task,
        &loop_state,
        "provider_blocker_wait_background",
        1_781_800_000,
        1_781_800_060,
    );

    assert_eq!(payload["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        payload["task_lifecycle"]["resume_reason"],
        "provider_blocker_wait_background"
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["resume_reason"],
        "provider_blocker_wait_background"
    );
    assert_eq!(
        payload["task_checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["stop_reason_code"],
        "recoverable_failure_continue_round"
    );
    assert_eq!(
        payload["task_checkpoint"]["attempt_ledger"][0]["recovery_action"],
        "wait_background"
    );
    assert_eq!(
        payload["task_checkpoint"]["attempt_ledger"][0]["repair_signal"]["repair_envelope"]
            ["next_recovery_kind"],
        "wait_background"
    );
    assert_eq!(
        payload["task_checkpoint"]["attempt_ledger"][0]["repair_signal"]["repair_envelope"]
            ["provider_status"]["provider"],
        "minimax"
    );
    assert_eq!(
        payload["task_checkpoint"]["attempt_ledger"][0]["repair_signal"]["repair_envelope"]
            ["provider_status"]["status_code"],
        "rate_limited"
    );
    assert_eq!(
        payload["task_checkpoint"]["attempt_ledger"][0]["repair_signal"]["repair_envelope"]
            ["provider_status"]["retry_after_seconds"],
        60
    );
}

#[test]
fn verification_failure_checkpoint_payload_records_structured_attempt_evidence() {
    let task = support_test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.total_steps_executed = 5;
    loop_state.tool_calls_total = 3;
    loop_state.last_stop_signal = Some("recoverable_failure_continue_round".to_string());
    let err = crate::skills::structured_skill_error_from_parts(
        "run_cmd",
        "exit_status",
        "verification command failed",
        None,
        Some(serde_json::json!({
            "error_code": "exit_status",
            "exit_code": 101,
            "message_key": "clawd.run_cmd.exit_status",
            "command": "cargo test -p fixture"
        })),
    );
    crate::agent_engine::attempt_ledger::record_attempt(
        &mut loop_state,
        "run_cmd",
        "command=cargo test -p fixture",
        crate::executor::StepExecutionStatus::Error,
        "",
        None,
        &err,
    );

    let payload = build_agent_loop_checkpoint_progress_payload(
        &task,
        &loop_state,
        "agent_loop_max_rounds",
        1_781_800_000,
        1_781_800_060,
    );

    let attempt = &payload["task_checkpoint"]["attempt_ledger"][0];
    assert_eq!(attempt["tool_or_skill"], "run_cmd");
    assert_eq!(attempt["status"], "error");
    assert_eq!(attempt["error_kind"], "exit_status");
    assert_eq!(attempt["error_code"], "exit_status");
    assert_eq!(attempt["exit_code"], 101);
    assert_eq!(attempt["retryable"], true);
    assert_eq!(attempt["recovery_action"], "replan_changed_action_or_args");
    assert_eq!(attempt["repair_signal"]["status_code"], "exit_status");
    assert_eq!(
        attempt["repair_signal"]["message_key"],
        "clawd.run_cmd.exit_status"
    );
    assert_eq!(
        attempt["repair_signal"]["repair_envelope"]["repair_class"],
        "loop_bounded_recovery"
    );
    assert_eq!(attempt["repair_signal"]["owner_layer"], "execution_loop");
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["resume_entrypoint"],
        "next_planner_round"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["stop_reason_code"],
        "recoverable_failure_continue_round"
    );
}

#[test]
fn user_input_checkpoint_payload_records_hook_confirmation_state() {
    let task = support_test_task();
    let mut loop_state = LoopState::new(2);
    loop_state.round_no = 1;
    loop_state.tool_calls_total = 1;
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "read_file".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("{\"status\":\"ok\"}".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
        });

    let payload = build_agent_loop_user_input_checkpoint_progress_payload(
        &task,
        &loop_state,
        "hook_confirmation_required",
        1_781_800_001,
        "package_manager",
        "package_manager.install",
        &serde_json::json!({
            "action": "install",
            "package": "example",
            "token": "redacted-by-caller"
        }),
    );

    assert_eq!(payload["task_lifecycle"]["state"], "needs_user");
    assert_eq!(
        payload["task_lifecycle"]["resume_reason"],
        "hook_confirmation_required"
    );
    assert_eq!(
        payload["task_lifecycle"]["next_action_kind"],
        serde_json::Value::Null
    );
    assert_eq!(
        payload["task_checkpoint"]["resume_entrypoint"],
        "await_user_input"
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["source"],
        "agent_hooks"
    );
    assert_eq!(
        payload["task_checkpoint"]["pending_action"]["action_ref"],
        "package_manager.install"
    );
    assert_eq!(
        payload["task_checkpoint"]["pending_action"]["args_keys"],
        serde_json::json!(["action", "package", "token"])
    );
    assert!(!payload["task_checkpoint"]["pending_action"]
        .to_string()
        .contains("redacted-by-caller"));
}

#[test]
fn no_progress_checkpoint_payload_records_repair_budget_state() {
    let task = support_test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 3;
    loop_state.consecutive_no_progress = 2;
    loop_state.last_stop_signal = Some("no_progress".to_string());

    let payload = build_agent_loop_checkpoint_progress_payload(
        &task,
        &loop_state,
        "agent_loop_no_progress_limit",
        1_781_800_000,
        1_781_800_060,
    );

    assert_eq!(payload["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        payload["task_lifecycle"]["resume_reason"],
        "agent_loop_no_progress_limit"
    );
    assert_eq!(
        payload["task_lifecycle"]["message_key"],
        "clawd.task.agent_loop_no_progress_limit"
    );
    assert_eq!(payload["task_lifecycle"]["next_check_after"], 1_781_800_060);
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_class"],
        "checkpoint_resume_repair"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["next_recovery_kind"],
        "wait_background"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["stop_reason_code"],
        "no_progress"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["round_no"],
        3
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["max_attempts"],
        4
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["no_progress_count"],
        2
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["budget_exhausted"],
        true
    );
}

#[test]
fn budget_near_exhaustion_checkpoint_payload_records_message_key() {
    let task = support_test_task();
    let mut loop_state = LoopState::new(4);
    loop_state.round_no = 2;
    loop_state.total_steps_executed = 3;
    loop_state.last_stop_signal = Some("budget_near_exhaustion".to_string());

    let payload = build_agent_loop_checkpoint_progress_payload(
        &task,
        &loop_state,
        "budget_near_exhaustion",
        1_781_800_000,
        1_781_800_060,
    );

    assert_eq!(
        payload["task_lifecycle"]["resume_reason"],
        "budget_near_exhaustion"
    );
    assert_eq!(
        payload["task_lifecycle"]["message_key"],
        "clawd.task.budget_near_exhaustion"
    );
    assert_eq!(
        payload["task_checkpoint"]["boundary_context"]["message_key"],
        "clawd.task.budget_near_exhaustion"
    );
}

#[test]
fn seed_loop_state_restores_checkpoint_budget_and_side_effect_guards() {
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: "ckpt-seed-state".to_string(),
        boundary_context: serde_json::json!({"route_gate_kind": "execute"}),
        last_successful_round: Some(3),
        last_successful_step: Some("step_4".to_string()),
        pending_action: None,
        observations: vec![serde_json::json!({
            "step_id": "step_4",
            "status": "ok"
        })],
        evidence_refs: vec!["step_4".to_string()],
        artifact_refs: vec![
            "artifact:report".to_string(),
            "changed_file:src/lib.rs".to_string(),
        ],
        completed_side_effect_refs: vec![
            "skill:config_edit:action:apply_config_change".to_string(),
            " ".to_string(),
        ],
        budget: crate::task_lifecycle::CheckpointBudgetCounters {
            round: 3,
            step: 4,
            llm_calls: 5,
            tool_calls: 2,
            elapsed_ms: 900,
            llm_elapsed_ms: 900,
            tool_elapsed_ms: 0,
        },
        attempt_ledger: Some(serde_json::json!([{
            "attempt_id": "a1",
            "action_ref": "config_edit",
            "tool_or_skill": "config_edit",
            "status": "error",
            "error_code": "needs_retry"
        }])),
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
    };
    let mut loop_state = LoopState::new(8);

    let report = seed_loop_state_for_agent_run(&mut loop_state, None, Some(&checkpoint))
        .expect("checkpoint seed report");

    assert_eq!(report.checkpoint_id, "ckpt-seed-state");
    assert_eq!(report.restored_round, 3);
    assert_eq!(report.restored_step, 4);
    assert_eq!(report.restored_tool_calls, 2);
    assert_eq!(report.completed_side_effect_count, 1);
    assert_eq!(report.observation_count, 1);
    assert_eq!(loop_state.round_no, 3);
    assert_eq!(loop_state.total_steps_executed, 4);
    assert_eq!(loop_state.tool_calls_total, 2);
    assert!(loop_state.has_tool_or_skill_output);
    assert_eq!(
        loop_state
            .successful_action_fingerprints
            .get("skill:config_edit:action:apply_config_change"),
        Some(&1)
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.resume_checkpoint_id"),
        Some(&"ckpt-seed-state".to_string())
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.resume_completed_side_effect_count"),
        Some(&"1".to_string())
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.resume_changed_files_json"),
        Some(&"[\"src/lib.rs\"]".to_string())
    );
    assert_eq!(
        loop_state.last_written_file_path.as_deref(),
        Some("src/lib.rs")
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.resume_attempt_ledger_present"),
        Some(&"true".to_string())
    );
    assert!(loop_state
        .history_compact
        .iter()
        .any(|line| line.starts_with("checkpoint_attempt_ledger_json=")
            && line.contains("\"error_code\":\"needs_retry\"")));
    assert_eq!(
        loop_state
            .task_checkpoint
            .as_ref()
            .and_then(|value| value.get("checkpoint_id"))
            .and_then(serde_json::Value::as_str),
        Some("ckpt-seed-state")
    );
}

#[test]
fn seed_loop_state_extracts_current_request_locator_boundary_observation() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "current_request_locator": {
            "source": "current_request",
            "has_concrete_surface": true,
            "explicit_locator_hints": [
                {"kind": "path", "hint": "docs/README.md"}
            ],
            "resolved_workspace_child": "/tmp/rustclaw/docs/README.md",
            "has_multiple_local_paths": false
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("read docs readme\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    let evidence = loop_state
        .output_vars
        .get("current_request_locator_evidence")
        .expect("locator evidence");
    assert!(evidence.contains("docs/README.md"));
    assert!(evidence.contains("/tmp/rustclaw/docs/README.md"));
    assert_eq!(
        loop_state
            .output_vars
            .get("current_request_resolved_workspace_child_targets"),
        Some(&"[\"/tmp/rustclaw/docs/README.md\"]".to_string())
    );
}

#[test]
fn seed_loop_state_ignores_missing_referent_when_current_request_locator_is_concrete() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "needs_clarify": false,
        "current_request_locator": {
            "source": "current_request",
            "has_concrete_surface": true,
            "explicit_locator_hints": [
                {"kind": "filename", "hint": "README.md"}
            ],
            "resolved_workspace_root": "/tmp/rustclaw"
        },
        "missing_referent": {
            "owner_layer": "agent_loop_boundary",
            "reason_code": "unbound_deictic_reference",
            "status_code": "missing_referent",
            "missing_slot": "referent"
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("read README.md\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    assert!(!loop_state.boundary_observation_needs_clarify);
    assert!(!loop_state
        .output_vars
        .contains_key("agent_loop.boundary_observation_needs_clarify"));
    assert!(loop_state
        .output_vars
        .get("current_request_locator_evidence")
        .is_some_and(|evidence| evidence.contains("README.md")));
}

#[test]
fn seed_loop_state_extracts_active_plan_file_targets_boundary_observation() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "active_plan_files": [{
            "source": "workspace_plan_directory",
            "logical_path": "plan/active.md",
            "workspace_path": "/tmp/rustclaw/plan/active.md",
            "bytes": 128
        }]
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("review plan\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    assert_eq!(
        loop_state.output_vars.get("active_plan_file_targets"),
        Some(&"[\"/tmp/rustclaw/plan/active.md\"]".to_string())
    );
}

#[test]
fn seed_loop_state_extracts_default_main_config_contract_boundary_observation() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "default_main_config_contract": {
            "source": "boundary_contract",
            "contract": "rustclaw_main_config",
            "logical_path": "configs/config.toml",
            "workspace_path": "/tmp/rustclaw/configs/config.toml",
            "exists": true,
            "route_markers": ["config_validation"]
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("audit config\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    let evidence = loop_state
        .output_vars
        .get("default_main_config_contract_evidence")
        .expect("default config evidence");
    assert!(evidence.contains("rustclaw_main_config"));
    assert_eq!(
        loop_state
            .output_vars
            .get("default_main_config_contract_logical_path"),
        Some(&"configs/config.toml".to_string())
    );
    assert_eq!(
        loop_state
            .output_vars
            .get("default_main_config_contract_workspace_path"),
        Some(&"/tmp/rustclaw/configs/config.toml".to_string())
    );
}

#[test]
fn seed_loop_state_extracts_registry_capability_contract_boundary_observation() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "registry_capability_contract": {
            "source": "registry_capability_ref",
            "capability_refs": ["kb.list_namespaces", "kb.search"],
            "has_conflicting_route_contract": true,
            "route_gate_kind": "clarify",
            "needs_clarify": true,
            "locator_kind": "current_workspace",
            "locator_hint": "docs",
            "delivery_required": true,
            "delivery_intent": "none",
            "response_shape": "free"
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("query kb\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    let evidence = loop_state
        .output_vars
        .get("registry_capability_contract_evidence")
        .expect("registry capability evidence");
    assert!(evidence.contains("kb.list_namespaces"));
    assert!(!evidence.contains("directory_entry_groups"));
    assert_eq!(
        loop_state
            .output_vars
            .get("registry_capability_contract_refs"),
        Some(&"[\"kb.list_namespaces\",\"kb.search\"]".to_string())
    );
}

#[test]
fn seed_loop_state_extracts_contract_repair_candidate_boundary_observation() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "contract_repair_candidates": [
            {
                "source": "sqlite_structured_version",
                "contract_ref": "contract:sqlite_schema_version",
                "locator_hint": "data/app.sqlite",
                "response_shape": "scalar"
            }
        ]
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("inspect sqlite\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    let evidence = loop_state
        .output_vars
        .get("contract_repair_candidate_evidence")
        .expect("contract repair candidate evidence");
    assert!(evidence.contains("sqlite_structured_version"));
    assert!(evidence.contains("contract:sqlite_schema_version"));
    assert!(evidence.contains("data/app.sqlite"));
}

#[test]
fn seed_loop_state_extracts_pre_loop_clarify_candidates() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "pre_loop_clarify_candidates": ["bare_topic_context_expansion"]
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("logs\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    assert_eq!(
        loop_state.output_vars.get("pre_loop_clarify_candidates"),
        Some(&"[\"bare_topic_context_expansion\"]".to_string())
    );
}

#[test]
fn seed_loop_state_extracts_boundary_observation_needs_clarify() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "needs_clarify": true,
        "post_route_boundary_record": {
            "outcome": "boundary_clarify",
            "reason_code": "post_route_boundary_clarify_required"
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("ambiguous delivery\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    assert!(loop_state.boundary_observation_needs_clarify);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.boundary_observation_needs_clarify"),
        Some(&"true".to_string())
    );
}

#[test]
fn seed_loop_state_extracts_pending_user_boundary() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "runtime_session_state": {
            "active_clarify_present": true,
            "pending_user_boundary_present": true
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("followup\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    assert!(loop_state.pending_user_boundary_present);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.pending_user_boundary_present"),
        Some(&"true".to_string())
    );
}

#[test]
fn seed_loop_state_treats_missing_referent_as_boundary_clarify() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "needs_clarify": false,
        "missing_referent": {
            "owner_layer": "agent_loop_boundary",
            "reason_code": "unbound_deictic_reference",
            "status_code": "missing_referent",
            "missing_slot": "referent"
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("continue previous project\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    assert!(loop_state.boundary_observation_needs_clarify);
    assert_eq!(
        loop_state
            .output_vars
            .get("agent_loop.boundary_observation_needs_clarify"),
        Some(&"true".to_string())
    );
}

#[test]
fn seed_loop_state_ignores_missing_referent_when_auto_locator_boundary_ready() {
    let observation = serde_json::json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "needs_clarify": false,
        "post_route_boundary_record": {
            "outcome": "boundary_ready",
            "reason_code": "post_route_auto_locator_satisfied_path_scoped_content"
        },
        "auto_locator": {
            "resolved_direct": true,
            "path": "/workspace/rustclaw.service",
            "fuzzy_candidates": []
        },
        "missing_referent": {
            "owner_layer": "agent_loop_boundary",
            "reason_code": "unbound_deictic_reference",
            "status_code": "missing_referent",
            "missing_slot": "referent"
        }
    });
    let block = format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    );
    let ctx = AgentRunContext {
        user_request: Some(format!("check file\n{block}")),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(4);

    seed_loop_state_for_agent_run(&mut loop_state, Some(&ctx), None);

    assert!(!loop_state.boundary_observation_needs_clarify);
    assert!(!loop_state
        .output_vars
        .contains_key("agent_loop.boundary_observation_needs_clarify"));
}

#[test]
fn guard_policy_defaults_to_agent_loop_authority_when_config_missing() {
    let root = temp_support_workspace("rollout-defaults");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_answer_verifier_required_evidence_scope(),
        AnswerVerifierRequiredEvidenceScope::All
    );
    assert!(policy.answer_verifier_required_evidence_enabled());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn registry_idempotency_guard_switches_mutate_capability_to_action_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit", "fs_basic"]);
    let mut policy = base_policy();
    let left = crate::AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "apply_config_change",
            "field_path": "skills.a",
            "value": true
        }),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "apply_config_change",
            "field_path": "skills.b",
            "value": true
        }),
    };

    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );

    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        "skill:config_edit:action:apply_config_change"
    );
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
}

#[test]
fn registry_idempotency_guard_resolves_planner_capability_before_policy() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let action = crate::AgentAction::CallCapability {
        capability: "config.apply".to_string(),
        args: serde_json::json!({
            "field_path": "skills.photo_organize",
            "value": true
        }),
    };
    let fingerprint = action_fingerprint_for_policy(&state, &policy, &action);

    assert_eq!(fingerprint, "skill:config_edit:action:apply_config_change");
    assert!(super::registry_idempotency_guard_attribution(
        &state,
        &policy,
        &action,
        &fingerprint,
        "registry_idempotency_repeat_completed_action",
        Some(1),
        None,
    )
    .is_some());
}

#[test]
fn registry_idempotency_guard_keeps_observe_capability_args_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit", "fs_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let left = crate::AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({"action": "list_dir", "path": "/tmp/a"}),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({"action": "list_dir", "path": "/tmp/b"}),
    };

    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
}

#[test]
fn registry_idempotency_guard_keeps_validate_capability_args_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["config_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let left = crate::AgentAction::CallSkill {
        skill: "config_basic".to_string(),
        args: serde_json::json!({"action": "validate_config", "path": "/tmp/a.toml"}),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "config_basic".to_string(),
        args: serde_json::json!({"action": "validate_config", "path": "/tmp/b.toml"}),
    };

    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
}

#[test]
fn registry_idempotency_guard_keeps_direct_run_cmd_command_args_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["run_cmd"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let left = crate::AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({"command": "true"}),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({"command": "false"}),
    };

    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
    assert!(super::registry_idempotency_guard_attribution(
        &state,
        &policy,
        &left,
        &action_fingerprint_for_policy(&state, &policy, &left),
        "registry_idempotency_repeat_completed_action",
        Some(1),
        None,
    )
    .is_none());
}

#[test]
fn registry_idempotency_guard_keeps_system_run_cmd_command_args_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["system_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let left = crate::AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action": "run_cmd", "command": "true"}),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action": "run_cmd", "command": "false"}),
    };

    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
}

#[test]
fn registry_idempotency_guard_keeps_action_fingerprint_without_command_args() {
    let state = state_with_registry(registry_governance_fixture(), &["system_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let action = crate::AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({"action": "run_cmd"}),
    };

    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &action),
        "skill:system_basic:action:run_cmd"
    );
}

#[test]
fn registry_idempotency_guard_keeps_run_cmd_args_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["system_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let left = crate::AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "run_cmd",
            "command": "echo RC_STEP_ONE",
            super::super::CLAWD_LITERAL_COMMAND_ARG: true
        }),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "system_basic".to_string(),
        args: serde_json::json!({
            "action": "run_cmd",
            "command": "definitely_missing_command_rc_step_two",
            super::super::CLAWD_LITERAL_COMMAND_ARG: true
        }),
    };

    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
    assert!(super::registry_idempotency_guard_attribution(
        &state,
        &policy,
        &left,
        &action_fingerprint_for_policy(&state, &policy, &left),
        "registry_idempotency_repeat_completed_action",
        Some(1),
        None,
    )
    .is_none());
}

#[test]
fn registry_idempotency_guard_does_not_attribute_idempotent_read_repeats() {
    let state = state_with_registry(registry_governance_fixture(), &["fs_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let action = crate::AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({"action": "list_dir", "path": "/tmp/a"}),
    };
    let fingerprint = action_fingerprint_for_policy(&state, &policy, &action);

    assert!(super::registry_idempotency_guard_attribution(
        &state,
        &policy,
        &action,
        &fingerprint,
        "registry_idempotency_repeat_completed_action",
        Some(1),
        None,
    )
    .is_none());
}

#[test]
fn rollout_switches_are_read_from_agent_guard_config() {
    let root = temp_support_workspace("rollout-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
answer_verifier_enforce_required_scope = "all"
registry_idempotency_guard_scope = "all"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_answer_verifier_required_evidence_scope(),
        AnswerVerifierRequiredEvidenceScope::All
    );
    assert_eq!(
        policy.effective_registry_idempotency_guard_scope(),
        RegistryIdempotencyGuardScope::All
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn answer_verifier_required_scope_legacy_token_normalizes_to_all() {
    let root = temp_support_workspace("answer-verifier-scope-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
answer_verifier_enforce_required_scope = "selected_agent_loop"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_answer_verifier_required_evidence_scope(),
        AnswerVerifierRequiredEvidenceScope::All
    );
    assert!(policy
        .enabled_rollout_switches()
        .contains(&"answer_verifier_enforce_required_scope"));
    assert!(policy.answer_verifier_required_evidence_enabled());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn answer_verifier_required_scope_accepts_all_token() {
    let root = temp_support_workspace("answer-verifier-scope-all-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
answer_verifier_enforce_required_scope = "all"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_answer_verifier_required_evidence_scope(),
        AnswerVerifierRequiredEvidenceScope::All
    );
    assert!(policy
        .enabled_rollout_switches()
        .contains(&"answer_verifier_enforce_required_scope"));
    assert!(!policy
        .enabled_rollout_switches()
        .contains(&"answer_verifier_enforce_required"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn answer_verifier_required_scope_final_all_is_enabled() {
    let mut policy = base_policy();
    policy.answer_verifier_enforce_required_scope = AnswerVerifierRequiredEvidenceScope::All;
    assert!(policy.answer_verifier_required_evidence_enabled());
}

#[test]
fn answer_verifier_required_scope_all_token_is_enabled() {
    let mut policy = base_policy();
    policy.answer_verifier_enforce_required_scope = AnswerVerifierRequiredEvidenceScope::All;
    assert!(policy.answer_verifier_required_evidence_enabled());
}

#[test]
fn registry_idempotency_guard_scope_legacy_token_normalizes_to_all() {
    let root = temp_support_workspace("registry-scope-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
registry_idempotency_guard_scope = "selected_agent_loop"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_registry_idempotency_guard_scope(),
        RegistryIdempotencyGuardScope::All
    );
    assert!(policy
        .enabled_rollout_switches()
        .contains(&"registry_idempotency_guard_scope"));
    assert!(policy.registry_idempotency_guard_enabled());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn registry_idempotency_guard_scope_accepts_all_token() {
    let root = temp_support_workspace("registry-scope-all-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
registry_idempotency_guard_scope = "all"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_registry_idempotency_guard_scope(),
        RegistryIdempotencyGuardScope::All
    );
    assert!(policy
        .enabled_rollout_switches()
        .contains(&"registry_idempotency_guard_scope"));
    assert!(!policy
        .enabled_rollout_switches()
        .contains(&"registry_idempotency_guard"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn registry_idempotency_guard_scope_final_all_is_enabled() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let left = crate::AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "apply_config_change",
            "field_path": "skills.a",
            "value": true
        }),
    };
    let right = crate::AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "apply_config_change",
            "field_path": "skills.b",
            "value": true
        }),
    };

    assert!(policy.registry_idempotency_guard_enabled());
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        "skill:config_edit:action:apply_config_change"
    );
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left),
        action_fingerprint_for_policy(&state, &policy, &right)
    );
}

#[test]
fn registry_idempotency_guard_scope_all_token_is_enabled() {
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    assert!(policy.registry_idempotency_guard_enabled());
}

#[test]
fn deprecated_domain_action_lists_do_not_change_loop_guard_policy() {
    let root = temp_support_workspace("deprecated-domain-actions");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard.crypto]
news_actions = ["legacy_news"]
market_query_actions = ["legacy_quote"]
trade_preview_actions = ["legacy_preview"]
trade_submit_actions = ["legacy_submit"]

[agent.loop_guard.fs_search]
query_actions = ["legacy_find"]

[agent.loop_guard.media]
image_generate_skills = ["legacy_image_generate"]
image_edit_skills = ["legacy_image_edit"]
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(policy.max_rounds, 2);
    assert_eq!(policy.max_steps, 32);
    assert_eq!(policy.max_tool_calls, 12);
    assert_eq!(
        policy.effective_registry_idempotency_guard_scope(),
        RegistryIdempotencyGuardScope::All
    );
    assert!(policy.registry_idempotency_guard_enabled());

    let _ = std::fs::remove_dir_all(root);
}

fn route_with_contract(
    semantic_kind: OutputSemanticKind,
    locator_kind: OutputLocatorKind,
) -> IntentOutputContract {
    IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    }
}

#[test]
fn ops_closed_loop_policy_uses_override_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    let adjusted = policy.adjusted_for_context(recipe, None);
    assert_eq!(adjusted.max_steps, 48);
    assert_eq!(adjusted.max_rounds, 4);
    assert_eq!(adjusted.max_tool_calls, 24);
    assert_eq!(adjusted.repeat_action_limit, 6);
    assert_eq!(adjusted.no_progress_limit, 2);
    assert_eq!(
        adjusted.run_cmd_timeout_override(recipe, crate::execution_recipe::ActionEffect::mutate()),
        Some(180)
    );
    assert_eq!(
        adjusted
            .run_cmd_timeout_override(recipe, crate::execution_recipe::ActionEffect::validate()),
        Some(90)
    );
}

#[test]
fn planner_contract_selects_grounded_summary_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::default();
    let route = route_with_contract(
        OutputSemanticKind::CommandOutputSummary,
        OutputLocatorKind::None,
    );

    assert_eq!(
        AgentLoopGuardPolicy::budget_profile_for_context(recipe, Some(&route)),
        LoopBudgetProfile::GroundedSummary
    );
    let adjusted = policy.adjusted_for_context(recipe, Some(&route));
    assert_eq!(adjusted.max_rounds, 4);
    assert_eq!(adjusted.max_tool_calls, 16);
    assert_eq!(adjusted.no_progress_limit, 2);
}

#[test]
fn planner_contract_budget_does_not_depend_on_legacy_route_trace() {
    let recipe = ExecutionRecipeRuntimeState::default();
    let route = route_with_contract(
        OutputSemanticKind::CommandOutputSummary,
        OutputLocatorKind::None,
    );

    assert_eq!(
        AgentLoopGuardPolicy::budget_profile_for_context(recipe, Some(&route)),
        LoopBudgetProfile::GroundedSummary
    );
}

#[test]
fn workspace_delivery_contract_selects_multi_step_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::default();
    let mut route = route_with_contract(
        OutputSemanticKind::GeneratedFileDelivery,
        OutputLocatorKind::Filename,
    );
    route.delivery_required = true;
    route.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.response_shape = OutputResponseShape::FileToken;

    assert_eq!(
        AgentLoopGuardPolicy::budget_profile_for_context(recipe, Some(&route)),
        LoopBudgetProfile::MultiStepWorkspace
    );
    let adjusted = policy.adjusted_for_context(recipe, Some(&route));
    assert_eq!(adjusted.max_rounds, 6);
    assert_eq!(adjusted.max_steps, 56);
    assert_eq!(adjusted.max_tool_calls, 24);
}

#[test]
fn ops_closed_loop_runtime_applies_repair_override() {
    let policy = base_policy();
    let mut recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        inspect_first: true,
        validation_required: true,
        max_repairs: 2,
        ..Default::default()
    });
    policy.apply_recipe_runtime_overrides(&mut recipe);
    assert_eq!(recipe.max_repairs, 3);
}

#[test]
fn append_delivery_message_sanitizes_structured_skill_errors() {
    let mut messages = Vec::new();
    append_delivery_message(
        "task-support-test",
        &mut messages,
        r#"执行失败：__RC_SKILL_ERROR__:{"skill":"archive_basic","error_kind":"unknown","error_text":"archive is required","text":null}。"#
            .to_string(),
    );

    assert_eq!(messages, vec!["执行失败：archive is required。"]);
}

#[test]
fn external_workspace_progress_hints_include_mode_and_ready_once() {
    let mut loop_state = LoopState::new(4);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        target_scope: ExecutionRecipeTargetScope::ExternalWorkspace,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    });

    let first = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("telegram.progress.ops_recipe_scope_external_mode"));
    assert!(first[1].contains("telegram.progress.ops_recipe_inspect"));

    loop_state.execution_recipe.saw_external_target = true;
    let second = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(second.len(), 1);
    assert!(second[0].contains("telegram.progress.ops_recipe_scope_external_ready"));

    let third = collect_execution_recipe_progress_hints(&mut loop_state);
    assert!(third.is_empty());
}

#[test]
fn greenfield_progress_hints_include_mode_and_creation_ready_once() {
    let mut loop_state = LoopState::new(4);
    loop_state.execution_recipe = ExecutionRecipeRuntimeState::from_spec(ExecutionRecipeSpec {
        kind: ExecutionRecipeKind::OpsClosedLoop,
        target_scope: ExecutionRecipeTargetScope::Greenfield,
        inspect_first: true,
        validation_required: true,
        ..Default::default()
    });

    let first = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(first.len(), 2);
    assert!(first[0].contains("telegram.progress.ops_recipe_scope_greenfield_mode"));
    assert!(first[1].contains("telegram.progress.ops_recipe_inspect"));

    loop_state.execution_recipe.saw_greenfield_creation = true;
    let second = collect_execution_recipe_progress_hints(&mut loop_state);
    assert_eq!(second.len(), 1);
    assert!(second[0].contains("telegram.progress.ops_recipe_scope_greenfield_ready"));

    let third = collect_execution_recipe_progress_hints(&mut loop_state);
    assert!(third.is_empty());
}

#[test]
fn code_change_phase_progress_uses_profile_specific_keys() {
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Inspect
        ),
        "telegram.progress.code_change_inspect"
    );
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Apply
        ),
        "telegram.progress.code_change_apply"
    );
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::CodeChange,
            ExecutionRecipePhase::Validate
        ),
        "telegram.progress.code_change_validate"
    );
}

#[test]
fn skill_authoring_validate_progress_uses_profile_specific_key() {
    assert_eq!(
        execution_recipe_phase_progress_key(
            ExecutionRecipeProfile::SkillAuthoring,
            ExecutionRecipePhase::Validate
        ),
        "telegram.progress.skill_authoring_validate"
    );
}
