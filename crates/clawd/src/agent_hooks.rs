use serde_json::{json, Value};

use crate::policy_decision::PolicyDecision;

mod command;
mod http;
mod mcp;
mod runtime;
mod shared;
#[cfg(test)]
use command::{execute_command_handler, validate_command_handler};
pub(crate) use runtime::{
    hook_admin_status_for_state, lifecycle_stage_outcome_for_state, pre_tool_use_outcome_for_state,
};
#[cfg(test)]
use shared::{lifecycle_hook_event, parse_handler_output, pre_tool_hook_event, HookHandlerConfig};

const HOOK_EVENT_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookStage {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PermissionRequest,
    PostToolUse,
    PreCompact,
    PostCompact,
    SubagentStart,
    SubagentStop,
    Stop,
    SessionEnd,
}

impl HookStage {
    pub(crate) fn all() -> &'static [Self] {
        &[
            Self::SessionStart,
            Self::UserPromptSubmit,
            Self::PreToolUse,
            Self::PermissionRequest,
            Self::PostToolUse,
            Self::PreCompact,
            Self::PostCompact,
            Self::SubagentStart,
            Self::SubagentStop,
            Self::Stop,
            Self::SessionEnd,
        ]
    }

    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::SessionStart => "session_start",
            Self::UserPromptSubmit => "user_prompt_submit",
            Self::PreToolUse => "pre_tool_use",
            Self::PermissionRequest => "permission_request",
            Self::PostToolUse => "post_tool_use",
            Self::PreCompact => "pre_compact",
            Self::PostCompact => "post_compact",
            Self::SubagentStart => "subagent_start",
            Self::SubagentStop => "subagent_stop",
            Self::Stop => "stop",
            Self::SessionEnd => "session_end",
        }
    }

    fn parse_token(value: &str) -> Option<Self> {
        match value.trim() {
            "session_start" => Some(Self::SessionStart),
            "user_prompt_submit" => Some(Self::UserPromptSubmit),
            "pre_tool_use" => Some(Self::PreToolUse),
            "permission_request" => Some(Self::PermissionRequest),
            "post_tool_use" => Some(Self::PostToolUse),
            "pre_compact" => Some(Self::PreCompact),
            "post_compact" => Some(Self::PostCompact),
            "subagent_start" => Some(Self::SubagentStart),
            "subagent_stop" => Some(Self::SubagentStop),
            "stop" => Some(Self::Stop),
            "session_end" => Some(Self::SessionEnd),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct HookEvaluation {
    pub(crate) outcome: HookOutcome,
    pub(crate) handler_observations: Vec<Value>,
}

impl HookEvaluation {
    pub(crate) fn requires_confirmation(&self) -> bool {
        self.outcome.requires_confirmation()
    }

    pub(crate) fn requires_background_wait(&self) -> bool {
        self.outcome.requires_background_wait()
    }

    pub(crate) fn machine_observations(&self, target: &str) -> Vec<Value> {
        let mut observations = self.handler_observations.clone();
        observations.push(self.outcome.to_machine_json(target));
        observations
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HookOutcome {
    pub(crate) stage: &'static str,
    pub(crate) decision: &'static str,
    pub(crate) reason_code: String,
    pub(crate) action_ref: String,
}

impl HookOutcome {
    pub(crate) fn decision_kind(&self) -> Option<PolicyDecision> {
        PolicyDecision::parse_token(self.decision)
    }

    pub(crate) fn requires_confirmation(&self) -> bool {
        self.decision_kind()
            .is_some_and(PolicyDecision::requires_confirmation)
    }

    pub(crate) fn requires_background_wait(&self) -> bool {
        self.decision_kind()
            .is_some_and(PolicyDecision::requires_background_wait)
    }

    pub(crate) fn to_machine_json(&self, tool_or_skill: &str) -> Value {
        json!({
            "schema_version": 1,
            "event_schema_version": HOOK_EVENT_SCHEMA_VERSION,
            "event_type": self.stage,
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
        reason_code: reason_code.to_string(),
        action_ref,
    }
}

#[cfg(test)]
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
        reason_code: reason_code.to_string(),
        action_ref: "agent_loop.stop".to_string(),
    }
}

#[cfg(test)]
pub(crate) fn session_start_outcome() -> HookOutcome {
    HookOutcome {
        stage: "session_start",
        decision: PolicyDecision::Allow.as_token(),
        reason_code: "session_start".to_string(),
        action_ref: "agent_loop.session_start".to_string(),
    }
}

#[cfg(test)]
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
        reason_code: reason_code.to_string(),
        action_ref: "agent_loop.session_end".to_string(),
    }
}

#[cfg(test)]
pub(crate) fn user_prompt_submit_outcome() -> HookOutcome {
    HookOutcome {
        stage: "user_prompt_submit",
        decision: PolicyDecision::Allow.as_token(),
        reason_code: "user_prompt_submitted".to_string(),
        action_ref: "agent_loop.user_prompt_submit".to_string(),
    }
}

pub(crate) fn structured_error_for_outcome(outcome: &HookOutcome) -> Option<String> {
    outcome
        .decision_kind()
        .is_some_and(PolicyDecision::blocks_execution)
        .then(|| structured_hook_error(outcome))
}

fn default_pre_tool_use_outcome(action_ref: &str) -> HookOutcome {
    let action_ref = normalize_machine_token(action_ref);
    HookOutcome {
        stage: "pre_tool_use",
        decision: PolicyDecision::Allow.as_token(),
        reason_code: "pre_tool_use_allowed".to_string(),
        action_ref,
    }
}

fn structured_hook_error(outcome: &HookOutcome) -> String {
    let message_key = match (outcome.stage, outcome.decision_kind()) {
        ("permission_request", Some(PolicyDecision::BackgroundWait)) => {
            "clawd.agent_hook.permission_request_background_wait"
        }
        ("permission_request", Some(PolicyDecision::RequireConfirmation)) => {
            "clawd.agent_hook.permission_request_requires_confirmation"
        }
        ("permission_request", _) => "clawd.agent_hook.permission_request_blocked",
        (_, Some(PolicyDecision::BackgroundWait)) => {
            "clawd.agent_hook.pre_tool_use_background_wait"
        }
        (_, Some(PolicyDecision::RequireConfirmation)) => {
            "clawd.agent_hook.pre_tool_use_requires_confirmation"
        }
        _ => "clawd.agent_hook.pre_tool_use_blocked",
    };
    json!({
        "schema_version": 1,
        "owner_layer": "agent_hooks",
        "stage": outcome.stage,
        "decision": outcome.decision,
        "reason_code": outcome.reason_code,
        "status_code": outcome.reason_code,
        "error_kind": outcome.reason_code,
        "message_key": message_key,
        "action_ref": outcome.action_ref,
    })
    .to_string()
}

fn merge_hook_decision(outcome: &mut HookOutcome, decision: PolicyDecision, reason_code: String) {
    if decision_priority(decision) > outcome.decision_kind().map(decision_priority).unwrap_or(0) {
        outcome.decision = decision.as_token();
        outcome.reason_code = reason_code;
    }
}

fn decision_priority(decision: PolicyDecision) -> u8 {
    match decision {
        PolicyDecision::Allow => 1,
        PolicyDecision::BackgroundWait => 2,
        PolicyDecision::RequireConfirmation => 3,
        PolicyDecision::Deny => 4,
    }
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

fn normalize_machine_token(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace('-', "_")
}

#[cfg(test)]
#[path = "agent_hooks_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_hooks_transport_tests.rs"]
mod transport_tests;
