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

pub(crate) fn structurally_satisfies_answer_contract(
    route_result: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if route_requires_single_file_delivery(route_result)
        && candidate_answer_has_grounded_existing_file_token(journal, candidate_answer)
    {
        return true;
    }
    if raw_command_answer_is_exact_single_successful_observation(journal, candidate_answer) {
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

fn route_requires_single_file_delivery(route: &RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    ) || matches!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    ) || (route.wants_file_delivery
        && !matches!(
            route.output_contract.delivery_intent,
            crate::OutputDeliveryIntent::DirectoryBatchFiles
        ))
}

fn candidate_answer_has_grounded_existing_file_token(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let Some((_kind, raw_path)) =
        crate::finalize::parse_delivery_file_token(candidate_answer.trim())
    else {
        return false;
    };
    let token_path = std::path::Path::new(raw_path.trim());
    let Ok(canonical_token_path) = token_path.canonicalize() else {
        return false;
    };
    file_token_path_is_grounded_in_observations(journal, &canonical_token_path)
}

fn file_token_path_is_grounded_in_observations(
    journal: &crate::task_journal::TaskJournal,
    canonical_token_path: &std::path::Path,
) -> bool {
    let current_dir = std::env::current_dir().ok();
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && step.output_excerpt.as_deref().is_some_and(|output| {
                observed_output_contains_path(output, canonical_token_path, current_dir.as_deref())
            })
    })
}

fn observed_output_contains_path(
    output: &str,
    canonical_token_path: &std::path::Path,
    current_dir: Option<&std::path::Path>,
) -> bool {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        return json_value_contains_path(&value, canonical_token_path, current_dir);
    }
    candidate_path_matches(output.trim(), canonical_token_path, current_dir)
}

fn json_value_contains_path(
    value: &serde_json::Value,
    canonical_token_path: &std::path::Path,
    current_dir: Option<&std::path::Path>,
) -> bool {
    match value {
        serde_json::Value::String(candidate) => {
            candidate_path_matches(candidate, canonical_token_path, current_dir)
        }
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_value_contains_path(item, canonical_token_path, current_dir)),
        serde_json::Value::Object(map) => {
            if resolved_dir_names_contain_path(map, canonical_token_path) {
                return true;
            }
            map.values()
                .any(|item| json_value_contains_path(item, canonical_token_path, current_dir))
        }
        _ => false,
    }
}

fn resolved_dir_names_contain_path(
    map: &serde_json::Map<String, serde_json::Value>,
    canonical_token_path: &std::path::Path,
) -> bool {
    let Some(resolved_dir) = map
        .get("resolved_path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(std::path::Path::new)
    else {
        return false;
    };
    let Some(names) = map.get("names").and_then(|value| value.as_array()) else {
        return false;
    };
    names.iter().filter_map(|value| value.as_str()).any(|name| {
        let candidate = resolved_dir.join(name.trim());
        candidate
            .canonicalize()
            .is_ok_and(|path| path == canonical_token_path)
    })
}

fn candidate_path_matches(
    candidate: &str,
    canonical_token_path: &std::path::Path,
    current_dir: Option<&std::path::Path>,
) -> bool {
    let candidate = candidate.trim();
    if candidate.is_empty() {
        return false;
    }
    let candidate_path = std::path::Path::new(candidate);
    if candidate_path
        .canonicalize()
        .is_ok_and(|path| path == canonical_token_path)
    {
        return true;
    }
    current_dir.is_some_and(|dir| {
        dir.join(candidate_path)
            .canonicalize()
            .is_ok_and(|path| path == canonical_token_path)
    })
}

fn raw_command_answer_is_exact_single_successful_observation(
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    let mut external_steps = journal
        .step_results
        .iter()
        .filter(|step| is_external_execution_step(step));
    let Some(step) = external_steps.next() else {
        return false;
    };
    if external_steps.next().is_some() {
        return false;
    }
    if step.status != crate::executor::StepExecutionStatus::Ok || step.skill != "run_cmd" {
        return false;
    }
    let Some(output) = step.output_excerpt.as_deref().map(str::trim) else {
        return false;
    };
    !output.is_empty() && !output.ends_with("...(truncated)") && output == candidate_answer.trim()
}

fn is_external_execution_step(step: &crate::task_journal::TaskJournalStepTrace) -> bool {
    !matches!(
        step.skill.as_str(),
        "synthesize_answer" | "respond" | "think"
    )
}

fn existence_with_path_answer_is_grounded_in_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ExistenceWithPath {
        return false;
    }
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && step.output_excerpt.as_deref().is_some_and(|output| {
                path_batch_facts_contain_answer_path(output, candidate_answer)
            })
    })
}

