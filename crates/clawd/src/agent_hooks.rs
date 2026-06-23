use serde_json::{json, Value};

use crate::{policy_decision::PolicyDecision, AppState};

#[derive(Debug, Clone, Default)]
struct HookPolicy {
    blocked_action_refs: Vec<String>,
    blocked_tools: Vec<String>,
    require_confirmation_action_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookOutcome {
    pub(crate) stage: &'static str,
    pub(crate) decision: &'static str,
    pub(crate) reason_code: &'static str,
    pub(crate) action_ref: String,
}

impl HookOutcome {
    pub(crate) fn to_machine_json(&self, tool_or_skill: &str) -> Value {
        json!({
            "schema_version": 1,
            "owner_layer": "agent_hooks",
            "stage": self.stage,
            "decision": self.decision,
            "reason_code": self.reason_code,
            "status_code": self.reason_code,
            "action_ref": self.action_ref,
            "tool_or_skill": normalize_machine_token(tool_or_skill),
        })
    }
}

pub(crate) fn pre_tool_use_outcome_for_state(
    state: &AppState,
    tool_or_skill: &str,
    args: &Value,
) -> HookOutcome {
    let action_ref = tool_action_ref(tool_or_skill, args);
    let policy = load_hook_policy(state);
    evaluate_pre_tool_use(&policy, &action_ref)
}

pub(crate) fn post_tool_use_outcome(
    tool_or_skill: &str,
    args: &Value,
    step_status: &str,
) -> HookOutcome {
    let action_ref = tool_action_ref(tool_or_skill, args);
    let reason_code = if step_status == "ok" {
        "post_tool_use_ok"
    } else {
        "post_tool_use_error"
    };
    HookOutcome {
        stage: "post_tool_use",
        decision: PolicyDecision::Allow.as_token(),
        reason_code,
        action_ref,
    }
}

pub(crate) fn stop_outcome(final_status: &str) -> HookOutcome {
    let reason_code = match final_status {
        "success" => "stop_success",
        "failure" => "stop_failure",
        "clarify" => "stop_clarify",
        "resume_failure" => "stop_resume_failure",
        _ => "stop_unknown",
    };
    HookOutcome {
        stage: "stop",
        decision: PolicyDecision::Allow.as_token(),
        reason_code,
        action_ref: "agent_loop.stop".to_string(),
    }
}

pub(crate) fn session_start_outcome() -> HookOutcome {
    HookOutcome {
        stage: "session_start",
        decision: PolicyDecision::Allow.as_token(),
        reason_code: "session_start",
        action_ref: "agent_loop.session_start".to_string(),
    }
}

pub(crate) fn session_end_outcome(final_status: &str) -> HookOutcome {
    let reason_code = match final_status {
        "success" => "session_end_success",
        "failure" => "session_end_failure",
        "clarify" => "session_end_clarify",
        "resume_failure" => "session_end_resume_failure",
        _ => "session_end_unknown",
    };
    HookOutcome {
        stage: "session_end",
        decision: PolicyDecision::Allow.as_token(),
        reason_code,
        action_ref: "agent_loop.session_end".to_string(),
    }
}

pub(crate) fn user_prompt_submit_outcome() -> HookOutcome {
    HookOutcome {
        stage: "user_prompt_submit",
        decision: PolicyDecision::Allow.as_token(),
        reason_code: "user_prompt_submitted",
        action_ref: "agent_loop.user_prompt_submit".to_string(),
    }
}

pub(crate) fn structured_error_for_outcome(outcome: &HookOutcome) -> Option<String> {
    matches!(outcome.decision, "deny" | "require_confirmation")
        .then(|| structured_hook_error(outcome))
}

fn evaluate_pre_tool_use(policy: &HookPolicy, action_ref: &str) -> HookOutcome {
    let action_ref = normalize_machine_token(action_ref);
    let tool_ref = action_ref
        .split_once('.')
        .map(|(tool, _)| tool)
        .unwrap_or(&action_ref);
    let decision = if token_list_contains(&policy.blocked_action_refs, &action_ref)
        || token_list_contains(&policy.blocked_tools, tool_ref)
    {
        PolicyDecision::Deny
    } else if token_list_contains(&policy.require_confirmation_action_refs, &action_ref) {
        PolicyDecision::RequireConfirmation
    } else {
        PolicyDecision::Allow
    };
    let reason_code = match decision {
        PolicyDecision::Allow => "pre_tool_use_allowed",
        PolicyDecision::Deny => "pre_tool_use_blocked",
        PolicyDecision::RequireConfirmation => "pre_tool_use_requires_confirmation",
        PolicyDecision::BackgroundWait => "pre_tool_use_background_wait",
    };
    HookOutcome {
        stage: "pre_tool_use",
        decision: decision.as_token(),
        reason_code,
        action_ref,
    }
}

fn structured_hook_error(outcome: &HookOutcome) -> String {
    json!({
        "schema_version": 1,
        "owner_layer": "agent_hooks",
        "stage": outcome.stage,
        "decision": outcome.decision,
        "reason_code": outcome.reason_code,
        "status_code": outcome.reason_code,
        "error_kind": outcome.reason_code,
        "message_key": "clawd.agent_hook.pre_tool_use_blocked",
        "action_ref": outcome.action_ref,
    })
    .to_string()
}

fn load_hook_policy(state: &AppState) -> HookPolicy {
    let path = state
        .skill_rt
        .workspace_root
        .join("configs/agent_guard.toml");
    let parsed = std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<toml::Value>(&raw).ok());
    let Some(root) = parsed.as_ref() else {
        return HookPolicy::default();
    };
    HookPolicy {
        blocked_action_refs: toml_string_array(root, &["agent", "hooks", "blocked_action_refs"]),
        blocked_tools: toml_string_array(root, &["agent", "hooks", "blocked_tools"]),
        require_confirmation_action_refs: toml_string_array(
            root,
            &["agent", "hooks", "require_confirmation_action_refs"],
        ),
    }
}

fn toml_string_array(root: &toml::Value, path: &[&str]) -> Vec<String> {
    let mut cursor = root;
    for segment in path {
        let Some(next) = cursor.get(*segment) else {
            return Vec::new();
        };
        cursor = next;
    }
    cursor
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(toml::Value::as_str)
                .map(normalize_machine_token)
                .filter(|value| !value.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn tool_action_ref(tool_or_skill: &str, args: &Value) -> String {
    let base = normalize_machine_token(tool_or_skill);
    args.get("action")
        .and_then(Value::as_str)
        .map(normalize_machine_token)
        .filter(|action| !action.is_empty())
        .map(|action| format!("{base}.{action}"))
        .unwrap_or(base)
}

fn token_list_contains(values: &[String], target: &str) -> bool {
    values.iter().any(|value| value == target)
}

fn normalize_machine_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

#[cfg(test)]
#[path = "agent_hooks_tests.rs"]
mod tests;
