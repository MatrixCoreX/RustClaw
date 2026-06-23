use crate::policy_decision::PolicyDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubagentRole {
    Observe,
    Review,
    Test,
}

impl SubagentRole {
    fn all() -> &'static [Self] {
        &[Self::Observe, Self::Review, Self::Test]
    }

    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Review => "review",
            Self::Test => "test",
        }
    }

    pub(crate) fn parse_token(value: &str) -> Option<Self> {
        match value.trim() {
            "observe" => Some(Self::Observe),
            "review" => Some(Self::Review),
            "test" => Some(Self::Test),
            _ => None,
        }
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
    let roles = SubagentRole::all()
        .iter()
        .map(|role| role.as_token())
        .collect::<Vec<_>>()
        .join("|");
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
        assert!(hint.contains("subagent_roles:observe|review|test"));
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
