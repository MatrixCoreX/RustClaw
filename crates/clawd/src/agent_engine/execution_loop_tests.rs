use super::{
    action_counts_as_tool_call, action_effect_is_repeatable_for_active_recipe,
    capture_round_progress_snapshot, check_repeat_action_guard, finalize_execute_round_outcome,
    terminal_synthesis_can_skip_remaining_actions,
};
use crate::agent_engine::action_fingerprint_for_policy;
use crate::agent_engine::support::{
    AnswerVerifierRequiredEvidenceScope, RegistryIdempotencyGuardScope,
};
use claw_core::skill_registry::SkillsRegistry;
use std::sync::{Arc, RwLock};

fn test_policy(registry_idempotency_guard_enabled: bool) -> super::AgentLoopGuardPolicy {
    super::AgentLoopGuardPolicy {
        max_steps: 8,
        max_rounds: 2,
        max_tool_calls: 12,
        recoverable_failure_extra_rounds: 0,
        repeat_action_limit: 1,
        no_progress_limit: 1,
        multi_round_enabled: true,
        answer_verifier_retry_limit: 1,
        answer_verifier_enforce_required_scope: AnswerVerifierRequiredEvidenceScope::Off,
        registry_idempotency_guard_scope: if registry_idempotency_guard_enabled {
            RegistryIdempotencyGuardScope::All
        } else {
            RegistryIdempotencyGuardScope::Off
        },
        fast_read: Default::default(),
        grounded_summary: Default::default(),
        multi_step_workspace: Default::default(),
        ops_closed_loop: Default::default(),
    }
}

fn task_fixture(id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: id.to_string(),
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

fn state_with_registry(toml: &str, skills: &[&str]) -> crate::AppState {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-execution-loop-registry-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("create temp registry dir");
    let path = root.join("skills_registry.toml");
    std::fs::write(&path, toml).expect("write registry");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry"));
    let _ = std::fs::remove_dir_all(root);
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(crate::SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(skills.iter().map(|skill| (*skill).to_string()).collect()),
    })));
    state
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
"#
}

fn filesystem_write_registry_fixture() -> &'static str {
    r#"
[[skills]]
name = "fs_basic"
enabled = true
kind = "runner"
planner_capabilities = [
  { name = "filesystem.write_text", action = "write_text", effect = "mutate", required = ["path", "content"], risk_level = "high", once_per_task = true, dedup_scope = "args", idempotent = false },
]
"#
}

#[test]
fn observed_output_alone_does_not_mark_plan_exhausted_user_visible() {
    let loop_state = super::LoopState::new(2);
    let snapshot = capture_round_progress_snapshot(&loop_state);
    let outcome = finalize_execute_round_outcome(&loop_state, &snapshot, 1, 1, false, None);
    assert!(outcome.stop_signal.is_none());
}

#[test]
fn explicit_user_visible_output_marks_plan_exhausted() {
    let loop_state = super::LoopState::new(2);
    let snapshot = capture_round_progress_snapshot(&loop_state);
    let outcome = finalize_execute_round_outcome(&loop_state, &snapshot, 1, 1, true, None);
    assert_eq!(
        outcome.stop_signal.as_deref(),
        Some("plan_exhausted_user_visible")
    );
}

#[test]
fn max_tool_call_budget_counts_only_external_calls() {
    assert!(action_counts_as_tool_call(&crate::AgentAction::CallTool {
        tool: "system_basic".to_string(),
        args: serde_json::json!({})
    }));
    assert!(action_counts_as_tool_call(&crate::AgentAction::CallSkill {
        skill: "fs_basic".to_string(),
        args: serde_json::json!({})
    }));
    assert!(action_counts_as_tool_call(
        &crate::AgentAction::CallCapability {
            capability: "fs_basic.read_text_range".to_string(),
            args: serde_json::json!({})
        }
    ));
    assert!(!action_counts_as_tool_call(
        &crate::AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()]
        }
    ));
    assert!(!action_counts_as_tool_call(&crate::AgentAction::Respond {
        content: "done".to_string()
    }));
}

