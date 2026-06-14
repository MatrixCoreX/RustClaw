use serde_json::Value;

use crate::AgentAction;

pub(in crate::agent_engine) fn planned_action_is_single_path_metadata_facts(
    action: &AgentAction,
) -> bool {
    planned_single_path_metadata_facts_path(action).is_some()
}

pub(in crate::agent_engine) fn planned_single_path_metadata_facts_path(
    action: &AgentAction,
) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            (skill.as_str(), args)
        }
        AgentAction::CallCapability { .. }
        | AgentAction::Think { .. }
        | AgentAction::Respond { .. }
        | AgentAction::SynthesizeAnswer { .. } => return None,
    };
    let action = args.get("action").and_then(Value::as_str)?.trim();
    let is_stat = (skill.eq_ignore_ascii_case("fs_basic")
        && action.eq_ignore_ascii_case("stat_paths"))
        || (skill.eq_ignore_ascii_case("system_basic")
            && action.eq_ignore_ascii_case("path_batch_facts"));
    if !is_stat {
        return None;
    }
    let obj = args.as_object()?;
    let paths = strings_from_value(obj.get("paths"))
        .into_iter()
        .chain(strings_from_value(obj.get("targets")))
        .chain(strings_from_value(obj.get("path")))
        .collect::<Vec<_>>();
    (paths.len() == 1).then(|| paths[0].clone())
}

fn strings_from_value(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::String(text)) if !text.trim().is_empty() => vec![text.trim().to_string()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToString::to_string)
            .collect(),
        _ => Vec::new(),
    }
}
