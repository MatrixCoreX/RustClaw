use super::*;

#[path = "answer_verifier_runtime/compacted_machine_ref_gap.rs"]
mod compacted_machine_ref_gap;
#[path = "answer_verifier_runtime/compound_listing_gap.rs"]
mod compound_listing_gap;
#[path = "answer_verifier_runtime/prompt_evidence_blocks.rs"]
mod prompt_evidence_blocks;
#[path = "answer_verifier_runtime/structured_read_scalar_gap.rs"]
mod structured_read_scalar_gap;

use compacted_machine_ref_gap::local_compacted_machine_ref_answer_verifier_gap;
pub(crate) use compound_listing_gap::local_compound_listing_answer_verifier_gap;
pub(super) use compound_listing_gap::structured_json_values_from_step_output;
pub(super) use prompt_evidence_blocks::{
    current_context_prompt_block, evidence_policy_context_prompt_block,
    execution_evidence_prompt_block, output_contract_prompt_block,
};
pub(super) use structured_read_scalar_gap::{
    read_range_excerpt_without_line_prefixes,
    scalar_field_value_gap_is_grounded_in_structured_read,
    structured_read_output_contains_scalar_answer,
};

pub(crate) async fn verify_answer_observe_only(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    if let Some(local_gap) =
        local_compacted_machine_ref_answer_verifier_gap(journal, candidate_answer)
    {
        tracing::warn!(
            task_id = %task.task_id,
            missing_evidence_fields = ?local_gap.missing_evidence_fields,
            answer_incomplete_reason = %local_gap.answer_incomplete_reason,
            retry_instruction = %local_gap.retry_instruction,
            "answer_verifier_local_compacted_machine_ref_gap"
        );
        return Some(local_gap);
    }
    if !should_verify_answer(route_result, journal, candidate_answer) {
        return None;
    }
    if let Some(local_gap) =
        local_missing_evidence_verifier_gap_for_answer(route_result, journal, candidate_answer)
    {
        tracing::warn!(
            task_id = %task.task_id,
            missing_evidence_fields = ?local_gap.missing_evidence_fields,
            answer_incomplete_reason = %local_gap.answer_incomplete_reason,
            retry_instruction = %local_gap.retry_instruction,
            "answer_verifier_local_missing_evidence_gap"
        );
        return Some(local_gap);
    }
    if let Some(local_gap) =
        local_compound_listing_answer_verifier_gap(route_result, journal, candidate_answer)
    {
        tracing::warn!(
            task_id = %task.task_id,
            missing_evidence_fields = ?local_gap.missing_evidence_fields,
            answer_incomplete_reason = %local_gap.answer_incomplete_reason,
            retry_instruction = %local_gap.retry_instruction,
            "answer_verifier_local_compound_listing_gap"
        );
        return Some(local_gap);
    }
    if structural_satisfaction_can_skip_verifier(route_result, journal, candidate_answer) {
        tracing::info!(
            task_id = %task.task_id,
            "answer_verifier_skipped_structural_satisfaction"
        );
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
    let evidence_policy_context_prompt = evidence_policy_context_prompt_block(route_result);
    let user_request_for_prompt = answer_verifier_user_request_for_prompt(task, user_request);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__USER_REQUEST__", user_request_for_prompt.trim()),
            ("__REQUEST_LANGUAGE_HINT__", request_language_hint.as_str()),
            (
                "__EVIDENCE_POLICY_CONTEXT__",
                &evidence_policy_context_prompt,
            ),
            (
                "__OUTPUT_CONTRACT__",
                &output_contract_prompt_block(route_result),
            ),
            (
                "__EXECUTION_EVIDENCE__",
                &execution_evidence_prompt_block(journal),
            ),
            (
                "__CURRENT_CONTEXT__",
                &current_context_prompt_block(journal),
            ),
            (
                "__AGENT_RUNTIME_IDENTITY__",
                state.agent_runtime_identity_label(),
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
    if validation.high_confidence_gap()
        && !verifier_gap_requires_visible_answer_repair(&validation)
        && structurally_satisfies_answer_contract(route_result, journal, candidate_answer)
    {
        tracing::info!(
            task_id = %task.task_id,
            answer_incomplete_reason = %validation.answer_incomplete_reason,
            "answer_verifier_gap_suppressed_structural_satisfaction"
        );
        return None;
    }
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

#[cfg(test)]
#[path = "answer_verifier_runtime/compacted_machine_ref_gap_tests.rs"]
mod compacted_machine_ref_gap_tests;

fn verifier_gap_requires_visible_answer_repair(validation: &AnswerVerifierOut) -> bool {
    validation.missing_evidence_fields.iter().any(|field| {
        matches!(
            field.as_str(),
            "output_format" | "unsupported_claims" | "candidates"
        )
    })
}

pub(super) fn answer_verifier_user_request_for_prompt(
    task: &ClaimedTask,
    user_request: &str,
) -> String {
    crate::language_policy::task_user_request_for_prompt(task, user_request)
}

pub(super) fn structural_satisfaction_can_skip_verifier(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if config_guard_machine_payload_can_skip_answer_verifier(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if confirmed_missing_file_delivery_can_skip_answer_verifier(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if route_result.output_contract.requires_content_evidence
        && !route_result.output_contract.delivery_required
        && latest_missing_target_answer_mentions_observed_path(journal, candidate_answer)
    {
        return true;
    }
    local_missing_evidence_verifier_gap_for_answer(route_result, journal, candidate_answer)
        .is_none()
        && (finalizer_summary_can_skip_answer_verifier(route_result, journal)
            || structurally_satisfies_answer_contract(route_result, journal, candidate_answer))
}

pub(super) fn finalizer_summary_can_skip_answer_verifier(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(shape) = crate::evidence_policy::final_answer_shape_for_output_contract(
        &route_result.output_contract,
    ) else {
        return false;
    };
    if shape.allows_model_language() {
        return false;
    }
    let Some(summary) = journal.finalizer_summary.as_ref() else {
        return false;
    };
    summary.contract_ok
        && summary.grounded_ok == Some(true)
        && summary.format_ok == Some(true)
        && summary.completion_ok != Some(false)
        && summary.used_evidence_ids_count > 0
}

pub(super) fn finalizer_missing_target_can_skip_missing_evidence_gap(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    finalizer_summary_can_skip_answer_verifier(route_result, journal)
        && latest_non_control_step_is_missing_target(journal)
}

pub(super) fn finalizer_account_access_error_can_skip_missing_evidence_gap(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    if !route_result.output_contract.requires_content_evidence {
        return false;
    }
    let Some(summary) = journal.finalizer_summary.as_ref() else {
        return false;
    };
    summary.disposition == Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        && summary.completion_ok == Some(true)
        && summary.grounded_ok == Some(true)
        && summary.format_ok != Some(false)
        && summary.used_evidence_ids_count > 0
        && latest_non_control_step_is_crypto_account_access_error(journal)
}

pub(super) fn finalizer_terminal_blocker_can_skip_answer_verifier(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    if !route_result.output_contract.requires_content_evidence {
        return false;
    }
    let Some(summary) = journal.finalizer_summary.as_ref() else {
        return false;
    };
    summary.disposition == Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        && summary.contract_ok
        && summary.completion_ok == Some(true)
        && summary.grounded_ok == Some(true)
        && summary.format_ok != Some(false)
        && summary.used_evidence_ids_count > 0
        && latest_non_control_step_is_terminal_blocker(journal)
}

pub(super) fn latest_non_control_step_is_terminal_blocker(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal
        .step_results
        .iter()
        .rev()
        .find(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        })
        .is_some_and(|step| {
            step.status == crate::executor::StepExecutionStatus::Error
                && step
                    .error_excerpt
                    .as_deref()
                    .is_some_and(step_error_is_terminal_blocker)
        })
}

pub(super) fn latest_non_control_step_is_missing_target(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal
        .step_results
        .iter()
        .rev()
        .find(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        })
        .is_some_and(|step| {
            step.status == crate::executor::StepExecutionStatus::Error
                && step
                    .error_excerpt
                    .as_deref()
                    .is_some_and(step_error_is_missing_target)
        })
}

pub(super) fn step_error_is_missing_target(error: &str) -> bool {
    crate::skills::parse_structured_skill_error(error)
        .is_some_and(|structured| structured.error_kind == "not_found")
        || error.trim().starts_with("__RC_READ_FILE_NOT_FOUND__:")
}

pub(super) fn step_error_is_terminal_blocker(error: &str) -> bool {
    if step_error_is_missing_target(error)
        || crate::skills::error_looks_like_os_permission_denied(error)
    {
        return true;
    }
    crate::skills::parse_structured_skill_error(error).is_some_and(|structured| {
        matches!(
            structured.error_kind.as_str(),
            "permission_denied" | "policy_block" | "path_outside_workspace"
        )
    })
}

pub(super) fn latest_non_control_step_is_crypto_account_access_error(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal
        .step_results
        .iter()
        .rev()
        .find(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        })
        .is_some_and(|step| {
            step.status == crate::executor::StepExecutionStatus::Error
                && step.error_excerpt.as_deref().is_some_and(|error| {
                    crate::skills::is_crypto_account_access_error(&step.skill, error)
                })
        })
}

pub(crate) fn local_missing_evidence_verifier_gap(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
) -> Option<AnswerVerifierOut> {
    let required_evidence_fields =
        crate::evidence_policy::required_evidence_fields_for_output_contract(
            &route_result.output_contract,
        );
    if required_evidence_fields.is_empty() {
        return None;
    }
    let coverage = crate::task_journal::evidence_coverage_for_output_contract(
        &route_result.effective_output_contract(),
        journal,
    );
    if coverage.is_complete() {
        return None;
    }
    if finalizer_missing_target_can_skip_missing_evidence_gap(route_result, journal) {
        return None;
    }
    if finalizer_account_access_error_can_skip_missing_evidence_gap(route_result, journal) {
        return None;
    }
    let missing = answer_verifier_blocking_missing_evidence(coverage.missing_evidence);
    if missing.is_empty() {
        return None;
    }
    let missing_fields = missing.join(",");
    Some(AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: missing.clone(),
        answer_incomplete_reason: format!("missing_required_evidence:{missing_fields}"),
        should_retry: coverage.repair_eligible,
        retry_instruction: format!("collect_required_evidence_fields:{missing_fields}"),
        confidence: coverage.confidence,
    })
}

fn answer_verifier_blocking_missing_evidence(missing_evidence: Vec<String>) -> Vec<String> {
    missing_evidence
        .into_iter()
        .filter(|field| !field.trim().starts_with("negative_evidence("))
        .collect()
}

pub(super) fn local_missing_evidence_verifier_gap_for_answer(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    if config_guard_machine_payload_can_skip_answer_verifier(
        route_result,
        journal,
        candidate_answer,
    ) {
        return None;
    }
    let gap = local_missing_evidence_verifier_gap(route_result, journal)?;
    if missing_target_answer_is_grounded_in_latest_error(
        route_result,
        journal,
        candidate_answer,
        &gap,
    ) {
        return None;
    }
    if scalar_field_value_gap_is_grounded_in_structured_read(
        route_result,
        journal,
        candidate_answer,
        &gap,
    ) {
        return None;
    }
    if confirmed_missing_file_delivery_can_skip_answer_verifier(
        route_result,
        journal,
        candidate_answer,
    ) {
        return None;
    }
    if non_path_status_observation_can_skip_path_content_gap(route_result, journal, &gap) {
        return None;
    }
    Some(gap)
}

fn non_path_status_observation_can_skip_path_content_gap(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    gap: &AnswerVerifierOut,
) -> bool {
    if !route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.semantic_kind_is_unclassified()
    {
        return false;
    }
    if !gap
        .missing_evidence_fields
        .iter()
        .any(|field| field == "path")
    {
        return false;
    }
    let non_control_steps: Vec<_> = journal
        .step_results
        .iter()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        })
        .collect();
    !non_control_steps.is_empty()
        && non_control_steps.iter().all(|step| {
            step.status == crate::executor::StepExecutionStatus::Ok
                && non_path_status_skill(step.skill.as_str())
                && step_output_has_status_observation(step.output_excerpt.as_deref())
        })
}

