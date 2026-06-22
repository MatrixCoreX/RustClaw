use super::{
    action_fingerprint, action_fingerprint_for_policy, append_delivery_message,
    build_agent_loop_checkpoint_progress_payload,
    build_agent_loop_user_input_checkpoint_progress_payload,
    collect_execution_recipe_progress_hints, execution_recipe_phase_progress_key,
    load_agent_loop_guard_policy, AgentLoopGuardPolicy, AnswerVerifierRequiredEvidenceScope,
    LoopBudgetProfile, LoopRecipeOverrides, RegistryIdempotencyGuardScope, SemanticRouteAuthority,
};
use crate::agent_engine::{seed_loop_state_for_agent_run, LoopState};
use crate::execution_recipe::{
    ExecutionRecipeKind, ExecutionRecipePhase, ExecutionRecipeProfile, ExecutionRecipeRuntimeState,
    ExecutionRecipeSpec, ExecutionRecipeTargetScope,
};
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind, SkillViewsSnapshot,
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
        semantic_route_authority: SemanticRouteAuthority::Legacy,
        agent_loop_canary_bucket: "none".to_string(),
        registry_idempotency_guard_scope: RegistryIdempotencyGuardScope::Off,
        structured_evidence_required_for_selected_contracts: false,
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
    loop_state
        .executed_step_results
        .push(crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "config_edit".to_string(),
            status: crate::executor::StepExecutionStatus::Ok,
            output: Some("{\"status\":\"ok\"}".to_string()),
            error: None,
            started_at: 0,
            finished_at: 0,
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
    assert_eq!(payload["task_lifecycle"]["next_check_after"], 1_781_800_060);
    assert_eq!(
        payload["task_checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
    assert_eq!(payload["task_checkpoint"]["budget"]["round"], 2);
    assert_eq!(payload["task_checkpoint"]["budget"]["step"], 3);
    assert_eq!(payload["task_checkpoint"]["budget"]["tool_calls"], 2);
    assert_eq!(
        payload["task_checkpoint"]["completed_side_effect_refs"][0],
        "skill:config_edit:action:apply_config_change"
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

    assert_eq!(
        payload["task_lifecycle"]["resume_reason"],
        "agent_loop_no_progress_limit"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_class"],
        "checkpoint_resume_repair"
    );
    assert_eq!(
        payload["task_checkpoint"]["repair_signal"]["repair_envelope"]["next_recovery_kind"],
        "wait_background"
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
        artifact_refs: vec!["artifact:report".to_string()],
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
fn rollout_switches_default_to_false_when_config_missing() {
    let root = temp_support_workspace("rollout-defaults");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_answer_verifier_required_evidence_scope(),
        AnswerVerifierRequiredEvidenceScope::Off
    );
    assert_eq!(
        policy.semantic_route_authority,
        SemanticRouteAuthority::Legacy
    );
    assert!(!policy.records_agent_decides_attribution());
    assert_eq!(policy.agent_loop_canary_bucket, "none");
    assert!(!policy.structured_evidence_required_for_selected_contracts);

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
        action_fingerprint_for_policy(&state, &policy, &left, None),
        action_fingerprint_for_policy(&state, &policy, &right, None)
    );

    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left, None),
        "skill:config_edit:action:apply_config_change"
    );
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left, None),
        action_fingerprint_for_policy(&state, &policy, &right, None)
    );
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
        action_fingerprint_for_policy(&state, &policy, &left, None),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left, None),
        action_fingerprint_for_policy(&state, &policy, &right, None)
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
        action_fingerprint_for_policy(&state, &policy, &left, None),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left, None),
        action_fingerprint_for_policy(&state, &policy, &right, None)
    );
}

#[test]
fn registry_idempotency_guard_switches_external_capability_to_action_fingerprint() {
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
        action_fingerprint_for_policy(&state, &policy, &left, None),
        "skill:system_basic:action:run_cmd"
    );
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left, None),
        action_fingerprint_for_policy(&state, &policy, &right, None)
    );
}

