use super::*;
use claw_core::skill_registry::SkillsRegistry;
use std::sync::{Arc, RwLock};

fn state_with_batch_registry() -> AppState {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-action-batch-contract-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("create registry fixture root");
    let registry_path = root.join("skills_registry.toml");
    std::fs::write(
        &registry_path,
        r#"
[[skills]]
name = "batch_fixture"
enabled = true
kind = "runner"
input_schema = { type = "object", properties = { path = { type = "string" }, content = { type = "string" } } }
planner_capabilities = [
  { name = "batch.read", action = "read", effect = "observe", required = ["path"], idempotent = true },
  { name = "batch.write", action = "write", effect = "mutate", required = ["path", "content"], idempotent = false, once_per_task = true },
  { name = "batch.validate", action = "validate", effect = "validate", required = ["path"], idempotent = true },
]
"#,
    )
    .expect("write registry fixture");
    let registry =
        Arc::new(SkillsRegistry::load_from_path(&registry_path).expect("load registry fixture"));
    let _ = std::fs::remove_dir_all(root);
    let mut state = AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(crate::SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(["batch_fixture".to_string()].into_iter().collect()),
    })));
    state
}

fn read(path: &str) -> AgentAction {
    AgentAction::CallCapability {
        capability: "batch.read".to_string(),
        args: serde_json::json!({"path": path}),
    }
}

fn write(path: &str) -> AgentAction {
    AgentAction::CallCapability {
        capability: "batch.write".to_string(),
        args: serde_json::json!({"path": path, "content": "updated"}),
    }
}

#[test]
fn independent_reads_share_a_dependency_frontier() {
    let state = state_with_batch_registry();
    let actions = vec![
        read("a.txt"),
        read("b.txt"),
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["step_1".to_string(), "step_2".to_string()],
        },
    ];

    assert_eq!(
        planner_action_dependencies(Some(&state), &actions),
        vec![
            Vec::<String>::new(),
            Vec::<String>::new(),
            vec!["step_1".to_string(), "step_2".to_string()],
        ]
    );
}

#[test]
fn parallel_prefix_contains_only_consecutive_independent_reads() {
    let state = state_with_batch_registry();
    let actions = vec![
        read("a.txt"),
        read("b.txt"),
        write("target.txt"),
        read("c.txt"),
    ];

    assert_eq!(
        independent_read_batch_prefix_len(&state, &actions, actions.len()),
        2
    );
    assert_eq!(independent_read_batch_prefix_len(&state, &actions, 1), 0);
}

#[test]
fn mutation_and_post_mutation_reads_form_material_boundaries() {
    let state = state_with_batch_registry();
    let actions = vec![
        read("before-a.txt"),
        read("before-b.txt"),
        write("target.txt"),
        read("after-a.txt"),
        read("after-b.txt"),
        AgentAction::Respond {
            content: "done".to_string(),
        },
    ];

    assert_eq!(
        planner_action_dependencies(Some(&state), &actions),
        vec![
            vec![],
            vec![],
            vec!["step_1".to_string(), "step_2".to_string()],
            vec!["step_3".to_string()],
            vec!["step_3".to_string()],
            vec!["step_4".to_string(), "step_5".to_string()],
        ]
    );
}

#[test]
fn runtime_returns_control_after_read_batch_or_material_action() {
    let state = state_with_batch_registry();
    let reads = vec![read("a.txt"), read("b.txt"), write("target.txt")];
    assert_eq!(
        return_control_boundary_after_action(&state, &reads, 0, 8),
        None
    );
    assert_eq!(
        return_control_boundary_after_action(&state, &reads, 1, 8),
        Some("independent_read_batch_observed")
    );
    assert_eq!(
        return_control_boundary_after_action(&state, &reads, 2, 8),
        Some("material_action_observed")
    );
}

#[test]
fn unresolved_observation_reference_is_never_batchable() {
    let state = state_with_batch_registry();
    let dependent = AgentAction::CallCapability {
        capability: "batch.read".to_string(),
        args: serde_json::json!({"path": "{{last_output.path}}"}),
    };

    assert_eq!(
        return_control_boundary_after_action(&state, &[dependent], 0, 8),
        Some("material_action_observed")
    );
}
