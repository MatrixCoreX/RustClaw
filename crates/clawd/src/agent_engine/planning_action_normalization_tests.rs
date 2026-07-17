use serde_json::json;

use super::{
    annotate_readonly_cli_surface_args, collapse_redundant_drafting_synthesis,
    command_looks_like_readonly_cli_surface_probe,
};
use crate::AgentAction;

#[test]
fn readonly_help_probe_receives_machine_policy_fields() {
    let mut args = json!({"command": "cargo --help"});

    assert!(annotate_readonly_cli_surface_args(&mut args));
    assert_eq!(
        args.get("action").and_then(|value| value.as_str()),
        Some("inspect_cli_help")
    );
    assert_eq!(
        args.get("timeout_seconds").and_then(|value| value.as_i64()),
        Some(10)
    );
}

#[test]
fn explicit_action_is_not_overwritten() {
    let mut args = json!({"action": "run", "command": "cargo --help"});

    assert!(!annotate_readonly_cli_surface_args(&mut args));
    assert_eq!(
        args.get("action").and_then(|value| value.as_str()),
        Some("run")
    );
}

#[test]
fn mutating_shell_surface_is_not_marked_readonly() {
    assert!(!command_looks_like_readonly_cli_surface_probe(
        "sudo cargo --help"
    ));
    assert!(!command_looks_like_readonly_cli_surface_probe("rm --help"));
}

#[test]
fn pure_drafting_plan_drops_redundant_synthesis_before_literal_response() {
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["assistant[-1]".to_string()],
        },
        AgentAction::Respond {
            content: "complete literal answer".to_string(),
        },
    ];

    let normalized = collapse_redundant_drafting_synthesis(actions);
    assert_eq!(normalized.len(), 1);
    assert!(matches!(
        normalized.as_slice(),
        [AgentAction::Respond { content }] if content == "complete literal answer"
    ));
}

#[test]
fn synthesis_before_last_output_passthrough_is_preserved() {
    let actions = vec![
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];

    let normalized = collapse_redundant_drafting_synthesis(actions);
    assert_eq!(normalized.len(), 2);
    assert!(matches!(
        normalized.as_slice(),
        [
            AgentAction::SynthesizeAnswer { evidence_refs },
            AgentAction::Respond { content }
        ] if evidence_refs == &["last_output"] && content == "{{last_output}}"
    ));
}