#[test]
fn registry_idempotency_guard_keeps_literal_execution_failed_step_run_cmd_args_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["system_basic"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let route = route_with_contract(
        OutputSemanticKind::ExecutionFailedStep,
        OutputLocatorKind::None,
    );
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
        action_fingerprint_for_policy(&state, &policy, &left, Some(&route)),
        action_fingerprint(&state, &left)
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left, Some(&route)),
        action_fingerprint_for_policy(&state, &policy, &right, Some(&route))
    );
    assert!(super::registry_idempotency_guard_attribution(
        &state,
        &policy,
        &left,
        Some(&route),
        &action_fingerprint_for_policy(&state, &policy, &left, Some(&route)),
        "registry_idempotency_repeat_completed_action",
        Some(1),
        None,
    )
    .is_none());

    let non_failed_step_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::None);
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left, Some(&non_failed_step_route)),
        "skill:system_basic:action:run_cmd"
    );
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
    let fingerprint = action_fingerprint_for_policy(&state, &policy, &action, None);

    assert!(super::registry_idempotency_guard_attribution(
        &state,
        &policy,
        &action,
        None,
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
semantic_route_authority = "agent_loop_canary"
agent_loop_canary_bucket = "structured_field_read"
registry_idempotency_guard_scope = "all"
structured_evidence_required_for_selected_contracts = true
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
        policy.semantic_route_authority,
        SemanticRouteAuthority::AgentLoopCanary
    );
    assert!(policy.records_agent_decides_attribution());
    assert_eq!(policy.agent_loop_canary_bucket, "structured_field_read");
    assert_eq!(
        policy.effective_registry_idempotency_guard_scope(),
        RegistryIdempotencyGuardScope::All
    );
    assert!(policy.structured_evidence_required_for_selected_contracts);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn answer_verifier_required_scope_accepts_selected_agent_loop_token() {
    let root = temp_support_workspace("answer-verifier-scope-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
semantic_route_authority = "agent_loop_default"
answer_verifier_enforce_required_scope = "selected_agent_loop"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_answer_verifier_required_evidence_scope(),
        AnswerVerifierRequiredEvidenceScope::SelectedAgentLoop
    );
    assert!(policy
        .enabled_rollout_switches()
        .contains(&"answer_verifier_enforce_required_scope"));

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
semantic_route_authority = "agent_loop_default"
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
fn answer_verifier_required_scope_only_enables_selected_agent_loop_routes() {
    let mut policy = base_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopDefault;
    policy.answer_verifier_enforce_required_scope =
        AnswerVerifierRequiredEvidenceScope::SelectedAgentLoop;
    let selected_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    let mut blocked_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    blocked_route.risk_ceiling = RiskCeiling::High;

    assert!(policy.answer_verifier_required_evidence_enabled_for_route(Some(&selected_route)));
    assert!(!policy.answer_verifier_required_evidence_enabled_for_route(Some(&blocked_route)));
    assert!(!policy.answer_verifier_required_evidence_enabled_for_route(None));
}

#[test]
fn answer_verifier_required_scope_all_enables_all_routes() {
    let mut policy = base_policy();
    policy.answer_verifier_enforce_required_scope = AnswerVerifierRequiredEvidenceScope::All;
    let selected_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    let mut high_risk_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    high_risk_route.risk_ceiling = RiskCeiling::High;

    assert!(policy.answer_verifier_required_evidence_enabled_for_route(Some(&selected_route)));
    assert!(policy.answer_verifier_required_evidence_enabled_for_route(Some(&high_risk_route)));
    assert!(policy.answer_verifier_required_evidence_enabled_for_route(None));
}

#[test]
fn registry_idempotency_guard_scope_accepts_selected_agent_loop_token() {
    let root = temp_support_workspace("registry-scope-config");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
semantic_route_authority = "agent_loop_default"
registry_idempotency_guard_scope = "selected_agent_loop"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.effective_registry_idempotency_guard_scope(),
        RegistryIdempotencyGuardScope::SelectedAgentLoop
    );
    assert!(policy
        .enabled_rollout_switches()
        .contains(&"registry_idempotency_guard_scope"));

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
semantic_route_authority = "agent_loop_default"
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
fn registry_idempotency_guard_scope_only_changes_selected_agent_loop_routes() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit"]);
    let mut policy = base_policy();
    policy.semantic_route_authority = SemanticRouteAuthority::AgentLoopDefault;
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::SelectedAgentLoop;
    let selected_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    let mut blocked_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    blocked_route.risk_ceiling = RiskCeiling::High;
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

    assert!(policy.registry_idempotency_guard_enabled_for_route(Some(&selected_route)));
    assert!(!policy.registry_idempotency_guard_enabled_for_route(Some(&blocked_route)));
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left, Some(&selected_route)),
        "skill:config_edit:action:apply_config_change"
    );
    assert_eq!(
        action_fingerprint_for_policy(&state, &policy, &left, Some(&selected_route)),
        action_fingerprint_for_policy(&state, &policy, &right, Some(&selected_route))
    );
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &left, Some(&blocked_route)),
        action_fingerprint_for_policy(&state, &policy, &right, Some(&blocked_route))
    );
}