fn non_path_status_skill(skill: &str) -> bool {
    matches!(
        skill,
        "docker_basic"
            | "git_basic"
            | "health_check"
            | "http_basic"
            | "package_manager"
            | "process_basic"
            | "service_control"
            | "system_basic"
            | "task_control"
    )
}

fn step_output_has_status_observation(output: Option<&str>) -> bool {
    let Some(output) = output.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output) else {
        return false;
    };
    status_observation_value(&value)
}

fn status_observation_value(value: &serde_json::Value) -> bool {
    if value
        .get("available")
        .and_then(serde_json::Value::as_bool)
        .is_some()
        || value
            .get("command_succeeded")
            .and_then(serde_json::Value::as_bool)
            .is_some()
        || value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .is_some()
        || value
            .get("field_value")
            .is_some_and(|field_value| !field_value.is_null())
        || value
            .get("count")
            .and_then(serde_json::Value::as_u64)
            .is_some()
    {
        return true;
    }
    value.get("extra").is_some_and(status_observation_value)
}

fn config_guard_machine_payload_can_skip_answer_verifier(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let contract = route_result.effective_output_contract();
    if contract.delivery_required
        || !route_result.output_contract_marker_is_any(&[
            crate::OutputSemanticKind::ConfigRiskAssessment,
            crate::OutputSemanticKind::ConfigValidation,
        ])
    {
        return false;
    }
    if !is_config_guard_machine_payload(candidate_answer) {
        return false;
    }
    finalizer_grounded_machine_payload_can_skip_verifier(journal)
}

