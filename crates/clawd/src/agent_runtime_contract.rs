use crate::policy_decision::PolicyDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SubagentRole {
    Observe,
    Explorer,
    Worker,
    Writer,
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
            Self::Writer,
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
            Self::Writer => "writer",
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
            "writer" => Some(Self::Writer),
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
            Self::Worker | Self::Writer => "worker",
            Self::Review | Self::Reviewer => "reviewer",
            Self::Test | Self::Verifier => "verifier",
        }
    }

    pub(crate) fn default_scope_token(self) -> &'static str {
        match self {
            Self::Observe | Self::Explorer => "read_only_discovery",
            Self::Worker => "read_only_worker",
            Self::Writer => "isolated_worktree_write",
            Self::Review | Self::Reviewer => "read_only_review",
            Self::Test | Self::Verifier => "read_only_verification",
        }
    }

    pub(crate) fn result_contract_required(self) -> bool {
        matches!(
            self,
            Self::Writer | Self::Review | Self::Reviewer | Self::Test | Self::Verifier
        )
    }
}

pub(crate) fn runtime_protocol_hint_line() -> String {
    let roles = SubagentRole::all_tokens().join("|");
    let hooks = crate::agent_hooks::HookStage::all()
        .iter()
        .map(|stage| stage.as_token())
        .collect::<Vec<_>>()
        .join("|");
    let policy_decisions = PolicyDecision::all_tokens().join("|");
    format!(
        "agent_runtime_protocols=subagent_roles:{roles};subagent_inline_write_enabled:false;subagent_persistent_worktree_write_enabled:true;subagent_external_publish_enabled:false;hook_stages:{hooks};hook_decisions:{policy_decisions}"
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
        assert!(hint.contains("subagent_inline_write_enabled:false"));
        assert!(hint.contains("subagent_persistent_worktree_write_enabled:true"));
        assert!(hint.contains("subagent_external_publish_enabled:false"));
        let expected_hook_stages = format!(
            "hook_stages:{}",
            crate::agent_hooks::HookStage::all()
                .iter()
                .map(|stage| stage.as_token())
                .collect::<Vec<_>>()
                .join("|")
        );
        assert!(hint.contains(&expected_hook_stages));
        let expected_hook_decisions =
            format!("hook_decisions:{}", PolicyDecision::all_tokens().join("|"));
        assert!(hint.contains(&expected_hook_decisions));
    }
}