fn path_batch_facts_contain_answer_path(output: &str, candidate_answer: &str) -> bool {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) else {
        return false;
    };
    let has_path_batch_shape = value.get("action").and_then(|item| item.as_str())
        == Some("path_batch_facts")
        || value
            .get("facts")
            .and_then(|item| item.as_array())
            .is_some();
    if !has_path_batch_shape {
        return false;
    }
    value
        .get("facts")
        .and_then(|item| item.as_array())
        .is_some_and(|facts| {
            facts.iter().any(|fact| {
                fact.get("exists").and_then(|item| item.as_bool()).is_some()
                    && path_fact_candidates(fact)
                        .into_iter()
                        .any(|path| candidate_answer.contains(path.as_str()))
            })
        })
}

fn path_fact_candidates(fact: &serde_json::Value) -> Vec<String> {
    let mut paths = Vec::new();
    let mut push_path = |value: Option<&serde_json::Value>| {
        if let Some(path) = value
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            paths.push(path.to_string());
        }
    };
    push_path(fact.get("resolved_path"));
    push_path(fact.get("path"));
    if let Some(inner) = fact.get("fact").and_then(|item| item.as_object()) {
        push_path(inner.get("resolved_path"));
        push_path(inner.get("path"));
    }
    paths.sort();
    paths.dedup();
    paths
}

fn scalar_answer_is_grounded_in_successful_observation(
    route: &RouteResult,
    journal: &crate::task_journal::TaskJournal,
    candidate_answer: &str,
) -> bool {
    if !matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    ) {
        return false;
    }
    let candidate_answer = candidate_answer.trim();
    if candidate_answer.is_empty() || candidate_answer.lines().count() > 1 {
        return false;
    }
    journal.step_results.iter().any(|step| {
        step.status == crate::executor::StepExecutionStatus::Ok
            && step.output_excerpt.as_deref().is_some_and(|output| {
                observed_output_contains_scalar_answer(output, candidate_answer)
            })
    })
}

fn observed_output_contains_scalar_answer(output: &str, candidate_answer: &str) -> bool {
    let output = output.trim();
    if output == candidate_answer {
        return true;
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output) {
        return json_value_contains_scalar_answer(&value, candidate_answer);
    }
    output
        .lines()
        .map(str::trim)
        .any(|line| line == candidate_answer)
}