fn is_config_guard_machine_payload(candidate_answer: &str) -> bool {
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(candidate_answer.trim())
    else {
        return false;
    };
    let Some(message_key) = object
        .get("message_key")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    matches!(
        message_key,
        "clawd.msg.config_edit.guard" | "clawd.msg.config_risk.summary"
    ) && object.contains_key("path")
        && (object.contains_key("risk_count")
            || object.contains_key("count")
            || object.contains_key("candidates")
            || object.contains_key("risks"))
}

fn finalizer_grounded_machine_payload_can_skip_verifier(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(summary) = journal.finalizer_summary.as_ref() else {
        return false;
    };
    summary.disposition == Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
        && summary.contract_ok
        && summary.completion_ok != Some(false)
        && summary.grounded_ok == Some(true)
        && summary.format_ok != Some(false)
        && summary.used_evidence_ids_count > 0
}

pub(super) fn structured_machine_projection_can_skip_answer_verifier(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if journal_grounded_local_code_strict_json_projection_can_skip_answer_verifier(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    if super::answer_verifier_machine_kv::requested_machine_kv_projection_can_skip_answer_verifier(
        route_result,
        journal,
        candidate_answer,
    ) {
        return true;
    }
    finalizer_grounded_machine_payload_can_skip_verifier(journal)
        && (is_structured_machine_projection(candidate_answer)
            || super::answer_verifier_control_envelope::control_machine_envelope_answer_can_skip_answer_verifier(candidate_answer))
}

pub(crate) fn post_write_content_evidence_missing_before_verifier(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    local_code_strict_json_projection_has_supported_keys(candidate_answer)
        && journal_has_code_write_validation_missing_readback(journal)
}

fn journal_grounded_local_code_strict_json_projection_can_skip_answer_verifier(
    route_result: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let contract = route_result.effective_output_contract();
    if !local_code_strict_json_projection_has_supported_keys(candidate_answer)
        || !journal_has_successful_nonterminal_observation(journal)
    {
        return false;
    }
    let strict_contract_matches = contract.requires_content_evidence
        && !contract.delivery_required
        && matches!(contract.response_shape, crate::OutputResponseShape::Strict)
        && (latest_synthesis_step_matches_candidate(journal, candidate_answer)
            || strict_json_projection_observation_matches_candidate(journal, candidate_answer))
        && crate::task_journal::evidence_coverage_for_output_contract(
            &route_result.effective_output_contract(),
            journal,
        )
        .is_complete();
    let publishable_projection_matches =
        strict_json_projection_observation_matches_candidate(journal, candidate_answer)
            && (journal_has_code_write_readback_validation_evidence(journal)
                || journal_has_readback_only_code_validation_evidence(journal));
    strict_contract_matches || publishable_projection_matches
}

fn strict_json_projection_observation_matches_candidate(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate = candidate_answer.trim();
    if candidate.is_empty() {
        return false;
    }
    journal.task_observations.iter().rev().any(|observation| {
        observation.get("kind").and_then(serde_json::Value::as_str)
            == Some("agent_loop_strict_json_projection")
            && observation
                .get("owner_layer")
                .and_then(serde_json::Value::as_str)
                == Some("agent_loop")
            && observation
                .get("publishable")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            && observation
                .get("output")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                == Some(candidate)
    })
}

fn local_code_strict_json_projection_has_supported_keys(candidate_answer: &str) -> bool {
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(candidate_answer.trim())
    else {
        return false;
    };
    if object.len() < 2 || object.len() > 8 {
        return false;
    }
    object.iter().all(|(key, value)| {
        matches!(
            key.as_str(),
            "created_files"
                | "changed_files"
                | "test_command"
                | "test_status"
                | "functions"
                | "error_codes"
                | "evidence_files"
                | "project_dir"
        ) && json_machine_projection_value_has_payload(value)
    })
}

fn latest_synthesis_step_matches_candidate(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let candidate = candidate_answer.trim();
    journal
        .step_results
        .iter()
        .rev()
        .find(|step| {
            step.status == crate::executor::StepExecutionStatus::Ok
                && step.skill == "synthesize_answer"
        })
        .is_some_and(|step| {
            step.output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| output == candidate)
        })
}

