use super::*;
use serde_json::json;

#[test]
fn command_fingerprint_preserves_shell_content() {
    let state = state_with_registry(registry_governance_fixture(), &["run_cmd"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    for (left, right) in [
        ("printf 'a  b'", "printf 'a b'"),
        ("printf 'a\nb'", "printf 'a b'"),
        ("printf one\nprintf two", "printf one printf two"),
        (
            "printf 'https://example.test/a//b'",
            "printf 'https://example.test/a/b'",
        ),
        ("printf './value'", "printf 'value'"),
        (
            "printf a # comment\nprintf b",
            "printf a # comment printf b",
        ),
    ] {
        let action = |command| crate::AgentAction::CallSkill {
            skill: "run_cmd".to_string(),
            args: json!({"command": command}),
        };
        assert_ne!(
            action_fingerprint_for_policy(&state, &policy, &action(left)),
            action_fingerprint_for_policy(&state, &policy, &action(right)),
            "distinct commands must not collide: {left:?} / {right:?}"
        );
        assert_eq!(
            action_fingerprint_for_policy(&state, &policy, &action(left)),
            action_fingerprint_for_policy(&state, &policy, &action(left))
        );
    }
}

fn subagent_args() -> serde_json::Value {
    json!({
        "action": "inline_readonly", "role": "review",
        "objective": "Review authentication for unauthorized access",
        "context_refs": ["src/lib.rs", "AGENTS.md"],
        "allowed_capabilities": ["filesystem.read_text"],
        "result_contract": {"fields": ["aligned", "conflicts"]}
    })
}

#[test]
fn inline_subagent_fingerprint_preserves_the_complete_delegation() {
    let state = state_with_registry(registry_governance_fixture(), &["subagent"]);
    let action = |args| crate::AgentAction::CallSkill {
        skill: "subagent".into(),
        args,
    };
    let original = subagent_args();
    let expected = action_fingerprint(&state, &action(original.clone()));
    for (field, value) in [
        ("objective", json!("Review authentication performance")),
        ("allowed_capabilities", json!(["filesystem.stat_path"])),
        ("result_contract", json!({"fields": ["performance_risks"]})),
        ("context_refs", json!(["src/api.rs", "AGENTS.md"])),
        ("timeout_seconds", json!(30)),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        assert_ne!(
            expected,
            action_fingerprint(&state, &action(changed)),
            "field={field}"
        );
    }
    let mut equivalent = original;
    equivalent["role"] = json!("reviewer");
    equivalent["context_refs"] = json!(["./AGENTS.md", "src/lib.rs", "src/lib.rs"]);
    assert_eq!(expected, action_fingerprint(&state, &action(equivalent)));
}

#[test]
fn planner_subagent_capability_keeps_distinct_objectives() {
    let state = state_with_registry(registry_governance_fixture(), &["subagent"]);
    let mut policy = base_policy();
    policy.registry_idempotency_guard_scope = RegistryIdempotencyGuardScope::All;
    let action = |args| crate::AgentAction::CallCapability {
        capability: "agent.subagent".into(),
        args,
    };
    let first = subagent_args();
    let mut second = first.clone();
    second["objective"] = json!("Check memory use in the same file");
    assert_ne!(
        action_fingerprint_for_policy(&state, &policy, &action(first)),
        action_fingerprint_for_policy(&state, &policy, &action(second))
    );
}

#[test]
fn unrecognized_subagent_roles_do_not_share_a_fingerprint() {
    let state = state_with_registry(registry_governance_fixture(), &["subagent"]);
    let action = |role| {
        let mut args = subagent_args();
        args["role"] = json!(role);
        crate::AgentAction::CallSkill {
            skill: "subagent".into(),
            args,
        }
    };
    assert_ne!(
        action_fingerprint(&state, &action("unknown_a")),
        action_fingerprint(&state, &action("unknown_b"))
    );
}

#[test]
fn roles_in_the_same_family_keep_distinct_permission_contracts() {
    let state = state_with_registry(registry_governance_fixture(), &["subagent"]);
    let action = |role| {
        let mut args = subagent_args();
        args["role"] = json!(role);
        crate::AgentAction::CallSkill {
            skill: "subagent".into(),
            args,
        }
    };
    assert_ne!(
        action_fingerprint(&state, &action("worker")),
        action_fingerprint(&state, &action("writer"))
    );
}
