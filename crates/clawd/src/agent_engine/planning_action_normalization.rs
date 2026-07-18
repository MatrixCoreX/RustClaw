use serde_json::Value;
use std::collections::HashSet;
use tracing::info;

use crate::{AgentAction, AppState};

pub(super) fn normalize_planned_actions(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let actions = collapse_redundant_config_preview_reads(actions);
    let actions = crate::capability_resolver::resolve_agent_actions_for_state(state, actions);
    let actions = normalize_action_arg_aliases(state, actions);
    let actions = annotate_readonly_cli_surface_run_cmds(state, actions);
    let actions = collapse_redundant_drafting_synthesis(actions);
    super::media_artifact_plan::strip_media_artifact_text_overwrites(
        &state.skill_rt.workspace_root,
        actions,
    )
}

fn collapse_redundant_config_preview_reads(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    let preview_targets = actions
        .iter()
        .filter_map(|action| config_capability_target(action, config_preview_capability))
        .collect::<HashSet<_>>();
    if preview_targets.len() != 1 {
        return actions;
    }
    let capability_call_count = actions
        .iter()
        .filter(|action| matches!(action, AgentAction::CallCapability { .. }))
        .count();
    let has_direct_call = actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallTool { .. } | AgentAction::CallSkill { .. }
        )
    });
    let redundant_read_count = actions
        .iter()
        .filter_map(|action| config_capability_target(action, config_read_capability))
        .filter(|target| preview_targets.contains(target))
        .count();
    if capability_call_count != 2 || redundant_read_count != 1 || has_direct_call {
        return actions;
    }
    let actions = actions
        .into_iter()
        .filter(|action| {
            config_capability_target(action, config_read_capability)
                .is_none_or(|target| !preview_targets.contains(&target))
        })
        .map(|action| match action {
            AgentAction::SynthesizeAnswer { .. } => AgentAction::SynthesizeAnswer {
                evidence_refs: vec!["last_output".to_string()],
            },
            other => other,
        })
        .collect::<Vec<_>>();
    info!("plan_normalize_redundant_config_preview_read");
    actions
}

fn config_capability_target(
    action: &AgentAction,
    capability_matches: fn(&str) -> bool,
) -> Option<(String, String)> {
    let AgentAction::CallCapability { capability, args } = action else {
        return None;
    };
    if !capability_matches(capability.trim()) {
        return None;
    }
    let path = args.get("path").and_then(Value::as_str)?.trim();
    let field_path = args.get("field_path").and_then(Value::as_str)?.trim();
    if path.is_empty() || field_path.is_empty() {
        return None;
    }
    Some((path.to_string(), field_path.to_string()))
}

fn config_preview_capability(capability: &str) -> bool {
    matches!(
        capability,
        "config.preview_change" | "config.plan_change" | "config.plan_config_change"
    )
}

fn config_read_capability(capability: &str) -> bool {
    matches!(
        capability,
        "config.read_field" | "config_basic.read_field" | "config_basic.extract_field"
    )
}

fn collapse_redundant_drafting_synthesis(actions: Vec<AgentAction>) -> Vec<AgentAction> {
    let [AgentAction::SynthesizeAnswer { .. }, AgentAction::Respond { content }] =
        actions.as_slice()
    else {
        return actions;
    };
    if content.trim().is_empty() || content.contains("{{") || content.contains("}}") {
        return actions;
    }
    info!("plan_normalize_redundant_drafting_synthesis");
    vec![AgentAction::Respond {
        content: content.clone(),
    }]
}

fn normalize_action_arg_aliases(state: &AppState, actions: Vec<AgentAction>) -> Vec<AgentAction> {
    actions
        .into_iter()
        .map(|mut action| {
            match &mut action {
                AgentAction::CallTool { tool, args } => {
                    let normalized = state.resolve_canonical_skill_name(tool);
                    super::arg_resolver::normalize_skill_arg_aliases(&normalized, args);
                }
                AgentAction::CallSkill { skill, args } => {
                    let normalized = state.resolve_canonical_skill_name(skill);
                    super::arg_resolver::normalize_skill_arg_aliases(&normalized, args);
                }
                AgentAction::CallCapability { .. }
                | AgentAction::SynthesizeAnswer { .. }
                | AgentAction::Respond { .. }
                | AgentAction::Think { .. } => {}
            }
            action
        })
        .collect()
}

fn annotate_readonly_cli_surface_run_cmds(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut changed = false;
    let actions = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if state.resolve_canonical_skill_name(&skill) == "run_cmd"
                    && annotate_readonly_cli_surface_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if state.resolve_canonical_skill_name(&tool) == "run_cmd"
                    && annotate_readonly_cli_surface_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_annotate_run_cmd_readonly_cli_surface");
    }
    actions
}

fn annotate_readonly_cli_surface_args(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    if obj.get("action").and_then(Value::as_str).is_some() {
        return false;
    }
    let Some(command) = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
    else {
        return false;
    };
    if !command_looks_like_readonly_cli_surface_probe(command) {
        return false;
    }
    obj.insert(
        "action".to_string(),
        Value::String("inspect_cli_help".to_string()),
    );
    obj.entry("timeout_seconds".to_string())
        .or_insert_with(|| Value::Number(10.into()));
    obj.entry("idle_timeout_seconds".to_string())
        .or_insert_with(|| Value::Number(5.into()));
    obj.entry("max_output_bytes".to_string())
        .or_insert_with(|| Value::Number(24000.into()));
    true
}

fn command_looks_like_readonly_cli_surface_probe(command: &str) -> bool {
    let lower = command.trim().to_ascii_lowercase();
    if lower.is_empty() || command_contains_forbidden_cli_probe_token(&lower) {
        return false;
    }
    let tokens = shell_word_tokens(&lower);
    lower.contains("--help")
        || lower.contains(" -h")
        || lower.contains("--version")
        || lower.contains(" -v")
        || tokens
            .first()
            .is_some_and(|token| matches!(*token, "which" | "type"))
        || tokens
            .windows(2)
            .any(|pair| matches!(pair, ["command", "-v"] | ["command", "v"]))
}

fn command_contains_forbidden_cli_probe_token(command_lower: &str) -> bool {
    const FORBIDDEN: &[&str] = &[
        "rm",
        "mv",
        "cp",
        "mkdir",
        "touch",
        "truncate",
        "install",
        "chmod",
        "chown",
        "ln",
        "sudo",
        "tee",
        "sed",
        "perl",
        "python",
        "python3",
        "node",
        "npm",
        "pnpm",
        "yarn",
        "bash",
        "sh",
        "zsh",
        "fish",
        "systemctl",
        "service",
        "kill",
        "pkill",
        "curl",
        "wget",
        "nc",
        "ssh",
        "scp",
        "rsync",
    ];
    shell_word_tokens(command_lower)
        .iter()
        .any(|token| FORBIDDEN.iter().any(|forbidden| token == forbidden))
}

fn shell_word_tokens(command_lower: &str) -> Vec<&str> {
    command_lower
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .filter(|token| !token.is_empty())
        .collect()
}

#[cfg(test)]
#[path = "planning_action_normalization_tests.rs"]
mod tests;