fn journal_has_successful_nonterminal_observation(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
            && step
                .output_excerpt
                .as_deref()
                .map(str::trim)
                .is_some_and(|output| !output.is_empty())
    })
}

fn journal_has_code_write_readback_validation_evidence(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let (written_paths, readback_paths, validation_ok) =
        journal_code_write_readback_validation_state(journal);
    validation_ok
        && !written_paths.is_empty()
        && written_paths
            .iter()
            .all(|path| readback_paths.contains(path))
}

fn journal_has_code_write_validation_missing_readback(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let (written_paths, readback_paths, validation_ok) =
        journal_code_write_readback_validation_state(journal);
    validation_ok
        && !written_paths.is_empty()
        && written_paths
            .iter()
            .any(|path| !readback_paths.contains(path))
}

fn journal_has_readback_only_code_validation_evidence(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let mut source_dirs = std::collections::BTreeSet::new();
    let mut test_dirs = std::collections::BTreeSet::new();
    let mut validation_ok = false;
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if step.skill == "run_cmd" {
            validation_ok = true;
            continue;
        }
        let Some(extra) = step_output_extra_object(step) else {
            continue;
        };
        if !matches!(
            extra.get("action").and_then(serde_json::Value::as_str),
            Some("read_range" | "read_text_range")
        ) {
            continue;
        }
        if !extra
            .get("excerpt")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|excerpt| !excerpt.trim().is_empty())
        {
            continue;
        }
        let Some(path) = extra
            .get("resolved_path")
            .or_else(|| extra.get("effective_path"))
            .or_else(|| extra.get("path"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty())
        else {
            continue;
        };
        let Some(parent) = normalized_code_evidence_parent(path) else {
            continue;
        };
        if path_looks_like_test_code_file(path) {
            test_dirs.insert(parent);
        } else if path_looks_like_code_file(path) {
            source_dirs.insert(parent);
        }
    }
    validation_ok
        && !source_dirs.is_empty()
        && !test_dirs.is_empty()
        && source_dirs.iter().any(|dir| test_dirs.contains(dir))
}

