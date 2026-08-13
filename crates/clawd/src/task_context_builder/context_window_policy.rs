use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::providers::client::LlmUsageSnapshot;
use crate::{AppState, ClaimedTask};

const CALIBRATION_SCALE: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ContextTokenScope {
    Total,
    #[cfg(test)]
    BodyAfterCarriedPrefix,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ContextWindowPolicy {
    pub(crate) schema_version: u32,
    pub(crate) provider_name: String,
    pub(crate) model: String,
    pub(crate) context_window_tokens: usize,
    pub(crate) output_reserve_tokens: usize,
    pub(crate) tool_observation_reserve_tokens: usize,
    pub(crate) estimator_safety_margin_tokens: usize,
    pub(crate) estimator_multiplier_millis: usize,
    pub(crate) token_scope: ContextTokenScope,
    pub(crate) policy_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ContextWindowDecision {
    pub(crate) trigger: bool,
    pub(crate) trigger_basis: &'static str,
    pub(crate) token_scope: ContextTokenScope,
    pub(crate) raw_token_estimate: usize,
    pub(crate) adjusted_token_estimate: usize,
    pub(crate) prefix_token_estimate: usize,
    pub(crate) body_token_estimate: usize,
    pub(crate) input_budget_tokens: usize,
    pub(crate) scoped_input_budget_tokens: usize,
    pub(crate) reserved_tokens: usize,
}

impl ContextWindowPolicy {
    pub(crate) fn for_task(state: &AppState, task: &ClaimedTask) -> Option<Self> {
        let provider = state
            .task_llm_providers(task)
            .into_iter()
            .filter_map(|provider| {
                provider
                    .model_descriptor()
                    .context_window_tokens
                    .map(|window| (provider, window))
            })
            .min_by_key(|(_, window)| *window)?;
        let descriptor = provider.0.model_descriptor();
        let multiplier = calibration_multiplier_millis(
            &provider.0.config.name,
            &provider.0.config.model,
            "mixed",
        );
        Some(Self::new(
            provider.0.config.name.clone(),
            provider.0.config.model.clone(),
            provider.1,
            descriptor.output_reserve_tokens,
            state.policy.limits.context_tool_observation_reserve_tokens,
            state.policy.limits.context_estimator_safety_margin_tokens,
            multiplier,
            ContextTokenScope::Total,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        provider_name: String,
        model: String,
        context_window_tokens: usize,
        output_reserve_tokens: usize,
        tool_observation_reserve_tokens: usize,
        estimator_safety_margin_tokens: usize,
        estimator_multiplier_millis: usize,
        token_scope: ContextTokenScope,
    ) -> Self {
        let digest_input = format!(
            "context-window-policy-v1\0{provider_name}\0{model}\0{context_window_tokens}\0{output_reserve_tokens}\0{tool_observation_reserve_tokens}\0{estimator_safety_margin_tokens}\0{estimator_multiplier_millis}\0{token_scope:?}"
        );
        Self {
            schema_version: 1,
            provider_name,
            model,
            context_window_tokens,
            output_reserve_tokens,
            tool_observation_reserve_tokens,
            estimator_safety_margin_tokens,
            estimator_multiplier_millis: estimator_multiplier_millis.max(CALIBRATION_SCALE),
            token_scope,
            policy_digest: format!("sha256:{:x}", Sha256::digest(digest_input.as_bytes())),
        }
    }

    pub(crate) fn evaluate(
        &self,
        total_token_estimate: usize,
        prefix_token_estimate: usize,
    ) -> ContextWindowDecision {
        let prefix_token_estimate = prefix_token_estimate.min(total_token_estimate);
        let body_token_estimate = total_token_estimate.saturating_sub(prefix_token_estimate);
        let reserved_tokens = self
            .output_reserve_tokens
            .saturating_add(self.tool_observation_reserve_tokens)
            .saturating_add(self.estimator_safety_margin_tokens);
        let input_budget_tokens = self.context_window_tokens.saturating_sub(reserved_tokens);
        let scoped_input_budget_tokens = match self.token_scope {
            ContextTokenScope::Total => input_budget_tokens,
            #[cfg(test)]
            ContextTokenScope::BodyAfterCarriedPrefix => {
                input_budget_tokens.saturating_sub(prefix_token_estimate)
            }
        };
        let raw_token_estimate = match self.token_scope {
            ContextTokenScope::Total => total_token_estimate,
            #[cfg(test)]
            ContextTokenScope::BodyAfterCarriedPrefix => body_token_estimate,
        };
        let adjusted_token_estimate = raw_token_estimate
            .saturating_mul(self.estimator_multiplier_millis)
            .saturating_add(CALIBRATION_SCALE - 1)
            .saturating_div(CALIBRATION_SCALE);
        ContextWindowDecision {
            trigger: adjusted_token_estimate >= scoped_input_budget_tokens,
            trigger_basis: match self.token_scope {
                ContextTokenScope::Total => "total_context_after_reserves",
                #[cfg(test)]
                ContextTokenScope::BodyAfterCarriedPrefix => {
                    "body_after_carried_prefix_after_reserves"
                }
            },
            token_scope: self.token_scope,
            raw_token_estimate,
            adjusted_token_estimate,
            prefix_token_estimate,
            body_token_estimate,
            input_budget_tokens,
            scoped_input_budget_tokens,
            reserved_tokens,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct CalibrationSample {
    multiplier_millis: usize,
    sample_count: u64,
}

fn calibrations() -> &'static Mutex<HashMap<String, CalibrationSample>> {
    static CALIBRATIONS: OnceLock<Mutex<HashMap<String, CalibrationSample>>> = OnceLock::new();
    CALIBRATIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn note_context_token_usage(
    provider_name: &str,
    provider_type: &str,
    model: &str,
    prompt: &str,
    usage: Option<&LlmUsageSnapshot>,
) {
    let Some(actual_tokens) = usage
        .and_then(|usage| usage.input_tokens.or(usage.prompt_tokens))
        .and_then(|tokens| usize::try_from(tokens).ok())
        .filter(|tokens| *tokens > 0)
    else {
        return;
    };
    let estimate = crate::token_estimator::estimate_provider_tokens(
        provider_name,
        provider_type,
        model,
        prompt,
    )
    .provider_tokens
    .max(1);
    let observed = actual_tokens
        .saturating_mul(CALIBRATION_SCALE)
        .saturating_add(estimate - 1)
        .saturating_div(estimate)
        .clamp(CALIBRATION_SCALE / 2, CALIBRATION_SCALE * 4);
    let language = prompt_language_class(prompt);
    let key = calibration_key(provider_name, model, language);
    if let Ok(mut values) = calibrations().lock() {
        let sample = values.entry(key).or_insert(CalibrationSample {
            multiplier_millis: observed,
            sample_count: 0,
        });
        sample.multiplier_millis = sample
            .multiplier_millis
            .saturating_mul(7)
            .saturating_add(observed.saturating_mul(3))
            .saturating_div(10);
        sample.sample_count = sample.sample_count.saturating_add(1);
    }
}

fn calibration_multiplier_millis(provider_name: &str, model: &str, language: &str) -> usize {
    let Ok(values) = calibrations().lock() else {
        return CALIBRATION_SCALE;
    };
    let exact = values
        .get(&calibration_key(provider_name, model, language))
        .map(|sample| sample.multiplier_millis);
    let mixed = values
        .iter()
        .filter(|(key, _)| key.starts_with(&format!("{provider_name}\0{model}\0")))
        .map(|(_, sample)| sample.multiplier_millis)
        .max();
    exact
        .or(mixed)
        .unwrap_or(CALIBRATION_SCALE)
        .max(CALIBRATION_SCALE)
}

fn calibration_key(provider_name: &str, model: &str, language: &str) -> String {
    format!("{provider_name}\0{model}\0{language}")
}

fn prompt_language_class(prompt: &str) -> &'static str {
    if prompt.chars().any(
        |character| matches!(character as u32, 0x3040..=0x30ff | 0x3400..=0x9fff | 0xac00..=0xd7af),
    ) {
        "cjk"
    } else {
        "other"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn total_and_body_after_prefix_have_equivalent_pressure_semantics() {
        let total = ContextWindowPolicy::new(
            "fixture".to_string(),
            "model".to_string(),
            10_000,
            1_000,
            1_000,
            500,
            1_000,
            ContextTokenScope::Total,
        );
        let body = ContextWindowPolicy::new(
            "fixture".to_string(),
            "model".to_string(),
            10_000,
            1_000,
            1_000,
            500,
            1_000,
            ContextTokenScope::BodyAfterCarriedPrefix,
        );
        assert_eq!(
            total.evaluate(7_500, 2_000).trigger,
            body.evaluate(7_500, 2_000).trigger
        );
        assert_eq!(
            total.evaluate(7_499, 2_000).trigger,
            body.evaluate(7_499, 2_000).trigger
        );
    }

    #[test]
    fn calibration_only_increases_runtime_safety_multiplier() {
        let usage = LlmUsageSnapshot {
            prompt_tokens: Some(2_000),
            completion_tokens: None,
            total_tokens: None,
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            cached_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        note_context_token_usage(
            "fixture-calibration",
            "fixture",
            "model",
            "small",
            Some(&usage),
        );
        assert!(calibration_multiplier_millis("fixture-calibration", "model", "mixed") >= 1_000);
    }
}
