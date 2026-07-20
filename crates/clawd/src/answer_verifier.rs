use std::collections::BTreeSet;

use crate::{AppState, ClaimedTask};
use serde::Deserialize;

const ANSWER_VERIFIER_PROMPT_LOGICAL_PATH: &str = "prompts/answer_verifier_prompt.md";
const MAX_VERIFIER_STEPS: usize = 8;
const DEFAULT_RETRY_INSTRUCTION: &str =
    "retry_policy=use_observed_evidence_and_original_contract;repeat_rejected_answer=false";
#[path = "answer_verifier_control_envelope.rs"]
mod answer_verifier_control_envelope;
#[path = "answer_verifier_delivery_raw.rs"]
mod answer_verifier_delivery_raw;
#[path = "answer_verifier_evidence_policy.rs"]
mod answer_verifier_evidence_policy;
#[path = "answer_verifier_machine_kv.rs"]
mod answer_verifier_machine_kv;
#[path = "answer_verifier_runtime.rs"]
mod answer_verifier_runtime;
#[path = "answer_verifier_scalar.rs"]
mod answer_verifier_scalar;

use answer_verifier_delivery_raw::*;
use answer_verifier_evidence_policy::*;
#[cfg(test)]
pub(crate) use answer_verifier_runtime::local_missing_evidence_verifier_gap;
use answer_verifier_runtime::*;
pub(crate) use answer_verifier_runtime::{
    post_write_content_evidence_missing_before_verifier, verify_answer_observe_only,
};
use answer_verifier_scalar::*;

#[derive(Debug, Clone)]
pub(crate) struct AnswerContract {
    pub(crate) request_text: String,
    pub(crate) output_contract: crate::IntentOutputContract,
}

impl AnswerContract {
    pub(crate) fn new(
        request_text: impl Into<String>,
        output_contract: crate::IntentOutputContract,
    ) -> Self {
        Self {
            request_text: request_text.into(),
            output_contract,
        }
    }

    pub(crate) fn output_contract_marker_is(
        &self,
        semantic_kind: crate::OutputSemanticKind,
    ) -> bool {
        self.output_contract.semantic_kind_is(semantic_kind)
    }

    pub(crate) fn output_contract_is_unclassified(&self) -> bool {
        self.output_contract.semantic_kind_is_unclassified()
    }

    pub(crate) fn effective_output_contract(&self) -> crate::IntentOutputContract {
        self.output_contract.clone()
    }
}

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
        !self.pass
            && (self.confidence >= 0.55
                || (self.should_retry
                    && (!self.answer_incomplete_reason.is_empty()
                        || !self.missing_evidence_fields.is_empty())))
    }
}

pub(crate) fn should_verify_answer(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
) -> bool {
    let candidate = answer_text.trim();
    if candidate.is_empty() {
        return false;
    }
    if matches!(
        journal.final_status,
        Some(crate::task_journal::TaskJournalFinalStatus::Clarify)
    ) {
        return false;
    }
    if finalizer_terminal_blocker_can_skip_answer_verifier(route_result, journal) {
        return false;
    }
    if terminal_answer_only_can_skip_answer_verifier(route_result, journal) {
        return false;
    }
    if structured_machine_projection_can_skip_answer_verifier(route_result, journal, candidate) {
        return false;
    }
    evidence_policy_requires_observation(route_result)
        || !journal.step_results.is_empty()
        || route_has_output_contract_marker(route_result)
}

fn terminal_answer_has_no_tool_observations(journal: &crate::task_journal::TaskJournal) -> bool {
    journal.step_results.iter().all(|step| {
        matches!(step.skill.as_str(), "respond" | "synthesize_answer")
            && step.status == crate::executor::StepExecutionStatus::Ok
            && step.error_excerpt.is_none()
    })
}

fn route_has_output_contract_marker(route_result: &AnswerContract) -> bool {
    !route_result.output_contract.semantic_kind_is_unclassified()
}

fn evidence_policy_requires_observation(route_result: &AnswerContract) -> bool {
    route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !crate::evidence_policy::required_evidence_fields_for_output_contract(
            &route_result.output_contract,
        )
        .is_empty()
}

fn terminal_answer_only_can_skip_answer_verifier(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_has_output_contract_marker(route_result)
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && terminal_answer_has_no_tool_observations(journal)
}

pub(crate) fn structurally_satisfies_answer_contract(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if let Some(shape) = crate::evidence_policy::final_answer_shape_for_output_contract(
        &route_result.output_contract,
    ) {
        if shape.class() == crate::evidence_policy::FinalAnswerShapeClass::ScalarValue {
            return evidence_policy_scalar_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
        if shape.class() == crate::evidence_policy::FinalAnswerShapeClass::StrictList {
            return evidence_policy_strict_list_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
        if shape.class() == crate::evidence_policy::FinalAnswerShapeClass::SinglePath {
            return evidence_policy_single_path_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
        if shape.class() == crate::evidence_policy::FinalAnswerShapeClass::DeliveryArtifact {
            return evidence_policy_delivery_artifact_answer_is_grounded_in_successful_observation(
                route_result,
                journal,
                candidate_answer,
            );
        }
    }
    if route_requires_single_file_delivery(route_result)
        && candidate_answer_has_grounded_existing_file_token(journal, candidate_answer)
    {
        return true;
    }
    if raw_bounded_read_answer_is_exact_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if raw_command_answer_is_exact_successful_observation(journal, candidate_answer) {
        return true;
    }
    if existence_with_path_answer_is_grounded_in_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    scalar_answer_is_grounded_in_successful_observation(route_result, journal, candidate_answer)
}

#[cfg(test)]
#[path = "answer_verifier_tests.rs"]
mod tests;