fn journal_code_write_readback_validation_state(
    journal: &crate::task_journal::TaskJournal,
) -> (
    std::collections::BTreeSet<String>,
    std::collections::BTreeSet<String>,
    bool,
) {
    let mut latest_write_indices = std::collections::BTreeMap::<String, usize>::new();
    let mut readback_records = Vec::<(String, usize)>::new();
    let mut validation_indices = Vec::<usize>::new();
    let mut readback_paths = std::collections::BTreeSet::new();
    for (index, step) in journal.step_results.iter().enumerate() {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        if step.skill == "run_cmd" {
            validation_indices.push(index);
            continue;
        }
        let Some(extra) = step_output_extra_object(step) else {
            continue;
        };
        let action = extra.get("action").and_then(serde_json::Value::as_str);
        let path = extra
            .get("resolved_path")
            .or_else(|| extra.get("effective_path"))
            .or_else(|| extra.get("path"))
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|path| !path.is_empty());
        match (action, path) {
            (Some("write_text" | "append_text"), Some(path)) => {
                latest_write_indices.insert(normalize_code_evidence_path(path), index);
            }
            (Some("read_range" | "read_text_range"), Some(path))
                if extra
                    .get("excerpt")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|excerpt| !excerpt.trim().is_empty()) =>
            {
                readback_records.push((normalize_code_evidence_path(path), index));
            }
            _ => {}
        }
    }
    let written_paths = latest_write_indices
        .keys()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    for (written_path, write_index) in &latest_write_indices {
        if readback_records.iter().any(|(readback_path, read_index)| {
            *read_index > *write_index && code_evidence_paths_match(readback_path, written_path)
        }) {
            readback_paths.insert(written_path.clone());
        }
    }
    let validation_ok =
        latest_write_indices
            .values()
            .max()
            .copied()
            .is_some_and(|latest_write_index| {
                validation_indices
                    .iter()
                    .any(|validation_index| *validation_index > latest_write_index)
            });
    (written_paths, readback_paths, validation_ok)
}

