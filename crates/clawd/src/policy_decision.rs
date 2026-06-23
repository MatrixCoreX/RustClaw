#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyDecision {
    Allow,
    RequireConfirmation,
    Deny,
    BackgroundWait,
}

impl PolicyDecision {
    pub(crate) fn all_tokens() -> &'static [&'static str] {
        &["allow", "deny", "require_confirmation", "background_wait"]
    }

    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireConfirmation => "require_confirmation",
            Self::Deny => "deny",
            Self::BackgroundWait => "background_wait",
        }
    }

    pub(crate) fn parse_token(value: &str) -> Option<Self> {
        match value.trim() {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "require_confirmation" => Some(Self::RequireConfirmation),
            "background_wait" => Some(Self::BackgroundWait),
            _ => None,
        }
    }

    pub(crate) fn blocks_execution(self) -> bool {
        matches!(self, Self::Deny | Self::RequireConfirmation)
    }

    pub(crate) fn requires_confirmation(self) -> bool {
        self == Self::RequireConfirmation
    }

    pub(crate) fn pre_tool_use_reason_code(self) -> &'static str {
        match self {
            Self::Allow => "pre_tool_use_allowed",
            Self::Deny => "pre_tool_use_blocked",
            Self::RequireConfirmation => "pre_tool_use_requires_confirmation",
            Self::BackgroundWait => "pre_tool_use_background_wait",
        }
    }

    pub(crate) fn from_permission_flags(
        approved: bool,
        needs_confirmation: bool,
        denied_by_policy: bool,
        background_wait: bool,
    ) -> Self {
        if background_wait {
            Self::BackgroundWait
        } else if approved && needs_confirmation {
            Self::RequireConfirmation
        } else if denied_by_policy || !approved {
            Self::Deny
        } else {
            Self::Allow
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PolicyDecision;

    #[test]
    fn permission_flags_map_to_closed_tokens() {
        assert_eq!(
            PolicyDecision::from_permission_flags(true, false, false, false).as_token(),
            "allow"
        );
        assert_eq!(
            PolicyDecision::from_permission_flags(true, true, false, false).as_token(),
            "require_confirmation"
        );
        assert_eq!(
            PolicyDecision::from_permission_flags(false, false, true, false).as_token(),
            "deny"
        );
        assert_eq!(
            PolicyDecision::from_permission_flags(false, true, true, false).as_token(),
            "deny"
        );
        assert_eq!(
            PolicyDecision::from_permission_flags(true, false, false, true).as_token(),
            "background_wait"
        );
    }

    #[test]
    fn all_tokens_exposes_protocol_vocabulary() {
        assert_eq!(
            PolicyDecision::all_tokens(),
            &["allow", "deny", "require_confirmation", "background_wait"]
        );
    }

    #[test]
    fn token_parse_and_execution_flags_are_closed() {
        assert_eq!(
            PolicyDecision::parse_token("allow"),
            Some(PolicyDecision::Allow)
        );
        assert_eq!(
            PolicyDecision::parse_token("require_confirmation"),
            Some(PolicyDecision::RequireConfirmation)
        );
        assert_eq!(PolicyDecision::parse_token("unknown"), None);
        assert!(!PolicyDecision::Allow.blocks_execution());
        assert!(PolicyDecision::Deny.blocks_execution());
        assert!(PolicyDecision::RequireConfirmation.blocks_execution());
        assert!(PolicyDecision::RequireConfirmation.requires_confirmation());
        assert_eq!(
            PolicyDecision::BackgroundWait.pre_tool_use_reason_code(),
            "pre_tool_use_background_wait"
        );
    }
}