#[test]
fn terminal_synthesis_skips_only_placeholder_delivery_suffix() {
    let mut loop_state = super::LoopState::new(2);
    loop_state.last_publishable_synthesis_output = Some(r#"{"test_status":"OK"}"#.to_string());
    let action = crate::AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["s4".to_string()],
    };

    assert!(terminal_synthesis_can_skip_remaining_actions(
        &action,
        &[
            crate::AgentAction::Think {
                content: "trace".to_string(),
            },
            crate::AgentAction::Respond {
                content: "{{ last_output }}".to_string(),
            },
        ],
        &loop_state,
    ));
}

#[test]
fn terminal_synthesis_skips_non_json_delivery_suffix_for_strict_json_contract() {
    let mut loop_state = super::LoopState::new(2);
    loop_state.last_publishable_synthesis_output = Some(r#"{"test_status":"OK"}"#.to_string());
    loop_state.output_contract = Some(crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        ..Default::default()
    });
    let action = crate::AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["s4".to_string()],
    };

    assert!(terminal_synthesis_can_skip_remaining_actions(
        &action,
        &[
            crate::AgentAction::Respond {
                content: "{{ last_output }}".to_string(),
            },
            crate::AgentAction::Respond {
                content: "FILE:/workspace/test_calc_core.py".to_string(),
            },
        ],
        &loop_state,
    ));
}

#[test]
fn terminal_synthesis_does_not_skip_concrete_or_executable_suffix() {
    let mut loop_state = super::LoopState::new(2);
    loop_state.last_publishable_synthesis_output = Some(r#"{"test_status":"OK"}"#.to_string());
    let action = crate::AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["s4".to_string()],
    };

    assert!(!terminal_synthesis_can_skip_remaining_actions(
        &action,
        &[crate::AgentAction::Respond {
            content: r#"{"test_status":"OK"}"#.to_string(),
        }],
        &loop_state,
    ));
    assert!(!terminal_synthesis_can_skip_remaining_actions(
        &action,
        &[crate::AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({"action":"read_range"}),
        }],
        &loop_state,
    ));
}

#[test]
fn active_recipe_allows_repeating_successful_observe_effect() {
    let recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    assert!(action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::observe(),
    ));
    assert!(action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::validate(),
    ));
    assert!(!action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::mutate(),
    ));
}

#[test]
fn done_recipe_does_not_allow_repeating_successful_observe_effect() {
    let mut recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    recipe.phase = crate::execution_recipe::ExecutionRecipePhase::Done;
    assert!(!action_effect_is_repeatable_for_active_recipe(
        recipe,
        crate::execution_recipe::ActionEffect::observe(),
    ));
}

#[test]
fn repeat_guard_allows_repeated_respond_delivery() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task_fixture("task-repeat-respond");
    let mut loop_state = super::LoopState::new(2);
    let action = crate::AgentAction::Respond {
        content: "final answer".to_string(),
    };
    let fingerprint = "respond:final answer".to_string();
    loop_state
        .successful_action_fingerprints
        .insert(fingerprint.clone(), 1);
    let policy = test_policy(false);

    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            None,
            &fingerprint,
            1,
        ),
        None
    );
}

#[test]
fn repeat_guard_blocks_identical_non_respond_after_limit() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task_fixture("task-repeat-run-cmd");
    let mut loop_state = super::LoopState::new(2);
    let action = crate::AgentAction::CallSkill {
        skill: "run_cmd".to_string(),
        args: serde_json::json!({"command": "pwd"}),
    };
    let fingerprint = "skill:run_cmd:{\"command\":\"pwd\"}".to_string();
    let policy = test_policy(false);

    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            None,
            &fingerprint,
            1,
        ),
        None
    );
    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            None,
            &fingerprint,
            2,
        )
        .as_deref(),
        Some("repeat_action_limit")
    );
}