fn normalize_code_evidence_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn normalized_code_evidence_parent(path: &str) -> Option<String> {
    let normalized = normalize_code_evidence_path(path);
    std::path::Path::new(&normalized)
        .parent()
        .map(|parent| parent.to_string_lossy().replace('\\', "/"))
        .filter(|parent| !parent.trim().is_empty())
}

fn path_looks_like_code_file(path: &str) -> bool {
    let normalized = normalize_code_evidence_path(path);
    let extension = std::path::Path::new(&normalized)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase());
    matches!(
        extension.as_deref(),
        Some(
            "py" | "rs"
                | "js"
                | "jsx"
                | "ts"
                | "tsx"
                | "go"
                | "java"
                | "c"
                | "h"
                | "cc"
                | "cpp"
                | "hpp"
                | "cs"
                | "rb"
                | "php"
                | "swift"
                | "kt"
                | "scala"
                | "sh"
        )
    )
}

fn path_looks_like_test_code_file(path: &str) -> bool {
    if !path_looks_like_code_file(path) {
        return false;
    }
    let normalized = normalize_code_evidence_path(path);
    let Some(name) = std::path::Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_ascii_lowercase())
    else {
        return false;
    };
    name.starts_with("test_")
        || name.ends_with("_test.py")
        || name.ends_with("_test.rs")
        || name.ends_with(".test.js")
        || name.ends_with(".test.ts")
        || name.ends_with(".spec.js")
        || name.ends_with(".spec.ts")
}

fn code_evidence_paths_match(candidate: &str, expected: &str) -> bool {
    let candidate = normalize_code_evidence_path(candidate);
    let expected = normalize_code_evidence_path(expected);
    if candidate == expected {
        return true;
    }
    let (shorter, longer) = if candidate.len() <= expected.len() {
        (candidate.as_str(), expected.as_str())
    } else {
        (expected.as_str(), candidate.as_str())
    };
    !shorter.starts_with('/') && !shorter.is_empty() && longer.ends_with(&format!("/{shorter}"))
}