fn json_value_contains_scalar_answer(value: &serde_json::Value, candidate_answer: &str) -> bool {
    match value {
        serde_json::Value::String(value) => value.trim() == candidate_answer,
        serde_json::Value::Number(value) => value.to_string() == candidate_answer,
        serde_json::Value::Bool(value) => value.to_string() == candidate_answer,
        serde_json::Value::Array(items) => items
            .iter()
            .any(|item| json_value_contains_scalar_answer(item, candidate_answer)),
        serde_json::Value::Object(map) => map
            .values()
            .any(|item| json_value_contains_scalar_answer(item, candidate_answer)),
        serde_json::Value::Null => false,
    }
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
    if structurally_satisfies_answer_contract(route_result, journal, candidate_answer) {
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

    use super::{should_verify_answer, structurally_satisfies_answer_contract, AnswerVerifierOut};

    fn route_with_mode(ask_mode: crate::AskMode) -> crate::RouteResult {
        crate::RouteResult {
            ask_mode,
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
        let route = route_with_mode(crate::AskMode::direct_answer());
        let journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
        assert!(!should_verify_answer(&route, &journal, "hi"));
    }

    #[test]
    fn clarify_final_status_skips_answer_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_chat_wrapped());
        route.output_contract.requires_content_evidence = true;
        let mut journal = crate::task_journal::TaskJournal::for_task("task-1", "ask", "hello");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Clarify);

        assert!(!should_verify_answer(
            &route,
            &journal,
            "please provide the missing path"
        ));
    }

    #[test]
    fn grounded_file_token_satisfies_file_delivery_contract_before_llm_verifier() {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-answer-verifier-file-token-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create temp root");
        let file = root.join("release_checklist.md");
        std::fs::write(&file, "ok").expect("write temp file");

        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.wants_file_delivery = true;
        route.output_contract.delivery_required = true;
        route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
        route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-file-token", "ask", "send that file");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "path_batch_facts",
                        "facts": [{
                            "path": file.display().to_string(),
                            "fact": {
                                "kind": "file",
                                "resolved_path": file.display().to_string()
                            }
                        }]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            &format!("FILE:{}", file.display())
        ));

        let _ = std::fs::remove_file(&file);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scalar_answer_grounded_in_plain_observation_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-scalar", "ask", "where am I");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("/home/guagua/rustclaw\n".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "/home/guagua/rustclaw"
        ));
    }

    #[test]
    fn scalar_answer_grounded_in_json_observation_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-json-scalar", "ask", "count them");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(json!({"count": 3, "items": ["a", "b", "c"]}).to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route, &journal, "3"
        ));
    }

    #[test]
    fn existence_with_path_answer_grounded_by_existing_path_fact_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-exists", "ask", "check path");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "system_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "path_batch_facts",
                        "facts": [{
                            "exists": true,
                            "path": "README.md",
                            "fact": {
                                "kind": "file",
                                "resolved_path": "/repo/README.md"
                            }
                        }]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "有，路径：/repo/README.md"
        ));
    }

    #[test]
    fn existence_with_path_answer_grounded_by_missing_path_fact_skips_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-missing", "ask", "check path");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "system_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some(
                    json!({
                        "action": "path_batch_facts",
                        "facts": [{
                            "exists": false,
                            "path": "missing.txt",
                            "error": "not found"
                        }]
                    })
                    .to_string(),
                ),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "未找到 `missing.txt`，请确认路径后再继续。"
        ));
    }

    #[test]
    fn exact_single_run_cmd_output_skips_llm_verifier_without_scalar_contract() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-run-cmd", "ask", "run it");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "run_cmd".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("line 1\nline 2\n".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_2".to_string(),
                skill: "synthesize_answer".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("line 1\nline 2".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(structurally_satisfies_answer_contract(
            &route,
            &journal,
            "line 1\nline 2"
        ));
    }

    #[test]
    fn exact_run_cmd_output_skip_requires_single_external_step() {
        let route = route_with_mode(crate::AskMode::planner_execute_plain());
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-two-commands", "ask", "run both");
        for (idx, output) in ["first", "second"].into_iter().enumerate() {
            journal
                .step_results
                .push(crate::task_journal::TaskJournalStepTrace {
                    step_id: format!("step_{}", idx + 1),
                    skill: "run_cmd".to_string(),
                    status: crate::executor::StepExecutionStatus::Ok,
                    output_excerpt: Some(output.to_string()),
                    error_excerpt: None,
                    started_at: 0,
                    finished_at: 0,
                });
        }

        assert!(!structurally_satisfies_answer_contract(
            &route, &journal, "second"
        ));
    }

    #[test]
    fn free_shape_non_command_plain_observation_still_uses_llm_verifier() {
        let mut route = route_with_mode(crate::AskMode::planner_execute_plain());
        route.output_contract.response_shape = crate::OutputResponseShape::Free;
        route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-free", "ask", "summarize output");
        journal
            .step_results
            .push(crate::task_journal::TaskJournalStepTrace {
                step_id: "step_1".to_string(),
                skill: "fs_basic".to_string(),
                status: crate::executor::StepExecutionStatus::Ok,
                output_excerpt: Some("ok".to_string()),
                error_excerpt: None,
                started_at: 0,
                finished_at: 0,
            });

        assert!(!structurally_satisfies_answer_contract(
            &route, &journal, "ok"
        ));
    }
}