#[test]
fn repeat_guard_allows_successful_observe_repeat_until_limit() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = task_fixture("task-repeat-observe");
    let mut loop_state = super::LoopState::new(2);
    let action = crate::AgentAction::CallSkill {
        skill: "git_basic".to_string(),
        args: serde_json::json!({"action": "status"}),
    };
    let mut policy = test_policy(false);
    policy.repeat_action_limit = 2;
    let fingerprint = action_fingerprint_for_policy(&state, &policy, &action, None);
    loop_state
        .successful_action_fingerprints
        .insert(fingerprint.clone(), 1);

    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            None,
            &fingerprint,
            1,
        ),
        None
    );
    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            None,
            &fingerprint,
            2,
        ),
        None
    );
    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            None,
            &fingerprint,
            3,
        )
        .as_deref(),
        Some("repeat_action_limit")
    );
}

#[test]
fn registry_idempotency_guard_records_repeat_block_attribution() {
    let state = state_with_registry(registry_governance_fixture(), &["config_edit"]);
    let task = task_fixture("task-registry-repeat");
    let mut loop_state = super::LoopState::new(2);
    let action = crate::AgentAction::CallSkill {
        skill: "config_edit".to_string(),
        args: serde_json::json!({
            "action": "apply_config_change",
            "field_path": "skills.a",
            "value": true
        }),
    };
    let fingerprint = "skill:config_edit:action:apply_config_change".to_string();
    loop_state
        .successful_action_fingerprints
        .insert(fingerprint.clone(), 1);
    let policy = test_policy(true);

    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &action,
            None,
            &fingerprint,
            1,
        )
        .as_deref(),
        Some("repeat_completed_action")
    );

    let attribution = loop_state
        .rollout_attribution
        .first()
        .expect("registry attribution");
    assert_eq!(attribution.switch_name, "registry_idempotency_guard_scope");
    assert_eq!(attribution.event, "registry_idempotency_guard_block");
    assert_eq!(
        attribution.reason_code.as_deref(),
        Some("registry_idempotency_repeat_completed_action")
    );
    assert_eq!(attribution.skill.as_deref(), Some("config_edit"));
    assert_eq!(attribution.action.as_deref(), Some("apply_config_change"));
    assert_eq!(attribution.dedup_scope.as_deref(), Some("action"));
    assert_eq!(
        attribution.fingerprint.as_deref(),
        Some(fingerprint.as_str())
    );
    assert_eq!(attribution.repeat_count, Some(1));
    assert_eq!(
        attribution
            .boundary_context
            .as_ref()
            .and_then(|value| value.pointer("/decision_source"))
            .and_then(serde_json::Value::as_str),
        Some("safety_policy")
    );
}

#[test]
fn registry_args_dedup_allows_multiple_distinct_filesystem_writes() {
    let state = state_with_registry(filesystem_write_registry_fixture(), &["fs_basic"]);
    let task = task_fixture("task-registry-write-multiple-files");
    let mut loop_state = super::LoopState::new(2);
    let first = crate::AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "write_text",
            "path": "run/nl_eval_tmp/a.py",
            "content": "A"
        }),
    };
    let second = crate::AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "write_text",
            "path": "run/nl_eval_tmp/b.py",
            "content": "B"
        }),
    };
    let policy = test_policy(true);
    let first_fingerprint = action_fingerprint_for_policy(&state, &policy, &first, None);
    let second_fingerprint = action_fingerprint_for_policy(&state, &policy, &second, None);

    assert_ne!(first_fingerprint, second_fingerprint);
    assert!(first_fingerprint.contains("run/nl_eval_tmp/a.py"));
    assert!(second_fingerprint.contains("run/nl_eval_tmp/b.py"));

    loop_state
        .successful_action_fingerprints
        .insert(first_fingerprint, 1);

    assert_eq!(
        check_repeat_action_guard(
            &state,
            &task,
            &mut loop_state,
            &policy,
            &second,
            None,
            &second_fingerprint,
            2,
        ),
        None
    );
    assert!(loop_state.rollout_attribution.is_empty());
}