fn step_output_extra_object(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Option<serde_json::Map<String, serde_json::Value>> {
    let output = step.output_excerpt.as_deref()?.trim();
    let value = serde_json::from_str::<serde_json::Value>(output).ok()?;
    value
        .get("extra")
        .and_then(serde_json::Value::as_object)
        .or_else(|| value.as_object())
        .cloned()
}

fn is_structured_machine_projection(candidate_answer: &str) -> bool {
    if json_object_is_structured_machine_projection(candidate_answer) {
        return true;
    }
    let mut field_count = 0usize;
    let mut anchor_count = 0usize;
    for line in candidate_answer
        .trim()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let Some((key, value)) = line.split_once('=') else {
            return false;
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() || value.is_empty() || !is_machine_projection_key(key) {
            return false;
        }
        field_count += 1;
        if machine_projection_key_is_anchor(key) || machine_projection_value_is_structured(value) {
            anchor_count += 1;
        }
    }
    field_count >= 2 && anchor_count > 0
}

fn json_object_is_structured_machine_projection(candidate_answer: &str) -> bool {
    let Ok(serde_json::Value::Object(object)) =
        serde_json::from_str::<serde_json::Value>(candidate_answer.trim())
    else {
        return false;
    };
    if object.len() < 2 || object.len() > 24 {
        return false;
    }
    let mut anchor_count = 0usize;
    for (key, value) in &object {
        if !is_machine_projection_key(key) || !json_machine_projection_value_has_payload(value) {
            return false;
        }
        if machine_projection_key_is_anchor(key)
            || json_machine_projection_value_is_structured(value)
        {
            anchor_count += 1;
        }
    }
    anchor_count > 0
}

fn json_machine_projection_value_has_payload(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => false,
        serde_json::Value::String(text) => json_machine_projection_string_has_payload(text),
        serde_json::Value::Array(items) => {
            !items.is_empty() && items.iter().all(json_machine_projection_value_has_payload)
        }
        serde_json::Value::Object(object) => {
            !object.is_empty()
                && object
                    .values()
                    .all(json_machine_projection_value_has_payload)
        }
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => true,
    }
}

fn json_machine_projection_string_has_payload(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty()
        && !trimmed.contains("{{")
        && !matches!(trimmed, "<missing>" | "not_observed" | "null")
}

fn json_machine_projection_value_is_structured(value: &serde_json::Value) -> bool {
    matches!(
        value,
        serde_json::Value::Array(_) | serde_json::Value::Object(_)
    )
}

fn is_machine_projection_key(key: &str) -> bool {
    key.chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '[' | ']'))
}

fn machine_projection_key_is_anchor(key: &str) -> bool {
    key.contains('.') || key.contains('[') || key.contains(']') || {
        matches!(
            key,
            "async_cancel_adapter_result"
                | "async_poll_adapter_result"
                | "dry_run"
                | "job_id"
                | "message_key"
                | "model"
                | "model_kind"
                | "output_path"
                | "planned_outputs"
                | "provider"
                | "status"
                | "task_id"
        )
    }
}

fn machine_projection_value_is_structured(value: &str) -> bool {
    value.starts_with('{') || value.starts_with('[')
}

pub(super) fn missing_target_answer_is_grounded_in_latest_error(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
    gap: &AnswerVerifierOut,
) -> bool {
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !missing_gap_allows_negative_path_evidence(gap)
    {
        return false;
    }
    latest_missing_target_answer_mentions_observed_path(journal, candidate_answer)
}

fn latest_missing_target_answer_mentions_observed_path(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let paths = latest_missing_target_paths(journal);
    !paths.is_empty()
        && paths
            .iter()
            .any(|path| candidate_answer_contains_machine_path(candidate_answer, path))
}

fn missing_gap_allows_negative_path_evidence(gap: &AnswerVerifierOut) -> bool {
    gap.missing_evidence_fields.iter().any(|field| {
        field == "content_excerpt"
            || field == "candidates"
            || field == "path"
            || field.contains("content_excerpt")
            || field.contains("candidates")
    })
}

fn confirmed_missing_file_delivery_can_skip_answer_verifier(
    route: &AnswerContract,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    route.output_contract.delivery_required
        && route.output_contract.response_shape == crate::OutputResponseShape::FileToken
        && !candidate_answer_has_file_delivery_token(candidate_answer)
        && journal_has_confirmed_missing_file_delivery_evidence(journal)
}

fn candidate_answer_has_file_delivery_token(candidate_answer: &str) -> bool {
    candidate_answer
        .lines()
        .any(|line| crate::finalize::parse_delivery_file_token(line.trim()).is_some())
}