#[test]
fn registry_idempotency_guard_scope_all_enables_all_routes() {
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let selected_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    let mut high_risk_route =
        route_with_contract(OutputSemanticKind::StructuredKeys, OutputLocatorKind::Path);
    high_risk_route.risk_ceiling = RiskCeiling::High;

    assert!(policy.registry_idempotency_guard_enabled_for_route(Some(&selected_route)));
    assert!(policy.registry_idempotency_guard_enabled_for_route(Some(&high_risk_route)));
    assert!(policy.registry_idempotency_guard_enabled_for_route(None));
}

#[test]
fn semantic_route_authority_accepts_machine_tokens() {
    for (token, expected, records, agent_authority) in [
        ("legacy", SemanticRouteAuthority::Legacy, false, false),
        ("shadow", SemanticRouteAuthority::Shadow, true, false),
        (
            "agent_loop_canary",
            SemanticRouteAuthority::AgentLoopCanary,
            true,
            true,
        ),
        (
            "agent_loop_default",
            SemanticRouteAuthority::AgentLoopDefault,
            true,
            true,
        ),
    ] {
        let root = temp_support_workspace(&format!("semantic-authority-{token}"));
        let config_dir = root.join("configs");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::write(
            config_dir.join("agent_guard.toml"),
            format!(
                r#"
[agent.loop_guard]
semantic_route_authority = "{token}"
"#
            ),
        )
        .expect("write agent guard config");
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root.clone();

        let policy = load_agent_loop_guard_policy(&state);

        assert_eq!(policy.semantic_route_authority, expected);
        assert_eq!(policy.records_agent_decides_attribution(), records);
        assert_eq!(policy.uses_agent_loop_semantic_authority(), agent_authority);
        let _ = std::fs::remove_dir_all(root);
    }
}

#[test]
fn semantic_route_authority_rejects_freeform_text() {
    let root = temp_support_workspace("semantic-authority-invalid");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
semantic_route_authority = "let the agent decide from user text"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.semantic_route_authority,
        SemanticRouteAuthority::Legacy
    );
    assert!(!policy.records_agent_decides_attribution());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn agent_loop_canary_bucket_rejects_unknown_tokens() {
    let root = temp_support_workspace("agent-loop-canary-bucket-invalid");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
agent_loop_canary_bucket = "freeform_user_phrase"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(policy.agent_loop_canary_bucket, "none");

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn legacy_agent_decides_config_keys_are_ignored() {
    let root = temp_support_workspace("legacy-agent-decides-config-ignored");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    std::fs::write(
        config_dir.join("agent_guard.toml"),
        r#"
[agent.loop_guard]
agent_decides_semantic_route = true
agent_decides_migration_class = "structured_field_read"
"#,
    )
    .expect("write agent guard config");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();

    let policy = load_agent_loop_guard_policy(&state);

    assert_eq!(
        policy.semantic_route_authority,
        SemanticRouteAuthority::Legacy
    );
    assert_eq!(policy.agent_loop_canary_bucket, "none");
    assert!(!policy.records_agent_decides_attribution());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn agent_loop_canary_bucket_accepts_low_risk_tokens_only() {
    for token in [
        "none",
        "bound_path_summary",
        "structured_field_read",
        "exact_path_list",
        "recent_artifacts_judgment",
        "scalar_count",
        "low_risk_status_observation",
        "low_risk_config_read",
        "low_risk_log_observation",
        "low_risk_workspace_question",
        "low_risk_tool_discovery",
        "low_risk_single_file_delivery",
    ] {
        let root = temp_support_workspace(&format!("agent-decides-class-{token}"));
        let config_dir = root.join("configs");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::write(
            config_dir.join("agent_guard.toml"),
            format!(
                r#"
[agent.loop_guard]
agent_loop_canary_bucket = "{token}"
"#
            ),
        )
        .expect("write agent guard config");
        let mut state = crate::AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root.clone();

        let policy = load_agent_loop_guard_policy(&state);

        assert_eq!(policy.agent_loop_canary_bucket, token);
        let _ = std::fs::remove_dir_all(root);
    }
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
        RegistryIdempotencyGuardScope::Off
    );

    let _ = std::fs::remove_dir_all(root);
}

fn route_with_contract(
    semantic_kind: OutputSemanticKind,
    locator_kind: OutputLocatorKind,
) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
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
fn route_contract_selects_grounded_summary_budget() {
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
fn workspace_delivery_contract_selects_multi_step_budget() {
    let policy = base_policy();
    let recipe = ExecutionRecipeRuntimeState::default();
    let mut route = route_with_contract(
        OutputSemanticKind::GeneratedFileDelivery,
        OutputLocatorKind::Filename,
    );
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.response_shape = OutputResponseShape::FileToken;

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
