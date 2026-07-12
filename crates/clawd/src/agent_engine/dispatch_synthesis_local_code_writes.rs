use serde_json::Value;

use crate::agent_engine::LoopState;
use crate::AgentAction;

pub(super) fn successful_fs_write_paths(loop_state: &LoopState) -> Vec<String> {
    let mut paths = Vec::new();
    for payload in local_code_write_payloads(loop_state) {
        push_payload_path(&payload, &mut paths);
    }
    paths
}

pub(super) fn successful_fs_changed_write_paths(loop_state: &LoopState) -> Vec<String> {
    let mut paths = Vec::new();
    for payload in local_code_write_payloads(loop_state) {
        if payload
            .get("changed")
            .and_then(Value::as_bool)
            .is_some_and(|changed| !changed)
        {
            continue;
        }
        push_payload_path(&payload, &mut paths);
    }
    paths
}

pub(super) fn successful_fs_write_readbacks_from_plan_trace(
    loop_state: &LoopState,
    current_write_paths: &[String],
) -> Vec<super::FsReadback> {
    if current_write_paths.is_empty() {
        return Vec::new();
    }

    let mut readbacks = Vec::new();
    for round in &loop_state.round_traces {
        let Some(plan) = round.plan_result.as_ref() else {
            continue;
        };
        for action in plan
            .steps
            .iter()
            .filter_map(crate::PlanStep::to_agent_action)
        {
            let Some((path, content)) = write_text_content_from_action(&action) else {
                continue;
            };
            let Some(matched_path) = current_write_paths
                .iter()
                .find(|write_path| super::projection_paths_match(&path, write_path))
                .cloned()
            else {
                continue;
            };
            if !super::path_looks_like_code_or_test_file(&matched_path) {
                continue;
            }
            replace_or_push_readback(&mut readbacks, matched_path, content);
        }
    }
    readbacks
}

fn local_code_write_payloads(loop_state: &LoopState) -> Vec<Value> {
    loop_state
        .executed_step_results
        .iter()
        .filter(|step| step.is_ok() && super::filesystem_projection_skill(&step.skill))
        .filter_map(|step| step.output.as_deref())
        .filter_map(super::skill_output_payload)
        .filter(|payload| {
            matches!(
                payload.get("action").and_then(Value::as_str),
                Some("write_text" | "append_text")
            )
        })
        .collect()
}

fn push_payload_path(payload: &Value, paths: &mut Vec<String>) {
    for key in ["effective_path", "resolved_path", "path"] {
        if let Some(path) = payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            super::push_unique_string(paths, path);
            break;
        }
    }
}

fn write_text_content_from_action(action: &AgentAction) -> Option<(String, String)> {
    match action {
        AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
            write_text_content_from_tool_like_action(tool, args)
        }
        AgentAction::CallCapability { capability, args } => {
            write_text_content_from_capability_action(capability, args)
        }
        AgentAction::Think { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. } => None,
    }
}

fn write_text_content_from_tool_like_action(tool: &str, args: &Value) -> Option<(String, String)> {
    let normalized = tool.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    if !matches!(
        normalized.as_str(),
        "fs_basic" | "system_basic" | "write_file"
    ) {
        return None;
    }
    if args
        .get("action")
        .and_then(Value::as_str)
        .is_some_and(|action| action != "write_text")
    {
        return None;
    }
    write_path_and_content(args)
}

fn write_text_content_from_capability_action(
    capability: &str,
    args: &Value,
) -> Option<(String, String)> {
    let normalized = capability
        .trim()
        .to_ascii_lowercase()
        .replace(['-', '_'], ".");
    if !matches!(
        normalized.as_str(),
        "filesystem.write.text" | "filesystem.write_text" | "filesystem.write"
    ) {
        return None;
    }
    write_path_and_content(args)
}

fn write_path_and_content(args: &Value) -> Option<(String, String)> {
    let path = ["resolved_path", "effective_path", "path"]
        .into_iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())?
        .to_string();
    let content = ["content", "text", "body"]
        .into_iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|content| !content.is_empty())?
        .to_string();
    Some((path, content))
}

fn replace_or_push_readback(readbacks: &mut Vec<super::FsReadback>, path: String, content: String) {
    if let Some(existing) = readbacks
        .iter_mut()
        .find(|existing| super::projection_paths_match(&existing.path, &path))
    {
        existing.path = path;
        existing.excerpt = content;
        return;
    }
    readbacks.push(super::FsReadback {
        path,
        excerpt: content,
    });
}
