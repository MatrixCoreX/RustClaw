use super::*;

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
    if let Some(identity_guard) =
        backend_identity_metadata_answer_verifier_guard(state, route_result, candidate_answer)
    {
        tracing::info!(
            task_id = %task.task_id,
            pass = identity_guard.pass,
            answer_incomplete_reason = %identity_guard.answer_incomplete_reason,
            "answer_verifier_backend_identity_metadata_guard"
        );
        return Some(identity_guard);
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
    let task_contract = TaskContract::from_route_result(route_result);
    let user_request_for_prompt = answer_verifier_user_request_for_prompt(task, user_request);
    let request_language_hint =
        crate::language_policy::task_response_language_hint(state, task, user_request);
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__USER_REQUEST__", user_request_for_prompt.trim()),
            ("__REQUEST_LANGUAGE_HINT__", request_language_hint.as_str()),
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

pub(super) fn answer_verifier_user_request_for_prompt(
    task: &ClaimedTask,
    user_request: &str,
) -> String {
    crate::language_policy::task_user_request_for_prompt(task, user_request)
}

pub(super) fn backend_identity_metadata_answer_verifier_guard(
    state: &AppState,
    route_result: &RouteResult,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    if !route_reason_has_backend_identity_metadata_marker(route_result) {
        return None;
    }
    if candidate_answer_mentions_backend_identity_metadata(state, candidate_answer) {
        return Some(AnswerVerifierOut {
            pass: false,
            missing_evidence_fields: vec!["identity".to_string()],
            answer_incomplete_reason: "backend_identity_metadata_in_final_answer".to_string(),
            should_retry: true,
            retry_instruction: "answer_with_agent_runtime_identity".to_string(),
            confidence: 0.98,
        });
    }
    if candidate_answer_is_runtime_identity_label(state, candidate_answer) {
        return Some(AnswerVerifierOut {
            pass: true,
            missing_evidence_fields: Vec::new(),
            answer_incomplete_reason: String::new(),
            should_retry: false,
            retry_instruction: String::new(),
            confidence: 0.99,
        });
    }
    None
}

fn route_reason_has_backend_identity_metadata_marker(route_result: &RouteResult) -> bool {
    route_result
        .route_reason
        .contains("agent_display_name_hint_backend_metadata_removed")
}

fn candidate_answer_mentions_backend_identity_metadata(
    state: &AppState,
    candidate_answer: &str,
) -> bool {
    let normalized_answer = normalize_identity_metadata_token(candidate_answer);
    if normalized_answer.is_empty() {
        return false;
    }
    state.core.llm_providers.iter().any(|provider| {
        provider
            .config
            .name
            .trim()
            .strip_prefix("vendor-")
            .into_iter()
            .chain([
                provider.config.name.trim(),
                provider.config.model.trim(),
                provider.config.provider_type.trim(),
            ])
            .map(normalize_identity_metadata_token)
            .filter(|token| token.len() >= 4)
            .any(|token| normalized_answer.contains(&token))
    })
}

fn candidate_answer_is_runtime_identity_label(state: &AppState, candidate_answer: &str) -> bool {
    let candidate = candidate_answer
        .trim()
        .trim_matches(|ch: char| ch.is_ascii_punctuation() || ch.is_ascii_whitespace());
    !candidate.is_empty() && candidate.eq_ignore_ascii_case(state.agent_runtime_identity_label())
}

fn normalize_identity_metadata_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

pub(super) fn structural_satisfaction_can_skip_verifier(
    route_result: &RouteResult,
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
    if workspace_project_summary_requires_model_verifier(route_result) {
        return false;
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

pub(super) fn workspace_project_summary_requires_model_verifier(
    route_result: &RouteResult,
) -> bool {
    if !route_result.output_contract_marker_is(crate::OutputSemanticKind::WorkspaceProjectSummary) {
        return false;
    }
    crate::contract_matrix::final_answer_shape_for_route(route_result)
        .is_some_and(|shape| shape.allows_model_language())
}

pub(super) fn finalizer_summary_can_skip_answer_verifier(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    let Some(shape) = crate::contract_matrix::final_answer_shape_for_route(route_result) else {
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
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    finalizer_summary_can_skip_answer_verifier(route_result, journal)
        && latest_non_control_step_is_missing_target(journal)
}

pub(super) fn finalizer_account_access_error_can_skip_missing_evidence_gap(
    route_result: &RouteResult,
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
    route_result: &RouteResult,
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
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Option<AnswerVerifierOut> {
    let task_contract = TaskContract::from_route_result(route_result);
    if task_contract.intent_kind.as_str() != "planner_execute"
        || task_contract.required_evidence_fields.is_empty()
    {
        return None;
    }
    let coverage = crate::task_journal::evidence_coverage_for_route(route_result, journal);
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
    route_result: &RouteResult,
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
    Some(gap)
}

fn config_guard_machine_payload_can_skip_answer_verifier(
    route_result: &RouteResult,
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

pub(super) fn missing_target_answer_is_grounded_in_latest_error(
    route: &RouteResult,
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
    route: &RouteResult,
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

pub(super) fn scalar_field_value_gap_is_grounded_in_structured_read(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
    gap: &AnswerVerifierOut,
) -> bool {
    if gap.missing_evidence_fields.len() != 1
        || gap.missing_evidence_fields.first().map(String::as_str) != Some("field_value")
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !route.output_contract_is_unclassified()
    {
        return false;
    }
    let Some(shape) = crate::contract_matrix::final_answer_shape_for_route(route) else {
        return false;
    };
    if shape.class() != crate::contract_matrix::FinalAnswerShapeClass::ScalarValue
        || !scalar_answer_is_strict_for_shape(shape, candidate_answer)
    {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step_can_supply_verifier_observation_for_route(route, step)
            && step_can_supply_strict_evidence_for_route(route, step)
            && step.output_excerpt.as_deref().is_some_and(|output| {
                structured_read_output_contains_scalar_answer(output, candidate_answer)
            })
    })
}

pub(super) fn structured_read_output_contains_scalar_answer(
    output: &str,
    candidate_answer: &str,
) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    structured_read_json_contains_scalar_answer(&value, candidate_answer)
}

pub(super) fn structured_read_json_contains_scalar_answer(
    value: &serde_json::Value,
    candidate_answer: &str,
) -> bool {
    let action = value
        .get("action")
        .or_else(|| value.pointer("/extra/action"))
        .and_then(|value| value.as_str())
        .map(str::trim);
    if !matches!(action, Some("read_range" | "read_text_range")) {
        return false;
    }
    [
        value.get("excerpt").and_then(|value| value.as_str()),
        value
            .pointer("/extra/excerpt")
            .and_then(|value| value.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(read_range_excerpt_without_line_prefixes)
    .filter_map(|body| serde_json::from_str::<serde_json::Value>(&body).ok())
    .any(|document| json_value_contains_scalar_answer(&document, candidate_answer.trim()))
}

pub(super) fn read_range_excerpt_without_line_prefixes(excerpt: &str) -> String {
    excerpt
        .lines()
        .map(strip_read_range_line_prefix)
        .collect::<Vec<_>>()
        .join("\n")
}

pub(crate) fn local_compound_listing_answer_verifier_gap(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> Option<AnswerVerifierOut> {
    let contract = route_result.effective_output_contract();
    if !contract.requires_content_evidence
        || contract.delivery_required
        || !route_result.output_contract_marker_is_any(&[
            crate::OutputSemanticKind::ExcerptKindJudgment,
            crate::OutputSemanticKind::ContentExcerptSummary,
            crate::OutputSemanticKind::ContentExcerptWithSummary,
            crate::OutputSemanticKind::DirectoryPurposeSummary,
        ])
    {
        return None;
    }
    let Some(_limit) = requested_listing_name_limit(route_result) else {
        return None;
    };
    let names = observed_inventory_names_for_contract(route_result, journal)?;
    if names.len() < 2 || !journal_has_content_excerpt_observation(journal) {
        return None;
    }
    let missing = names
        .iter()
        .filter(|name| !observed_name_is_mentioned(candidate_answer, name))
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return None;
    }
    Some(AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["candidates".to_string()],
        answer_incomplete_reason: format!(
            "answer omitted observed listing item(s): {}",
            missing.join(", ")
        ),
        should_retry: true,
        retry_instruction: "Use the already observed listing items and content excerpt to produce one combined final answer in the requested shape.".to_string(),
        confidence: 0.92,
    })
}

pub(super) fn latest_observed_inventory_names(
    journal: &crate::task_journal::TaskJournal,
) -> Option<Vec<String>> {
    latest_observed_directory_structure_names(journal).and_then(|names| {
        (!names.is_empty()).then_some(
            names
                .into_iter()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
                .collect::<Vec<_>>(),
        )
    })
}

pub(super) fn latest_observed_directory_structure_names(
    journal: &crate::task_journal::TaskJournal,
) -> Option<Vec<String>> {
    journal
        .step_results
        .iter()
        .rev()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .flat_map(structured_json_values_from_step_output)
        .find_map(|value| {
            if !value_is_directory_structure_observation(&value) {
                return None;
            }
            let mut names = BTreeSet::new();
            collect_observed_strict_list_items_from_value(&value, &mut names);
            let names = names.into_iter().collect::<Vec<_>>();
            (!names.is_empty()).then_some(names)
        })
}

pub(super) fn value_is_directory_structure_observation(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("inventory_dir" | "list_dir" | "tree_summary")
    ) || value
        .get("names")
        .and_then(|value| value.as_array())
        .is_some()
        || value
            .get("names_by_kind")
            .and_then(|value| value.as_object())
            .is_some()
        || value
            .get("entries")
            .and_then(|value| value.as_array())
            .is_some()
        || value
            .get("candidates")
            .and_then(|value| value.as_array())
            .is_some()
}

pub(super) fn observed_inventory_names_for_contract(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
) -> Option<Vec<String>> {
    let mut names = latest_observed_inventory_names(journal)?;
    if let Some(limit) = requested_listing_name_limit(route_result) {
        names.truncate(limit.min(names.len()));
    }
    Some(names)
}

pub(super) fn requested_listing_name_limit(route_result: &RouteResult) -> Option<usize> {
    route_result
        .output_contract
        .self_extension
        .list_selector
        .limit
        .and_then(|limit| usize::try_from(limit).ok())
        .filter(|limit| *limit > 0)
        .or_else(|| {
            contract_hint_selector_limit(&route_result.resolved_intent)
                .or_else(|| contract_hint_selector_limit(&route_result.route_reason))
                .and_then(|limit| usize::try_from(limit).ok())
                .filter(|limit| *limit > 0)
        })
}

pub(super) fn contract_hint_selector_limit(text: &str) -> Option<u64> {
    text.split(|ch: char| ch == '\n' || ch == ';' || ch == ',' || ch.is_whitespace())
        .filter_map(|part| part.split_once('='))
        .find_map(|(key, value)| {
            (key.trim() == "selector_limit")
                .then(|| value.trim().parse::<u64>().ok())
                .flatten()
        })
}

pub(super) fn observed_names_all_mentioned(candidate_answer: &str, names: &[String]) -> bool {
    names
        .iter()
        .all(|name| observed_name_is_mentioned(candidate_answer, name))
}

pub(super) fn observed_name_is_mentioned(candidate_answer: &str, name: &str) -> bool {
    let answer = candidate_answer.replace('\\', "/").to_ascii_lowercase();
    let normalized = name.replace('\\', "/").to_ascii_lowercase();
    !normalized.is_empty() && answer.contains(&normalized)
}

pub(super) fn journal_has_content_excerpt_observation(
    journal: &crate::task_journal::TaskJournal,
) -> bool {
    journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .filter_map(|step| step.output_excerpt.as_deref())
        .flat_map(structured_json_values_from_step_output)
        .any(|value| {
            matches!(
                value.get("action").and_then(|value| value.as_str()),
                Some("read_range" | "read_text_range")
            ) && value
                .get("excerpt")
                .or_else(|| value.get("content"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .is_some_and(|text| !text.is_empty())
        })
}

pub(super) fn observed_content_excerpt_path_names(
    journal: &crate::task_journal::TaskJournal,
) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for step in &journal.step_results {
        if !step_can_supply_verifier_observation(step) {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        collect_content_excerpt_path_names_from_structured_output(output, &mut names);
        collect_content_excerpt_path_names_from_truncated_json(output, &mut names);
    }
    names
}

pub(super) fn collect_content_excerpt_path_names_from_structured_output(
    output: &str,
    names: &mut BTreeSet<String>,
) {
    for value in structured_json_values_from_step_output(output) {
        if !value_is_read_content_observation(&value) {
            continue;
        }
        for path in json_path_string_values(&value) {
            collect_path_name_variants(&path, names);
        }
    }
}

pub(super) fn value_is_read_content_observation(value: &serde_json::Value) -> bool {
    matches!(
        value.get("action").and_then(|value| value.as_str()),
        Some("read_range" | "read_text_range")
    ) && value
        .get("excerpt")
        .or_else(|| value.get("content"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .is_some_and(|text| !text.is_empty())
}

pub(super) fn json_path_string_values(value: &serde_json::Value) -> Vec<String> {
    [
        value.get("path").and_then(|value| value.as_str()),
        value.get("resolved_path").and_then(|value| value.as_str()),
        value
            .pointer("/extra/path")
            .and_then(|value| value.as_str()),
        value
            .pointer("/extra/resolved_path")
            .and_then(|value| value.as_str()),
    ]
    .into_iter()
    .flatten()
    .map(str::trim)
    .filter(|value| !value.is_empty())
    .map(ToString::to_string)
    .collect()
}

pub(super) fn collect_content_excerpt_path_names_from_truncated_json(
    output: &str,
    names: &mut BTreeSet<String>,
) {
    if !output_has_read_content_machine_tokens(output) {
        return;
    }
    for key in ["path", "resolved_path"] {
        for value in json_string_field_values_from_text(output, key) {
            collect_path_name_variants(&value, names);
        }
    }
}

pub(super) fn output_has_read_content_machine_tokens(output: &str) -> bool {
    (output.contains("\"read_range\"") || output.contains("\"read_text_range\""))
        && (output.contains("\"excerpt\"") || output.contains("\"content\""))
}

pub(super) fn json_string_field_values_from_text(text: &str, field: &str) -> Vec<String> {
    let needle = format!("\"{field}\":\"");
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find(&needle) {
        let after = &rest[idx + needle.len()..];
        let mut value = String::new();
        let mut escaped = false;
        for ch in after.chars() {
            if escaped {
                value.push(ch);
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                break;
            }
            value.push(ch);
        }
        if !value.trim().is_empty() {
            values.push(value);
        }
        rest = after;
    }
    values
}

pub(super) fn collect_path_name_variants(path: &str, names: &mut BTreeSet<String>) {
    for variant in path_variants(path) {
        if !variant.trim().is_empty() {
            names.insert(variant);
        }
    }
}

pub(super) fn structured_json_values_from_step_output(output: &str) -> Vec<serde_json::Value> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return Vec::new();
    };
    let mut values = vec![value.clone()];
    if let Some(extra) = value.get("extra") {
        values.push(extra.clone());
    }
    values
}

pub(super) fn task_contract_prompt_block(task_contract: &TaskContract) -> String {
    task_contract.compact_prompt_line()
}

pub(super) fn output_contract_prompt_block(route_result: &RouteResult) -> String {
    let contract_matrix_trace = verifier_contract_matrix_prompt_trace(route_result);
    serde_json::to_string_pretty(&json!({
        "response_shape": route_result.output_contract.response_shape.as_str(),
        "requires_content_evidence": route_result.output_contract.requires_content_evidence,
        "delivery_required": route_result.output_contract.delivery_required,
        "locator_kind": route_result.output_contract.locator_kind.as_str(),
        "delivery_intent": route_result.output_contract.delivery_intent.as_str(),
        "contract_marker": route_result.effective_output_contract_semantic_kind().as_str(),
        "locator_hint": route_result.output_contract.locator_hint,
        "contract_matrix": contract_matrix_trace,
    }))
    .unwrap_or_else(|_| "{}".to_string())
}

fn verifier_contract_matrix_prompt_trace(route_result: &RouteResult) -> Option<serde_json::Value> {
    let mut trace = crate::contract_matrix::trace_snapshot_for_route(route_result)?;
    if let Some(obj) = trace.as_object_mut() {
        obj.remove("trace_policy");
        obj.remove("observation_extractors");
        obj.remove("observation_sources");
        obj.remove("artifact_kind");
        obj.remove("channel_visibility");
        obj.insert(
            "compact_line".to_string(),
            serde_json::Value::String(
                crate::contract_matrix::compact_prompt_line_for_route(route_result)
                    .unwrap_or_default(),
            ),
        );
    }
    Some(trace)
}

pub(super) fn provider_safe_excerpt_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("fnv64:{hash:016x}")
}

pub(super) fn provider_safe_numeric_evidence(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> Vec<serde_json::Value> {
    let Some(output) = step.output_excerpt.as_deref() else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    collect_provider_safe_numeric_evidence("", &value, &mut items);
    items.truncate(32);
    items
}

pub(super) fn collect_provider_safe_numeric_evidence(
    prefix: &str,
    value: &serde_json::Value,
    out: &mut Vec<serde_json::Value>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                let field = if prefix.is_empty() {
                    key.to_string()
                } else {
                    format!("{prefix}.{key}")
                };
                if provider_safe_numeric_evidence_leaf(key)
                    && matches!(
                        child,
                        serde_json::Value::Number(_) | serde_json::Value::Bool(_)
                    )
                {
                    out.push(json!({
                        "field": field,
                        "value": child,
                    }));
                }
                collect_provider_safe_numeric_evidence(&field, child, out);
            }
        }
        serde_json::Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                let field = format!("{prefix}[{index}]");
                collect_provider_safe_numeric_evidence(&field, child, out);
            }
        }
        _ => {}
    }
}

pub(super) fn provider_safe_numeric_evidence_leaf(key: &str) -> bool {
    matches!(
        key,
        "count"
            | "dirs"
            | "exists"
            | "files"
            | "hidden"
            | "size_bytes"
            | "total"
            | "total_size_bytes"
    )
}

pub(super) fn provider_safe_step_evidence(
    step: &crate::task_journal::TaskJournalStepTrace,
) -> serde_json::Value {
    json!({
        "step_id": step.step_id,
        "skill": step.skill,
        "status": step.status.as_str(),
        "observed_evidence": crate::task_journal::observed_evidence_for_step_trace(step),
        "key_numeric_evidence": provider_safe_numeric_evidence(step),
        "output_excerpt_present": step.output_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "output_excerpt_hash": step.output_excerpt.as_deref().map(provider_safe_excerpt_hash),
        "error_excerpt_present": step.error_excerpt.as_deref().is_some_and(|value| !value.trim().is_empty()),
        "error_excerpt_hash": step.error_excerpt.as_deref().map(provider_safe_excerpt_hash),
    })
}

pub(super) fn execution_evidence_prompt_block(
    journal: &crate::task_journal::TaskJournal,
) -> String {
    let mut steps = journal
        .step_results
        .iter()
        .filter(|step| step_can_supply_verifier_prompt_observation(step))
        .rev()
        .take(MAX_VERIFIER_STEPS)
        .map(provider_safe_step_evidence)
        .collect::<Vec<_>>();
    steps.reverse();
    serde_json::to_string_pretty(&steps).unwrap_or_else(|_| "[]".to_string())
}

pub(super) fn current_context_prompt_block(journal: &crate::task_journal::TaskJournal) -> String {
    const MAX_CHARS: usize = 12_000;
    let Some(summary) = journal.context_bundle_summary.as_deref() else {
        return "<none>".to_string();
    };
    let trimmed = summary.trim();
    if trimmed.is_empty() {
        return "<none>".to_string();
    }
    if trimmed.chars().count() <= MAX_CHARS {
        return trimmed.to_string();
    }
    trimmed.chars().take(MAX_CHARS).collect()
}