fn journal_has_confirmed_missing_file_delivery_evidence(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        })
        .any(step_has_confirmed_missing_file_delivery_evidence)
}

fn step_has_confirmed_missing_file_delivery_evidence(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> bool {
    step.output_excerpt
        .as_deref()
        .is_some_and(output_has_confirmed_missing_file_delivery_evidence)
        || step
            .error_excerpt
            .as_deref()
            .is_some_and(step_error_is_missing_target)
}

fn output_has_confirmed_missing_file_delivery_evidence(output: &str) -> bool {
    structured_json_values_from_step_output(output)
        .iter()
        .any(value_has_confirmed_missing_file_delivery_evidence)
}

fn value_has_confirmed_missing_file_delivery_evidence(value: &serde_json::Value) -> bool {
    if value.get("exists").and_then(serde_json::Value::as_bool) == Some(false) {
        return true;
    }
    let action = value.get("action").and_then(serde_json::Value::as_str);
    let count = value.get("count").and_then(serde_json::Value::as_u64);
    let has_empty_results = ["results", "matches"].iter().any(|field| {
        value
            .get(field)
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty)
    });
    matches!(action, Some("find_name" | "find_path")) && count == Some(0) && has_empty_results
}

fn latest_missing_target_paths(journal: &crate::task_journal::TaskJournal) -> Vec<String> {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| {
            !matches!(
                step.skill.as_str(),
                "respond" | "synthesize_answer" | "think" | "answer_verifier"
            )
        })
        .find_map(|step| {
            let paths = missing_target_paths_from_step_result(step);
            (!paths.is_empty()).then_some(paths)
        })
        .unwrap_or_default()
}

fn missing_target_paths_from_step_result(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Vec<String> {
    if step.status == crate::executor::StepExecutionStatus::Error {
        return step
            .error_excerpt
            .as_deref()
            .filter(|error| step_error_is_missing_target(error))
            .map(missing_target_paths_from_error)
            .unwrap_or_default();
    }
    if step.status != crate::executor::StepExecutionStatus::Ok {
        return Vec::new();
    }
    step.output_excerpt
        .as_deref()
        .map(missing_target_paths_from_output)
        .unwrap_or_default()
}

fn missing_target_paths_from_error(error: &str) -> Vec<String> {
    let mut paths = Vec::new();
    if let Some(path) = error.trim().strip_prefix("__RC_READ_FILE_NOT_FOUND__:") {
        push_non_empty_path(&mut paths, path);
    }
    if let Some(structured) = crate::skills::parse_structured_skill_error(error) {
        if let Some(extra) = structured.extra.as_ref() {
            collect_path_like_strings(extra, &mut paths);
        }
    }
    paths.sort();
    paths.dedup();
    paths
}

fn missing_target_paths_from_output(output: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    collect_missing_target_paths_from_json(&value, &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

fn collect_missing_target_paths_from_json(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("exists").and_then(serde_json::Value::as_bool) == Some(false) {
                collect_path_like_strings(value, paths);
            }
            for value in map.values() {
                if let Some(text) = value.as_str() {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(text.trim()) {
                        collect_missing_target_paths_from_json(&parsed, paths);
                    }
                }
                collect_missing_target_paths_from_json(value, paths);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_missing_target_paths_from_json(item, paths);
            }
        }
        _ => {}
    }
}

fn collect_path_like_strings(value: &serde_json::Value, paths: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, value) in map {
                if matches!(
                    key.as_str(),
                    "path" | "resolved_path" | "target" | "locator" | "root"
                ) {
                    if let Some(path) = value.as_str() {
                        push_non_empty_path(paths, path);
                    }
                }
                collect_path_like_strings(value, paths);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_path_like_strings(item, paths);
            }
        }
        _ => {}
    }
}

fn push_non_empty_path(paths: &mut Vec<String>, path: &str) {
    let path = path.trim();
    if !path.is_empty() {
        paths.push(path.to_string());
    }
}

fn candidate_answer_contains_machine_path(candidate_answer: &str, path: &str) -> bool {
    !path.trim().is_empty() && candidate_answer.contains(path.trim())
}
