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
            agents_by_id: Arc::new(std::sync::RwLock::new(Arc::new(agents_by_id))),
            agent_runtime_leases: Arc::new(std::sync::RwLock::new(HashMap::new())),
            skill_views_snapshot: Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
                binding: Default::default(),
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
        binding: Default::default(),
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

#[cfg(unix)]
#[test]
fn approval_scope_is_stable_when_creation_is_anchored_through_a_root_alias() {
    use std::os::unix::fs::symlink;

    let mut state = state_with_workspace_registry();
    let fixture_root = std::env::temp_dir().join(format!(
        "agent-runtime-approval-root-alias-{}",
        uuid::Uuid::new_v4().simple()
    ));
    let canonical_root = fixture_root.join("canonical");
    let configured_root = fixture_root.join("configured");
    std::fs::create_dir_all(&canonical_root).expect("create canonical workspace");
    symlink(&canonical_root, &configured_root).expect("create configured workspace alias");
    state.skill_rt.workspace_root = configured_root.clone();
    let ids = vec!["step-1".to_string()];
    let relative = PlanStep {
        step_id: "step-1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "fs_basic".to_string(),
        args: json!({
            "action": "write_text",
            "path": "run/example.txt",
            "content": "updated"
        }),
        depends_on: Vec::new(),
        why: String::new(),
    };
    let anchored = PlanStep {
        args: json!({
            "action": "write_text",
            "path": configured_root.join("run/example.txt"),
            "content": "updated"
        }),
        ..relative.clone()
    };

    let relative = binding_for_confirmation_steps(&state, &[relative], &ids)
        .expect("relative approval binding");
    let anchored = binding_for_confirmation_steps(&state, &[anchored], &ids)
        .expect("anchored approval binding");
    std::fs::remove_dir_all(&fixture_root).expect("remove alias fixture");

    assert_eq!(relative.scope, anchored.scope);
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
fn approval_binding_treats_explicit_workspace_cwd_as_implicit_default() {
    let state = state_with_workspace_registry();
    let ids = vec!["step-1".to_string()];
    let implicit = PlanStep {
        step_id: "step-1".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({"command": "printf approved"}),
        depends_on: Vec::new(),
        why: String::new(),
    };
    let explicit = PlanStep {
        args: json!({
            "command": "printf approved",
            "cwd": state.skill_rt.workspace_root
        }),
        ..implicit.clone()
    };

    let implicit =
        binding_for_confirmation_steps(&state, &[implicit], &ids).expect("implicit cwd binding");
    let explicit =
        binding_for_confirmation_steps(&state, &[explicit], &ids).expect("explicit cwd binding");

    assert_eq!(implicit, explicit);
}

#[test]
fn approval_binding_rejects_changed_command_working_directory() {
    let state = state_with_workspace_registry();
    let ids = vec!["step-1".to_string()];
    let root = PlanStep {
        step_id: "step-1".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({"command": "printf approved"}),
        depends_on: Vec::new(),
        why: String::new(),
    };
    let changed = PlanStep {
        args: json!({
            "command": "printf approved",
            "cwd": state.skill_rt.workspace_root.join("different")
        }),
        ..root.clone()
    };

    let root = binding_for_confirmation_steps(&state, &[root], &ids).expect("root cwd binding");
    let changed =
        binding_for_confirmation_steps(&state, &[changed], &ids).expect("changed cwd binding");

    assert_eq!(root.action_fingerprint, changed.action_fingerprint);
    assert_ne!(root.arguments_hash, changed.arguments_hash);
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
