use std::collections::BTreeSet;

use crate::{AppState, ClaimedTask, RouteResult};
use serde::Deserialize;

const ANSWER_VERIFIER_PROMPT_LOGICAL_PATH: &str = "prompts/answer_verifier_prompt.md";
const MAX_VERIFIER_STEPS: usize = 8;
const DEFAULT_RETRY_INSTRUCTION: &str =
    "retry_policy=use_observed_evidence_and_original_contract;repeat_rejected_answer=false";
const OUTPUT_CONTRACT_ROUTE_MARKERS: &[&str] = &[
    "content_excerpt_summary",
    "content_excerpt_with_summary",
    "scalar_count",
    "scalar_path_only",
    "file_basename",
    "raw_command_output",
    "command_output_summary",
    "file_names",
    "directory_names",
    "directory_entry_groups",
    "file_paths",
    "existence_with_path",
    "existence_with_path_summary",
    "hidden_entries_check",
    "execution_failed_step",
    "generated_file_delivery",
    "generated_file_path_report",
    "filesystem_mutation_result",
    "recent_scalar_equality_check",
    "quantity_comparison",
    "recent_artifacts_judgment",
    "directory_purpose_summary",
    "workspace_project_summary",
    "git_commit_subject",
    "git_repository_state",
    "structured_keys",
    "config_validation",
    "config_mutation",
    "config_risk_assessment",
    "sqlite_table_listing",
    "sqlite_table_names_only",
    "sqlite_database_kind_judgment",
    "sqlite_schema_version",
    "archive_list",
    "archive_read",
    "archive_pack",
    "archive_unpack",
    "service_status",
    "tool_discovery",
];

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
use answer_verifier_runtime::*;
#[cfg(test)]
pub(crate) use answer_verifier_runtime::{
    local_compound_listing_answer_verifier_gap, local_missing_evidence_verifier_gap,
};
pub(crate) use answer_verifier_runtime::{
    post_write_content_evidence_missing_before_verifier, verify_answer_observe_only,
};
use answer_verifier_scalar::*;

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
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    answer_text: &str,
) -> bool {
    let candidate = answer_text.trim();
    if candidate.is_empty() || route_result.needs_clarify {
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
    if context_only_tool_discovery_answer_can_skip_answer_verifier(route_result) {
        return false;
    }
    if terminal_answer_only_can_skip_answer_verifier(route_result, journal) {
        return false;
    }
    if structured_machine_projection_can_skip_answer_verifier(route_result, journal, candidate) {
        return false;
    }
    if pure_chat_agent_loop_submode_can_skip_answer_verifier(route_result, journal) {
        return false;
    }
    let pure_chat_agent_loop = pure_chat_agent_loop_submode_should_verify(route_result, journal);
    if pure_chat_agent_loop {
        return true;
    }
    evidence_policy_requires_observation(route_result)
        || !journal.step_results.is_empty()
        || route_has_output_contract_marker(route_result)
}

fn pure_chat_agent_loop_submode_should_verify(
    route_result: &RouteResult,
    _journal: &crate::task_journal::TaskJournal,
) -> bool {
    if !route_result.uses_pure_chat_agent_loop_submode() {
        return false;
    }
    if route_reason_has_backend_identity_metadata_marker(route_result) {
        return true;
    }
    false
}

fn pure_chat_agent_loop_submode_can_skip_answer_verifier(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    route_result.uses_pure_chat_agent_loop_submode()
        && !route_reason_has_backend_identity_metadata_marker(route_result)
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && !route_has_output_contract_marker(route_result)
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && pure_chat_agent_loop_has_no_tool_observations(journal)
}

fn pure_chat_agent_loop_has_no_tool_observations(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal.step_results.iter().all(|step| {
        matches!(step.skill.as_str(), "respond" | "synthesize_answer")
            && step.status == crate::executor::StepExecutionStatus::Ok
            && step.error_excerpt.is_none()
    })
}

fn route_reason_has_backend_identity_metadata_marker(route_result: &RouteResult) -> bool {
    route_result.has_route_reason_machine_marker("agent_display_name_hint_backend_metadata_removed")
}

fn route_has_output_contract_marker(route_result: &RouteResult) -> bool {
    OUTPUT_CONTRACT_ROUTE_MARKERS
        .iter()
        .any(|marker| route_result.has_route_reason_machine_marker(marker))
}

fn evidence_policy_requires_observation(route_result: &RouteResult) -> bool {
    route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !crate::evidence_policy::required_evidence_fields_for_route(route_result).is_empty()
}

fn context_only_tool_discovery_answer_can_skip_answer_verifier(route_result: &RouteResult) -> bool {
    route_result.has_route_reason_machine_marker("tool_discovery")
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
}

fn terminal_answer_only_can_skip_answer_verifier(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    !route_reason_has_backend_identity_metadata_marker(route_result)
        && !route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && !route_result.wants_file_delivery
        && !route_has_output_contract_marker(route_result)
        && route_result.output_contract.locator_kind == crate::OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
        && pure_chat_agent_loop_has_no_tool_observations(journal)
}

pub(crate) fn structurally_satisfies_answer_contract(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if recent_scalar_equality_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if quantity_comparison_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if directory_purpose_summary_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if recent_artifacts_judgment_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if workspace_project_summary_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if git_repository_state_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if service_status_port_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if health_check_diagnostic_answer_is_grounded_in_successful_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if execution_failed_step_answer_is_grounded_in_failed_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if let Some(shape) = crate::evidence_policy::final_answer_shape_for_route(route_result) {
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
        if shape.class() == crate::evidence_policy::FinalAnswerShapeClass::Table {
            return evidence_policy_table_answer_is_grounded_in_successful_observation(
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
    if archive_unpack_summary_answer_is_grounded_in_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
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
    if markdown_heading_answer_is_grounded_in_read_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if existence_with_path_answer_is_grounded_in_observation(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if structured_keys_answer_is_grounded_in_observation(route_result, journal, candidate_answer) {
        return true;
    }
    scalar_answer_is_grounded_in_successful_observation(route_result, journal, candidate_answer)
}

#[cfg(test)]
#[path = "answer_verifier_tests.rs"]
mod tests;
