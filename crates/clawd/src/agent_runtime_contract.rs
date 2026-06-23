use crate::policy_decision::PolicyDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubagentRole {
    Observe,
    Explorer,
    Worker,
    Review,
    Reviewer,
    Test,
    Verifier,
}

impl SubagentRole {
    pub(crate) fn all() -> &'static [Self] {
        &[
            Self::Observe,
            Self::Explorer,
            Self::Worker,
            Self::Review,
            Self::Reviewer,
            Self::Test,
            Self::Verifier,
        ]
    }

    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Explorer => "explorer",
            Self::Worker => "worker",
            Self::Review => "review",
            Self::Reviewer => "reviewer",
            Self::Test => "test",
            Self::Verifier => "verifier",
        }
    }

    pub(crate) fn parse_token(value: &str) -> Option<Self> {
        match value.trim() {
            "observe" => Some(Self::Observe),
            "explorer" => Some(Self::Explorer),
            "worker" => Some(Self::Worker),
            "review" => Some(Self::Review),
            "reviewer" => Some(Self::Reviewer),
            "test" => Some(Self::Test),
            "verifier" => Some(Self::Verifier),
            _ => None,
        }
    }

    pub(crate) fn all_tokens() -> Vec<&'static str> {
        Self::all().iter().map(|role| role.as_token()).collect()
    }

    pub(crate) fn family_token(self) -> &'static str {
        match self {
            Self::Observe | Self::Explorer => "explorer",
            Self::Worker => "worker",
            Self::Review | Self::Reviewer => "reviewer",
            Self::Test | Self::Verifier => "verifier",
        }
    }

    pub(crate) fn default_scope_token(self) -> &'static str {
        match self {
            Self::Observe | Self::Explorer => "read_only_discovery",
            Self::Worker => "read_only_worker",
            Self::Review | Self::Reviewer => "read_only_review",
            Self::Test | Self::Verifier => "read_only_verification",
        }
    }

    pub(crate) fn result_contract_required(self) -> bool {
        matches!(
            self,
            Self::Review | Self::Reviewer | Self::Test | Self::Verifier
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookStage {
    PreToolUse,
    PostToolUse,
    Stop,
    SessionStart,
    SessionEnd,
    UserPromptSubmit,
}

impl HookStage {
    fn all() -> &'static [Self] {
        &[
            Self::PreToolUse,
            Self::PostToolUse,
            Self::Stop,
            Self::SessionStart,
            Self::SessionEnd,
            Self::UserPromptSubmit,
        ]
    }

    fn as_token(self) -> &'static str {
        match self {
            Self::PreToolUse => "pre_tool_use",
            Self::PostToolUse => "post_tool_use",
            Self::Stop => "stop",
            Self::SessionStart => "session_start",
            Self::SessionEnd => "session_end",
            Self::UserPromptSubmit => "user_prompt_submit",
        }
    }
}

pub(crate) fn runtime_protocol_hint_line() -> String {
    let roles = SubagentRole::all_tokens().join("|");
    let hooks = HookStage::all()
        .iter()
        .map(|stage| stage.as_token())
        .collect::<Vec<_>>()
        .join("|");
    let policy_decisions = PolicyDecision::all_tokens().join("|");
    format!(
        "agent_runtime_protocols=subagent_roles:{roles};subagent_write_enabled:false;subagent_external_publish_enabled:false;hook_stages:{hooks};hook_decisions:{policy_decisions}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_protocol_hint_exposes_safe_subagent_roles_and_hook_stages() {
        let hint = runtime_protocol_hint_line();
        let expected_roles = format!("subagent_roles:{}", SubagentRole::all_tokens().join("|"));
        assert!(hint.contains(&expected_roles));
        assert!(hint.contains("subagent_write_enabled:false"));
        assert!(hint.contains("subagent_external_publish_enabled:false"));
        assert!(hint.contains(
            "hook_stages:pre_tool_use|post_tool_use|stop|session_start|session_end|user_prompt_submit"
        ));
        let expected_hook_decisions =
            format!("hook_decisions:{}", PolicyDecision::all_tokens().join("|"));
        assert!(hint.contains(&expected_hook_decisions));
    }
}
