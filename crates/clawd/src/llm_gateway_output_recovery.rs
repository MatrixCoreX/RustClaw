//! Recover provider-declared output truncation before any native action is exposed.
use claw_core::model_turn::{ModelFinishReason, ModelTurnRequest, ModelTurnResponse};
use serde_json::json;

use crate::task_budget_contract::model_output::ModelOutputBudgetPolicy;
use crate::{AppState, ChatRequestHints, ClaimedTask};

use crate::llm_gateway::NATIVE_OUTPUT_LIMIT_EXHAUSTED_ERR as EXHAUSTED;

struct GenerationBudget {
    tokens: u64,
    attempts: u64,
    policy: ModelOutputBudgetPolicy,
}

impl GenerationBudget {
    fn new(policy: ModelOutputBudgetPolicy, label: &str) -> Self {
        Self {
            tokens: policy.stage_tokens(label),
            attempts: 0,
            policy,
        }
    }

    fn hints(&self) -> ChatRequestHints {
        ChatRequestHints {
            max_tokens: Some(self.tokens),
            minimum_output_tokens: (self.attempts > 0).then_some(self.tokens),
            ..Default::default()
        }
    }

    fn grow(&mut self) -> bool {
        if self.attempts >= self.policy.recovery_attempts {
            return false;
        }
        let next = self
            .tokens
            .saturating_mul(2)
            .min(self.policy.recovery_max_tokens);
        if next <= self.tokens {
            return false;
        }
        self.tokens = next;
        self.attempts = self.attempts.saturating_add(1);
        true
    }
}

pub(crate) async fn run_native_model_turn_with_fallback(
    state: &AppState,
    task: &ClaimedTask,
    prompt: &str,
    prompt_source: &str,
    request: &ModelTurnRequest,
) -> Result<Option<ModelTurnResponse>, String> {
    let policy =
        crate::task_budget_contract::load_task_budget_policy(&state.skill_rt.workspace_root);
    let label = crate::llm_gateway::classify_prompt_source(prompt_source);
    let mut budget = GenerationBudget::new(policy.model_output, label);
    let mut source = prompt_source.to_string();
    loop {
        let result = super::run_native_model_turn_attempt(
            state,
            task,
            prompt,
            &source,
            request,
            budget.hints(),
        )
        .await?;
        let Some(turn) = result else { return Ok(None) };
        if turn.finish_reason != ModelFinishReason::Length {
            return Ok(Some(turn));
        }
        // Even syntactically complete calls in a truncated turn may be only a
        // prefix of the intended plan. None of them can reach the executor.
        crate::assistant_presentation::abort_active_answer(
            state,
            task,
            "native_model_output_truncated",
            "assistant.output.contract_retry",
            true,
        );
        let previous = budget.tokens;
        let retry = budget.grow();
        let _ = crate::task_event_transport::publish_claimed_event(
            state,
            task,
            "model_output_recovery",
            json!({
                "schema_version":1,"owner_layer":"task_budget",
                "reason_code":if retry { "native_model_output_budget_increased" } else { EXHAUSTED },
                "finish_reason":turn.finish_reason,"prompt_label":label,
                "previous_output_tokens":previous,"next_output_tokens":if retry { Some(budget.tokens) } else { None },
                "recovery_attempt":budget.attempts,"recovery_max_tokens":budget.policy.recovery_max_tokens,
                "max_recovery_attempts":budget.policy.recovery_attempts,"partial_actions_discarded":true,
                "reported_completion_tokens":turn.usage.as_ref().and_then(|usage|usage.completion_tokens.or(usage.output_tokens)),
            }),
        );
        if !retry {
            return Err(EXHAUSTED.to_string());
        }
        source = format!("{prompt_source}+inline:native_output_limit_recovery");
    }
}

#[cfg(test)]
#[path = "llm_gateway_output_recovery_tests.rs"]
mod tests;
