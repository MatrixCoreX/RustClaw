#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PolicyDecision {
    Allow,
    RequireConfirmation,
    Deny,
    BackgroundWait,
}

impl PolicyDecision {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::RequireConfirmation => "require_confirmation",
            Self::Deny => "deny",
            Self::BackgroundWait => "background_wait",
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
}
