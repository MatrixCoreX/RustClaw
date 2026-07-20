use serde_json::Value;

use crate::{agent_engine::LoopState, AgentAction, IntentOutputContract};

pub(in crate::agent_engine) fn observation_round_needs_planner(
    output_contract: &IntentOutputContract,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    loop_state.round_no < loop_state.max_rounds
        && observe_only_round_should_continue(output_contract, loop_state, actions)
}

pub(super) fn read_observe_round_should_continue(
    output_contract: &IntentOutputContract,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    observe_only_round_should_continue(output_contract, loop_state, actions)
        && actions.iter().any(action_reads_text_content)
}

pub(in crate::agent_engine) fn observe_only_round_should_continue(
    _output_contract: &IntentOutputContract,
    loop_state: &LoopState,
    actions: &[AgentAction],
) -> bool {
    loop_state.round_no < loop_state.max_rounds
        && !super::has_discussion_followup_action(actions)
        && !super::has_authoritative_delivery(loop_state)
        && actions_are_observe_only_machine_steps(actions)
}

fn action_reads_text_content(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            let tool = tool.trim();
            if matches!(tool, "read_file" | "read_range") {
                return true;
            }
            let action = args
                .get("action")
                .and_then(Value::as_str)
                .map(normalize_machine_action_token)
                .unwrap_or_default();
            matches!(tool, "fs_basic" | "system_basic")
                && matches!(
                    action.as_str(),
                    "read_range" | "read_text" | "read_text_range"
                )
        }
        AgentAction::CallCapability { capability, .. } => matches!(
            capability.trim().to_ascii_lowercase().as_str(),
            "filesystem.read_range"
                | "filesystem.read_text_range"
                | "filesystem.read_text"
                | "filesystem.read_file"
                | "fs_basic.read_range"
                | "fs_basic.read_text_range"
                | "fs_basic.read_text"
                | "fs_basic.read_file"
        ),
        AgentAction::Think { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. } => false,
    }
}

fn actions_are_observe_only_machine_steps(actions: &[AgentAction]) -> bool {
    let mut saw_observe = false;
    for action in actions {
        match action {
            AgentAction::Think { .. } => {}
            AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
                if !machine_tool_action_is_observe_only(tool, args) {
                    return false;
                }
                saw_observe = true;
            }
            AgentAction::CallCapability { capability, .. } => {
                if !machine_capability_is_observe_only(capability) {
                    return false;
                }
                saw_observe = true;
            }
            AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. } => return false,
        }
    }
    saw_observe
}

fn machine_capability_is_observe_only(capability: &str) -> bool {
    matches!(
        capability.trim().to_ascii_lowercase().as_str(),
        "filesystem.stat_paths"
            | "filesystem.stat_path"
            | "filesystem.list_entries"
            | "filesystem.list_dir"
            | "filesystem.list_names"
            | "filesystem.list_file_names"
            | "filesystem.list_directory_names"
            | "filesystem.count_entries"
            | "filesystem.read_text_range"
            | "filesystem.read_text"
            | "filesystem.read_file"
            | "filesystem.find_entries"
            | "filesystem.find_files"
            | "filesystem.find_paths"
            | "filesystem.grep_text"
            | "filesystem.search_text"
            | "filesystem.compare_paths"
            | "fs_basic.list_dir"
            | "fs_basic.read_range"
            | "fs_basic.read_text"
            | "fs_basic.read_text_range"
            | "fs_basic.find_entries"
            | "fs_basic.grep_text"
            | "fs_basic.metadata"
            | "fs_basic.stat"
    )
}

fn machine_tool_action_is_observe_only(tool: &str, args: &Value) -> bool {
    let tool = tool.trim();
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_machine_action_token)
        .unwrap_or_default();
    match tool {
        "fs_basic" => matches!(
            action.as_str(),
            "list_dir"
                | "read_range"
                | "read_text"
                | "read_text_range"
                | "grep_text"
                | "find_entries"
                | "metadata"
                | "stat"
        ),
        "system_basic" => matches!(
            action.as_str(),
            "list_dir" | "read_range" | "read_text" | "read_text_range" | "stat"
        ),
        "list_dir" | "read_file" | "read_range" | "fs_search" => true,
        _ => false,
    }
}

fn normalize_machine_action_token(action: &str) -> String {
    action
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}
