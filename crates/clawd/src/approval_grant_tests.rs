use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};

use claw_core::config::{AgentConfig, ToolsConfig};
use serde_json::json;

use super::*;
use crate::{AgentRuntimeConfig, SkillViewsSnapshot, ToolsPolicy, DEFAULT_AGENT_ID};

fn test_state() -> AppState {
    let agents_by_id = HashMap::from([(
        DEFAULT_AGENT_ID.to_string(),
        AgentRuntimeConfig::from_config(&AgentConfig::default(), Vec::new()),
    )]);
    AppState {
        core: crate::CoreServices {
            agents_by_id: Arc::new(agents_by_id),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                registry: None,
                skills_list: Arc::new(HashSet::new()),
            }))),
            ..crate::CoreServices::test_default()
        },
        skill_rt: crate::SkillRuntime {
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

fn step(args: serde_json::Value) -> PlanStep {
    PlanStep {
        step_id: "step-1".to_string(),
        action_type: "call_skill".to_string(),
        skill: "write_file".to_string(),
        args,
        depends_on: Vec::new(),
        why: String::new(),
    }
}

fn state_with_workspace_registry() -> AppState {
    let state = AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load workspace skills registry");
    let enabled = registry.enabled_names().into_iter().collect::<HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(enabled),
    });
    state
}

#[test]
fn approval_binding_is_stable_across_json_object_key_order() {
    let state = test_state();
    let left = step(json!({"path":"notes.txt","content":"alpha"}));
    let right = step(json!({"content":"alpha","path":"notes.txt"}));
    let ids = vec!["step-1".to_string()];

    let left = binding_for_confirmation_steps(&state, &[left], &ids).expect("left binding");
    let right = binding_for_confirmation_steps(&state, &[right], &ids).expect("right binding");

    assert_eq!(left, right);
}

#[test]
fn approval_binding_changes_when_arguments_change() {
    let state = test_state();
    let ids = vec!["step-1".to_string()];
    let left = binding_for_confirmation_steps(
        &state,
        &[step(json!({"path":"notes.txt","content":"alpha"}))],
        &ids,
    )
    .expect("left binding");
    let right = binding_for_confirmation_steps(
        &state,
        &[step(json!({"path":"notes.txt","content":"beta"}))],
        &ids,
    )
    .expect("right binding");

    assert_eq!(left.action_fingerprint, right.action_fingerprint);
    assert_ne!(left.arguments_hash, right.arguments_hash);
}

#[test]
fn approval_binding_is_stable_across_capability_resolution() {
    let state = state_with_workspace_registry();
    let ids = vec!["step-1".to_string()];
    let capability_step = PlanStep {
        step_id: "step-1".to_string(),
        action_type: "call_capability".to_string(),
        skill: "system.run_command".to_string(),
        args: json!({"command": "pwd"}),
        depends_on: Vec::new(),
        why: String::new(),
    };
    let resolved_step = PlanStep {
        step_id: "step-1".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({"command": "pwd"}),
        depends_on: Vec::new(),
        why: String::new(),
    };

    let capability =
        binding_for_confirmation_steps(&state, &[capability_step], &ids).expect("capability");
    let resolved =
        binding_for_confirmation_steps(&state, &[resolved_step], &ids).expect("resolved");

    assert_eq!(capability, resolved);
}

#[test]
fn approval_binding_ignores_runtime_owned_validation_metadata() {
    let state = state_with_workspace_registry();
    let ids = vec!["step-1".to_string()];
    let first = PlanStep {
        step_id: "step-1".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({
            "command": "python3 -m unittest test_calc.py",
            "_clawd_validation": {
                "profile": "python_unittest_v1",
                "validator_type": "test"
            }
        }),
        depends_on: Vec::new(),
        why: String::new(),
    };
    let replanned = PlanStep {
        args: json!({
            "command": "python3 -m unittest test_calc.py",
            "_clawd_validation": {
                "profile": "execution_recipe",
                "validator_type": "test",
                "validated_target": "test_calc.py"
            }
        }),
        ..first.clone()
    };

    let first = binding_for_confirmation_steps(&state, &[first], &ids).expect("first binding");
    let replanned =
        binding_for_confirmation_steps(&state, &[replanned], &ids).expect("replanned binding");

    assert_eq!(first, replanned);
}

#[test]
fn pending_request_is_task_bound_and_expiring() {
    let binding = ApprovalBinding {
        action_fingerprint: "sha256:action".to_string(),
        arguments_hash: "sha256:args".to_string(),
        action_count: 1,
        targets: vec!["write_file".to_string()],
        scope: None,
    };
    let request = pending_approval_request_json("task-1", &binding, 100);

    assert_eq!(request["task_id"], "task-1");
    assert_eq!(request["status"], "pending");
    assert_eq!(request["issued_at"], 100);
    assert_eq!(request["expires_at"], 100 + APPROVAL_GRANT_TTL_SECONDS);
    assert!(request["request_id"]
        .as_str()
        .is_some_and(|value| value.starts_with("approval-")));
    assert_eq!(
        request["allowed_decisions"],
        json!(["approve_once", "deny"])
    );
}

#[test]
fn approval_decision_protocol_is_closed_to_machine_tokens() {
    assert_eq!(
        ApprovalDecision::parse_token("approve_once"),
        Some(ApprovalDecision::ApproveOnce)
    );
    assert_eq!(
        ApprovalDecision::parse_token("always_for_scope"),
        Some(ApprovalDecision::AlwaysForScope)
    );
    assert_eq!(
        ApprovalDecision::parse_token("deny"),
        Some(ApprovalDecision::Deny)
    );
    assert_eq!(ApprovalDecision::parse_token("approve"), None);
    assert_eq!(ApprovalDecision::parse_token("yes"), None);
}
