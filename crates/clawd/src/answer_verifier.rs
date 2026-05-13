use serde::Deserialize;
use serde_json::json;

use crate::{AppState, ClaimedTask, RouteResult, TaskContract};

const ANSWER_VERIFIER_PROMPT_LOGICAL_PATH: &str = "prompts/answer_verifier_prompt.md";
const MAX_VERIFIER_STEPS: usize = 8;
const DEFAULT_RETRY_INSTRUCTION: &str = "Re-answer using the observed execution evidence and the original user request/output contract. Do not repeat the rejected answer.";

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub(crate) struct AnswerVerifierOut {
    #[serde(default)]
    pub(crate) pass: bool,
    #[serde(default)]
    pub(crate) missing_evidence_fields: Vec<String>,
    #[serde(default)]
    pub(crate) answer_incomplete_reason: String,
    #[serde(default)]
    pub(crate) should_retry: bool,
    #[serde(default)]
    pub(crate) retry_instruction: String,
    #[serde(default)]
    pub(crate) confidence: f64,
}

impl AnswerVerifierOut {
    pub(crate) fn normalized(mut self) -> Self {
        self.confidence = self.confidence.clamp(0.0, 1.0);
        self.missing_evidence_fields = self
            .missing_evidence_fields
            .into_iter()
            .map(|field| field.trim().to_string())
            .filter(|field| !field.is_empty())
            .collect();
        self.retry_instruction = self.retry_instruction.trim().to_string();
        self.answer_incomplete_reason = self.answer_incomplete_reason.trim().to_string();
        if self.high_confidence_gap() {
            self.should_retry = true;
            if self.retry_instruction.is_empty() {
                self.retry_instruction = DEFAULT_RETRY_INSTRUCTION.to_string();
            }
        }
        self
    }

    pub(crate) fn high_confidence_gap(&self) -> bool {
        self.confidence >= 0.55 && !self.pass
    }
}

pub(crate) fn should_verify_answer(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
) -> bool {
    let candidate = answer_text.trim();
    if candidate.is_empty() || route_result.needs_clarify || route_result.is_clarify_gate() {
        return false;
    }
    if matches!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    ) {
        return false;
    }
    let task_contract = TaskContract::from_route_result(route_result);
    if task_contract.intent_kind.as_str() != "planner_execute" {
        return false;
    }
    task_contract.evidence_required
        || !journal.step_results.is_empty()
        || route_result.output_contract.semantic_kind != crate::OutputSemanticKind::None
}

pub(crate) async fn verify_answer_observe_only(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    if !should_verify_answer(route_result, journal, candidate_answer) {
        return None;
    }
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        ANSWER_VERIFIER_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            tracing::info!(
                "answer_verifier prompt_missing task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    let task_contract = TaskContract::from_route_result(route_result);
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__USER_REQUEST__", user_request.trim()),
            (
                "__TASK_CONTRACT__",
                &task_contract_prompt_block(&task_contract),
            ),
            (
                "__OUTPUT_CONTRACT__",
                &output_contract_prompt_block(route_result),
            ),
            (
                "__EXECUTION_EVIDENCE__",
                &execution_evidence_prompt_block(journal),
            ),
            ("__CANDIDATE_ANSWER__", candidate_answer.trim()),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "answer_verifier_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out = match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            tracing::info!(
                "answer_verifier llm_failed task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    let validation = match crate::prompt_utils::validate_against_schema::<AnswerVerifierOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::AnswerVerifier,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok || validated.schema_normalized {
                tracing::info!(
                        "answer_verifier schema_parse_recovery task_id={} raw_parse_ok={} schema_normalized={}",
                        task.task_id,
                        validated.raw_parse_ok,
                        validated.schema_normalized
                    );
            }
            validated.value.normalized()
        }
        Err(err) => {
            tracing::info!(
                "answer_verifier schema_validation_failed task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    if validation.high_confidence_gap() {
        tracing::warn!(
            task_id = %task.task_id,
            missing_evidence_fields = ?validation.missing_evidence_fields,
            answer_incomplete_reason = %validation.answer_incomplete_reason,
            should_retry = validation.should_retry,
            retry_instruction = %validation.retry_instruction,
            confidence = validation.confidence,
            "answer_verifier_observed_gap"
        );
    }
    Some(validation)
}

