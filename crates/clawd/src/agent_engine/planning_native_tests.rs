use claw_core::model_turn::{ModelFinishReason, ModelToolCall};

use super::*;

fn turn(tool_calls: Vec<ModelToolCall>, text: &str) -> ModelTurnResponse {
    ModelTurnResponse {
        text: text.to_string(),
        tool_calls,
        usage: None,
        finish_reason: ModelFinishReason::ToolCalls,
        reasoning_metadata: Default::default(),
        events: Vec::new(),
    }
}

#[test]
fn native_tool_call_maps_only_to_capability_action() {
    let actions = actions_from_native_turn(&turn(
        vec![ModelToolCall {
            id: "call-1".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({
                "capability": "fs.read",
                "args": {"path": "README.md"}
            }),
        }],
        "",
    ))
    .expect("native action");

    assert_eq!(actions.len(), 1);
    assert!(matches!(
        &actions[0],
        AgentAction::CallCapability { capability, args }
            if capability == "fs.read" && args["path"] == "README.md"
    ));
}

#[test]
fn native_text_is_a_terminal_model_response() {
    let actions = actions_from_native_turn(&turn(Vec::new(), "Done.")).expect("terminal action");

    assert!(matches!(
        &actions[0],
        AgentAction::Respond { content } if content == "Done."
    ));
}

#[test]
fn native_tool_rejects_unknown_protocol_name_and_invalid_args() {
    let unknown = turn(
        vec![ModelToolCall {
            id: "call-1".to_string(),
            name: "run_shell_directly".to_string(),
            arguments: json!({}),
        }],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&unknown).expect_err("unknown tool rejected"),
        "native_plan_unknown_tool"
    );

    let invalid = turn(
        vec![ModelToolCall {
            id: "call-2".to_string(),
            name: "call_capability".to_string(),
            arguments: json!({"capability": "fs.read", "args": "README.md"}),
        }],
        "",
    );
    assert_eq!(
        actions_from_native_turn(&invalid).expect_err("invalid args rejected"),
        "native_plan_args_not_object"
    );
}
