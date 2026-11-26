//! Per-call generation limits are separate from the task's frozen total budget.
use super::{parse_u64, value_at, TomlValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelOutputBudgetPolicy {
    pub(crate) default_tokens: u64,
    pub(crate) verifier_tokens: u64,
    pub(crate) recovery_max_tokens: u64,
    pub(crate) recovery_attempts: u64,
}

impl Default for ModelOutputBudgetPolicy {
    fn default() -> Self {
        Self {
            default_tokens: 8192,
            verifier_tokens: 4096,
            recovery_max_tokens: 32768,
            recovery_attempts: 2,
        }
    }
}

impl ModelOutputBudgetPolicy {
    pub(crate) fn stage_tokens(self, label: &str) -> u64 {
        if label == "verifier" {
            self.verifier_tokens
        } else {
            self.default_tokens
        }
    }

    pub(super) fn from_toml(root: &TomlValue) -> Self {
        let mut policy = Self::default();
        for (key, target) in [
            ("default_tokens", &mut policy.default_tokens),
            ("verifier_tokens", &mut policy.verifier_tokens),
            ("recovery_max_tokens", &mut policy.recovery_max_tokens),
        ] {
            *target = parse_u64(
                root,
                &["agent", "task_budget", "model_output", key],
                *target,
            );
        }
        policy.recovery_attempts = value_at(
            root,
            &["agent", "task_budget", "model_output", "recovery_attempts"],
        )
        .and_then(TomlValue::as_integer)
        .and_then(|value| u64::try_from(value).ok())
        .unwrap_or(policy.recovery_attempts);
        policy
    }
}

#[cfg(test)]
#[path = "task_budget_model_output_tests.rs"]
mod tests;