fn task_contract_prompt_block(task_contract: &TaskContract) -> String {
    task_contract.compact_prompt_line()
}

fn output_contract_prompt_block(route_result: &RouteResult) -> String {
    serde_json::to_string_pretty(&json!({
        "response_shape": route_result.output_contract.response_shape.as_str(),
        "requires_content_evidence": route_result.output_contract.requires_content_evidence,
        "delivery_required": route_result.output_contract.delivery_required,
        "locator_kind": route_result.output_contract.locator_kind.as_str(),
        "delivery_intent": route_result.output_contract.delivery_intent.as_str(),
        "semantic_kind": route_result.output_contract.semantic_kind.as_str(),
        "locator_hint": route_result.output_contract.locator_hint,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn execution_evidence_prompt_block(journal: &crate::task_journal::TaskJournal) -> String {
    let mut steps = journal
        .step_results
        .iter()
        .rev()
        .take(MAX_VERIFIER_STEPS)
        .map(|step| {
            json!({
                "step_id": step.step_id,
                "skill": step.skill,
                "status": step.status.as_str(),
                "output_excerpt": step.output_excerpt,
                "error_excerpt": step.error_excerpt,
            })
        })
        .collect::<Vec<_>>();
    steps.reverse();
    serde_json::to_string_pretty(&steps).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{should_verify_answer, AnswerVerifierOut};

    fn route_with_mode(routed_mode: crate::RoutedMode) -> crate::RouteResult {
        crate::RouteResult {
            routed_mode,
            ask_mode: crate::AskMode::from_routed_mode(routed_mode),
            resolved_intent: "test intent".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "test".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        }
    }

    #[test]
    fn answer_verifier_schema_accepts_typed_output() {
        let raw = json!({
            "pass": false,
            "missing_evidence_fields": ["size_bytes"],
            "answer_incomplete_reason": "missing requested size evidence",
            "should_retry": true,
            "retry_instruction": "Collect file metadata and answer with path plus size.",
            "confidence": 0.86
        });
        let validated = crate::prompt_utils::validate_against_schema::<AnswerVerifierOut>(
            &raw.to_string(),
            crate::prompt_utils::PromptSchemaId::AnswerVerifier,
        )
        .expect("schema should validate answer verifier output");
        assert!(!validated.value.pass);
        assert!(validated.value.should_retry);
    }

    #[test]
    fn answer_verifier_gap_is_high_confidence_only() {
        let low = AnswerVerifierOut {
            pass: false,
            confidence: 0.2,
            ..AnswerVerifierOut::default()
        };
        let high = AnswerVerifierOut {
            pass: false,
            confidence: 0.8,
            ..AnswerVerifierOut::default()
        };
        assert!(!low.high_confidence_gap());
        assert!(high.high_confidence_gap());
    }

    #[test]
    fn answer_verifier_normalizes_high_confidence_gap_to_retry() {
        let normalized = AnswerVerifierOut {
            pass: false,
            confidence: 0.82,
            retry_instruction: "  ".to_string(),
            ..AnswerVerifierOut::default()
        }
        .normalized();
        assert!(normalized.should_retry);
        assert!(!normalized.retry_instruction.trim().is_empty());
    }

    #[test]
    fn direct_answer_route_skips_answer_verifier() {
        let route = route_with_mode(crate::RoutedMode::Chat);
        let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
        assert!(!should_verify_answer(&route, &journal, "hi"));
    }

    #[test]
    fn clarify_final_status_skips_answer_verifier() {
        let mut route = route_with_mode(crate::RoutedMode::ChatAct);
        route.output_contract.requires_content_evidence = true;
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

        assert!(!should_verify_answer(
            &route,
            &journal,
            "please provide the missing path"
        ));
    }
}
