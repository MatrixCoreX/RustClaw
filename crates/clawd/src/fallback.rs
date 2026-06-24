//! §7.2 Clarify fallback source 矩阵
//!
//! 历史问题：4+ 类不同失败路径都收敛到同一句 `clawd.msg.clarify_question_fallback`
//! ("我需要确认一下：你这条消息是针对哪件事情？...")，根因被压扁、无法观测。
//!
//! 改造目标：
//! 1. 用 `ClarifyFallbackSource` enum 显式区分每种触发来源；
//! 2. 文案矩阵化：每个 source 一条 i18n key，告诉用户"我在哪卡住、你能怎么帮我"；
//! 3. 通过 `tracing::info!` 事件 `clarify_fallback_emitted` 上报 source 与 task_id，
//!    后续 `inspect_task.sh` 与未来 metric 都按 `fallback_source` label 聚合；
//! 4. 比对端（finalize/task / memory / routing_context 用以跳过历史 fallback turn）
//!    走 [`is_known_clarify_fallback_text`] 集合判定，不再依赖单条字符串相等。
//!
//! 兼容性：旧 key `clawd.msg.clarify_question_fallback` 在
//! [`all_clarify_fallback_texts`] 集合里保留，确保历史 DB 里的旧 fallback 文案
//! 仍能被识别为 placeholder 跳过；新写入一律走新 source。

use std::collections::HashMap;

use crate::AppState;
use serde::Deserialize;
use serde_json::json;

pub(crate) const USER_RESPONSE_COMPOSER_PROMPT_LOGICAL_PATH: &str =
    "prompts/user_response_composer_prompt.md";
const USER_RESPONSE_CONTRACT_VALIDATOR_PROMPT_LOGICAL_PATH: &str =
    "prompts/user_response_contract_validator_prompt.md";

/// 用户可见回复的结构化意图类型。
///
/// 代码负责填事实与边界；具体怎么对用户说，后续交给 LLM composer。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserResponseKind {
    Clarify,
    PolicyBlock,
    ToolFailure,
    SchemaInvalid,
    FinalAnswer,
}

impl UserResponseKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Clarify => "clarify",
            Self::PolicyBlock => "policy_block",
            Self::ToolFailure => "tool_failure",
            Self::SchemaInvalid => "schema_invalid",
            Self::FinalAnswer => "final_answer",
        }
    }
}

/// LLM 回复/澄清 composer 的最小 contract。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UserResponseContract {
    pub(crate) kind: UserResponseKind,
    pub(crate) reason_code: String,
    pub(crate) missing_slots: Vec<String>,
    pub(crate) observed_facts: Vec<String>,
    pub(crate) policy_boundary: Vec<String>,
    pub(crate) original_user_request: String,
    pub(crate) resolved_user_intent: String,
    pub(crate) response_shape: String,
    pub(crate) language_hint: String,
}

#[derive(Debug, Clone, Deserialize)]
struct UserResponseContractValidationOut {
    #[serde(default)]
    satisfies_contract: bool,
    #[serde(default)]
    false_claims: bool,
    #[serde(default)]
    asks_for_missing_target: bool,
    #[serde(default)]
    mentions_internal_details: bool,
    #[serde(default)]
    confidence: f64,
    #[serde(default)]
    reason: String,
}

impl Default for UserResponseKind {
    fn default() -> Self {
        Self::FinalAnswer
    }
}

impl UserResponseContract {
    pub(crate) fn clarify_from_fallback_source(
        source: ClarifyFallbackSource,
        original_user_request: &str,
        resolver_reason: &str,
        candidate_context: Option<&str>,
        language_hint: &str,
    ) -> Self {
        let mut observed_facts = Vec::new();
        let reason = resolver_reason.trim();
        if !reason.is_empty() {
            observed_facts.push(format!("resolver_reason: {reason}"));
        }
        if let Some(context) = candidate_context
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            observed_facts.push(format!("candidate_context: {context}"));
        }
        let mut missing_slots = Vec::new();
        if let Some(clarify_case) = clarify_case_from_candidate_context(candidate_context) {
            missing_slots.push(clarify_case);
        }
        missing_slots.push(source.as_metric_label().to_string());
        let resolved_user_intent =
            field_value_from_candidate_context(candidate_context, "resolved_user_intent")
                .unwrap_or_default();
        let mut policy_boundary = vec![
            "expose_internal_details=false".to_string(),
            "clarification_style=one_concise_situation_specific_question".to_string(),
        ];
        if missing_slots_have_specific_target_slot(&missing_slots)
            && !resolved_user_intent.trim().is_empty()
        {
            policy_boundary.push("known_operation_from_resolved_user_intent=true".to_string());
            policy_boundary.push("ask_only_for=target_path_scope_locator".to_string());
            policy_boundary.push("ask_action_again=false".to_string());
        }
        Self {
            kind: UserResponseKind::Clarify,
            reason_code: source.as_metric_label().to_string(),
            missing_slots,
            observed_facts,
            policy_boundary,
            original_user_request: original_user_request.trim().to_string(),
            resolved_user_intent,
            response_shape: "one_short_clarification".to_string(),
            language_hint: language_hint.trim().to_string(),
        }
    }

    pub(crate) fn verify_rejected(
        original_user_request: &str,
        resolved_user_intent: &str,
        response_shape: &str,
        semantic_kind: &str,
        verifier_reason_code: &str,
        verifier_reason: &str,
        language_hint: &str,
    ) -> Self {
        let mut observed_facts = Vec::new();
        let reason_code = verifier_reason_code.trim();
        if !reason_code.is_empty() {
            observed_facts.push(format!("verifier_reason_code: {reason_code}"));
        }
        let reason = verifier_reason.trim();
        if !reason.is_empty() {
            observed_facts.push(format!("verifier_reason: {reason}"));
        }
        if !response_shape.trim().is_empty() {
            observed_facts.push(format!("expected_response_shape: {response_shape}"));
        }
        if !semantic_kind.trim().is_empty() {
            observed_facts.push(format!("expected_semantic_kind: {semantic_kind}"));
        }
        Self {
            kind: UserResponseKind::SchemaInvalid,
            reason_code: ClarifyFallbackSource::VerifyRejected
                .as_metric_label()
                .to_string(),
            missing_slots: vec!["valid_final_answer_matching_user_requested_shape".to_string()],
            observed_facts,
            policy_boundary: vec![
                "expose_internal_details=false".to_string(),
                "delivery_constraint_policy=minimal_if_reshape_unsafe".to_string(),
                "exact_shape_ack_policy=user_language_without_internal_details".to_string(),
            ],
            original_user_request: original_user_request.trim().to_string(),
            resolved_user_intent: resolved_user_intent.trim().to_string(),
            response_shape: "one_short_clarification".to_string(),
            language_hint: language_hint.trim().to_string(),
        }
    }

    pub(crate) fn tool_failure(
        reason_code: &str,
        original_user_request: &str,
        resolved_user_intent: &str,
        observed_facts: Vec<String>,
        policy_boundary: Vec<String>,
        response_shape: &str,
        language_hint: &str,
    ) -> Self {
        Self {
            kind: UserResponseKind::ToolFailure,
            reason_code: reason_code.trim().to_string(),
            missing_slots: Vec::new(),
            observed_facts,
            policy_boundary,
            original_user_request: original_user_request.trim().to_string(),
            resolved_user_intent: resolved_user_intent.trim().to_string(),
            response_shape: response_shape.trim().to_string(),
            language_hint: language_hint.trim().to_string(),
        }
    }

    pub(crate) fn policy_block(
        reason_code: &str,
        original_user_request: &str,
        resolved_user_intent: &str,
        observed_facts: Vec<String>,
        policy_boundary: Vec<String>,
        language_hint: &str,
    ) -> Self {
        Self {
            kind: UserResponseKind::PolicyBlock,
            reason_code: reason_code.trim().to_string(),
            missing_slots: Vec::new(),
            observed_facts,
            policy_boundary,
            original_user_request: original_user_request.trim().to_string(),
            resolved_user_intent: resolved_user_intent.trim().to_string(),
            response_shape: "brief_failure_with_next_step".to_string(),
            language_hint: language_hint.trim().to_string(),
        }
    }

    pub(crate) fn verifier_gate(
        reason_code: &str,
        original_user_request: &str,
        resolved_user_intent: &str,
        missing_slots: Vec<String>,
        observed_facts: Vec<String>,
        response_shape: &str,
        language_hint: &str,
    ) -> Self {
        let kind = if missing_slots.is_empty() {
            UserResponseKind::PolicyBlock
        } else {
            UserResponseKind::Clarify
        };
        Self {
            kind,
            reason_code: reason_code.trim().to_string(),
            missing_slots,
            observed_facts,
            policy_boundary: vec![
                "expose_internal_details=false".to_string(),
                "blocked_action_execution_claim_allowed=false".to_string(),
                "confirmation_question_policy=one_concise_when_required".to_string(),
                "clarification_policy=smallest_missing_detail_only".to_string(),
            ],
            original_user_request: original_user_request.trim().to_string(),
            resolved_user_intent: resolved_user_intent.trim().to_string(),
            response_shape: response_shape.trim().to_string(),
            language_hint: language_hint.trim().to_string(),
        }
    }

    pub(crate) fn to_prompt_context_block(&self) -> String {
        let value = json!({
            "kind": self.kind.as_str(),
            "reason_code": &self.reason_code,
            "missing_slots": &self.missing_slots,
            "observed_facts": &self.observed_facts,
            "policy_boundary": &self.policy_boundary,
            "original_user_request": &self.original_user_request,
            "resolved_user_intent": &self.resolved_user_intent,
            "response_shape": &self.response_shape,
            "language_hint": &self.language_hint,
        });
        let body = serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string());
        format!("### USER_RESPONSE_CONTRACT\n{body}")
    }
}

fn clarify_case_from_candidate_context(candidate_context: Option<&str>) -> Option<String> {
    let value = field_value_from_candidate_context(candidate_context, "clarify_case")?;
    (!value.is_empty()
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-')))
    .then_some(value)
}

fn missing_slots_have_specific_target_slot(slots: &[String]) -> bool {
    slots.iter().any(|slot| {
        matches!(
            slot.as_str(),
            "missing_search_locator"
                | "missing_delivery_locator"
                | "missing_read_target"
                | "missing_file_locator"
                | "missing_directory_locator"
                | "missing_count_target"
                | "missing_service_target"
                | "missing_target"
        )
    })
}

fn field_value_from_candidate_context(
    candidate_context: Option<&str>,
    field: &str,
) -> Option<String> {
    let context = candidate_context?.trim();
    let prefix = format!("{field}:");
    for line in context.lines() {
        let Some(value) = line.trim().strip_prefix(&prefix) else {
            continue;
        };
        let value = value.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
pub(crate) fn missing_file_delivery_default_payload(locator_hint: Option<&str>) -> String {
    let locator = locator_hint
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let message_key = if locator.is_some() {
        "clawd.msg.delivery.file_not_found_path_next_step"
    } else {
        "clawd.msg.delivery.file_not_found_next_step"
    };
    let mut payload = json!({
        "message_key": message_key,
        "reason_code": "missing_file_delivery_not_found",
        "delivery_required": true,
        "file_found": false
    });
    if let Some(locator) = locator {
        payload["missing_path"] = json!(locator);
    }
    payload.to_string()
}

fn missing_file_delivery_default_machine_text(locator_hint: Option<&str>) -> String {
    let locator = locator_hint
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(locator) = locator {
        return format!("delivery_required=true\nfile_found=false\nmissing_path=`{locator}`");
    }
    "delivery_required=true\nfile_found=false".to_string()
}

pub(crate) fn missing_file_delivery_default_text(
    state: &AppState,
    locator_hint: Option<&str>,
    language_hint: &str,
) -> String {
    let locator = locator_hint
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let key = if locator.is_some() {
        "clawd.msg.delivery.file_not_found_path_next_step"
    } else {
        "clawd.msg.delivery.file_not_found_next_step"
    };
    let default_text = missing_file_delivery_default_machine_text(locator);
    let vars = locator
        .map(|missing_path| vec![("missing_path", missing_path)])
        .unwrap_or_default();
    crate::bilingual_t_with_default_vars(
        state,
        key,
        &default_text,
        &default_text,
        fallback_prefers_english_for_language_hint(state, language_hint),
        &vars,
    )
}

pub(crate) async fn compose_missing_file_delivery_response(
    state: &AppState,
    task: &crate::ClaimedTask,
    original_user_request: &str,
    resolved_user_intent: &str,
    locator_hint: Option<&str>,
    language_hint: &str,
) -> String {
    let locator = locator_hint
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let mut observed_facts = vec![
        "delivery_required: true".to_string(),
        "file_found: false".to_string(),
    ];
    if let Some(locator) = locator {
        observed_facts.push(format!("missing_path: {locator}"));
    }
    let contract = UserResponseContract::tool_failure(
        "missing_file_delivery_not_found",
        original_user_request,
        resolved_user_intent,
        observed_facts,
        vec![
            "file_sent_claim_allowed=false".to_string(),
            "include_missing_path_when_observed=true".to_string(),
            "recovery_next_step_policy=one_concise_no_internal_traces".to_string(),
        ],
        "brief_failure_with_next_step",
        language_hint,
    );
    let default_text = missing_file_delivery_default_text(state, locator, language_hint);
    compose_user_response_from_contract_with_default(
        state,
        task,
        &contract,
        ClarifyFallbackSource::ExecutionFailedPartial,
        &default_text,
    )
    .await
}

pub(crate) async fn compose_user_response_from_contract(
    state: &AppState,
    task: &crate::ClaimedTask,
    contract: &UserResponseContract,
    fallback_source: ClarifyFallbackSource,
) -> String {
    compose_user_response_from_contract_impl(state, task, contract, fallback_source, None).await
}

pub(crate) async fn compose_user_response_from_contract_with_default(
    state: &AppState,
    task: &crate::ClaimedTask,
    contract: &UserResponseContract,
    fallback_source: ClarifyFallbackSource,
    default_text: &str,
) -> String {
    compose_user_response_from_contract_impl(
        state,
        task,
        contract,
        fallback_source,
        Some(default_text),
    )
    .await
}

async fn compose_user_response_from_contract_impl(
    state: &AppState,
    task: &crate::ClaimedTask,
    contract: &UserResponseContract,
    fallback_source: ClarifyFallbackSource,
    default_text: Option<&str>,
) -> String {
    if let Some(text) = structured_clarify_default_text(state, contract) {
        return text;
    }
    let default_text = default_text
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (prompt_template, prompt_source) =
        match crate::bootstrap::load_required_prompt_template_for_state(
            state,
            USER_RESPONSE_COMPOSER_PROMPT_LOGICAL_PATH,
        ) {
            Ok(resolved) => resolved,
            Err(err) => {
                tracing::warn!(
                    "user_response_composer prompt load failed, fallback default: task_id={} err={}",
                    task.task_id,
                    err
                );
                if let Some(default_text) = default_text {
                    return default_text.to_string();
                }
                return render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    ClarifyFallbackSource::LlmUnavailable,
                    Some(&err.to_string()),
                    &contract.language_hint,
                );
            }
        };
    let prompt = crate::render_prompt_template(
        &prompt_template,
        &[
            (
                "__USER_RESPONSE_CONTRACT__",
                &contract.to_prompt_context_block(),
            ),
            (
                "__CONFIG_RESPONSE_LANGUAGE__",
                &state.policy.command_intent.default_locale,
            ),
        ],
    );
    crate::log_prompt_render(
        state,
        &task.task_id,
        "user_response_composer_prompt",
        &prompt_source,
        None,
    );
    match crate::llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(reply) => {
            let trimmed = reply.trim();
            if trimmed.is_empty() {
                if let Some(default_text) = default_text {
                    return default_text.to_string();
                }
                render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    ClarifyFallbackSource::EmptyResponse,
                    None,
                    &contract.language_hint,
                )
            } else if !user_response_contract_local_shape_satisfied(contract, trimmed) {
                tracing::warn!(
                    task_id = %task.task_id,
                    response_shape = %contract.response_shape,
                    fallback_source = fallback_source.as_metric_label(),
                    reply_chars = trimmed.chars().count(),
                    reply_lines = trimmed.lines().filter(|line| !line.trim().is_empty()).count(),
                    "user_response_composer_invalid_shape"
                );
                if let Some(default_text) = default_text {
                    return default_text.to_string();
                }
                render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    fallback_source,
                    None,
                    &contract.language_hint,
                )
            } else if !user_response_contract_llm_validated(state, task, contract, trimmed).await {
                tracing::warn!(
                    task_id = %task.task_id,
                    response_shape = %contract.response_shape,
                    fallback_source = fallback_source.as_metric_label(),
                    reply_chars = trimmed.chars().count(),
                    reply_lines = trimmed.lines().filter(|line| !line.trim().is_empty()).count(),
                    "user_response_composer_contract_validator_rejected"
                );
                if let Some(default_text) = default_text {
                    return default_text.to_string();
                }
                render_clarify_fallback_with_language_hint(
                    state,
                    &task.task_id,
                    fallback_source,
                    None,
                    &contract.language_hint,
                )
            } else {
                trimmed.to_string()
            }
        }
        Err(err) => {
            tracing::warn!(
                "user_response_composer llm failed, fallback default: task_id={} err={}",
                task.task_id,
                err
            );
            if let Some(default_text) = default_text {
                return default_text.to_string();
            }
            let hint = format!("source={},err={err}", fallback_source.as_metric_label());
            render_clarify_fallback_with_language_hint(
                state,
                &task.task_id,
                ClarifyFallbackSource::LlmUnavailable,
                Some(&hint),
                &contract.language_hint,
            )
        }
    }
}

fn structured_clarify_default_text(
    state: &AppState,
    contract: &UserResponseContract,
) -> Option<String> {
    if !matches!(contract.kind, UserResponseKind::Clarify) {
        return None;
    }
    let key = if contract
        .missing_slots
        .iter()
        .any(|slot| slot == "missing_read_target")
    {
        "clawd.msg.clarify_missing_read_target"
    } else if contract
        .missing_slots
        .iter()
        .any(|slot| slot == "missing_search_locator")
    {
        "clawd.msg.clarify_missing_search_locator"
    } else if contract.missing_slots.iter().any(|slot| {
        matches!(
            slot.as_str(),
            "missing_file_locator" | "missing_delivery_locator"
        )
    }) {
        "clawd.msg.clarify_missing_file_locator"
    } else {
        return None;
    };
    let default_payload = json!({
        "message_key": key,
        "reason_code": &contract.reason_code,
        "missing_slots": &contract.missing_slots,
    })
    .to_string();
    Some(crate::bilingual_t_with_default_vars(
        state,
        key,
        &default_payload,
        &default_payload,
        fallback_prefers_english_for_language_hint(state, &contract.language_hint),
        &[],
    ))
}

fn user_response_contract_local_shape_satisfied(
    contract: &UserResponseContract,
    reply: &str,
) -> bool {
    let trimmed = reply.trim();
    if trimmed.is_empty() || trimmed.starts_with('{') || trimmed.starts_with('[') {
        return false;
    }
    if trimmed.contains("```") {
        return false;
    }
    if reply_contains_obvious_internal_trace(trimmed) {
        return false;
    }
    match contract.response_shape.trim() {
        "one_short_clarification" => concise_single_line_reply_satisfied(trimmed, 160),
        "one_short_confirmation_question" => concise_single_line_reply_satisfied(trimmed, 220),
        _ => true,
    }
}

fn user_response_contract_validation_accepts(
    contract: &UserResponseContract,
    validation: &UserResponseContractValidationOut,
) -> bool {
    if validation.confidence < 0.55 {
        return true;
    }
    if !validation.satisfies_contract
        || validation.false_claims
        || validation.mentions_internal_details
    {
        return false;
    }
    if contract.response_shape.trim() == "one_short_clarification"
        && !validation.asks_for_missing_target
    {
        return false;
    }
    true
}

async fn user_response_contract_llm_validated(
    state: &AppState,
    task: &crate::ClaimedTask,
    contract: &UserResponseContract,
    reply: &str,
) -> bool {
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        USER_RESPONSE_CONTRACT_VALIDATOR_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            tracing::info!(
                "user_response_contract_validator prompt_missing task_id={} err={}",
                task.task_id,
                err
            );
            return true;
        }
    };
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            (
                "__USER_RESPONSE_CONTRACT__",
                &contract.to_prompt_context_block(),
            ),
            ("__CANDIDATE_REPLY__", reply.trim()),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "user_response_contract_validator_prompt",
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
                "user_response_contract_validator llm_failed task_id={} err={}",
                task.task_id,
                err
            );
            return true;
        }
    };
    let validation = match crate::prompt_utils::validate_against_schema::<
        UserResponseContractValidationOut,
    >(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::UserResponseContractValidator,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok || validated.schema_normalized {
                tracing::info!(
                        "user_response_contract_validator schema_parse_recovery task_id={} raw_parse_ok={} schema_normalized={}",
                        task.task_id,
                        validated.raw_parse_ok,
                        validated.schema_normalized
                    );
            }
            UserResponseContractValidationOut {
                confidence: validated.value.confidence.clamp(0.0, 1.0),
                ..validated.value
            }
        }
        Err(err) => {
            tracing::info!(
                "user_response_contract_validator schema_validation_failed task_id={} err={}",
                task.task_id,
                err
            );
            return true;
        }
    };
    let accepted = user_response_contract_validation_accepts(contract, &validation);
    if !accepted {
        tracing::info!(
            task_id = %task.task_id,
            satisfies_contract = validation.satisfies_contract,
            false_claims = validation.false_claims,
            asks_for_missing_target = validation.asks_for_missing_target,
            mentions_internal_details = validation.mentions_internal_details,
            confidence = validation.confidence,
            reason = %validation.reason,
            "user_response_contract_validator_reject"
        );
    }
    accepted
}

fn reply_contains_obvious_internal_trace(reply: &str) -> bool {
    let lower = reply.to_ascii_lowercase();
    [
        "fallback_source",
        "resolver_reason",
        "user_response_contract",
        "prompt_name",
        "prompt_source",
        "task_id=",
        "call_id=",
        "raw provider error",
        "raw model output",
        "stack trace",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn concise_single_line_reply_satisfied(reply: &str, max_chars: usize) -> bool {
    let nonempty_lines = reply.lines().filter(|line| !line.trim().is_empty()).count();
    nonempty_lines <= 1 && reply.chars().count() <= max_chars
}

/// 失败时给用户的兜底答案 source 分类，决定 i18n 文案 + tracing label。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClarifyFallbackSource {
    /// LLM 调用直接返回 `Err`：401 / 熔断 / 网络 / 超时。
    LlmUnavailable,
    /// LLM 调用 OK 但返回内容 trim 后为空。
    EmptyResponse,
    /// 路由层没看出明确意图（normalizer 信心不足 + clarify_question 也空）。
    IntentUnresolved,
    /// 预留：planner 多轮失败 / repair 兜不住。
    PlannerFailed,
    /// 预留：执行链中途失败但有部分有效 step 输出。
    ExecutionFailedPartial,
    /// finalize 判定 requires_clarify 或 delivery 全空，无法合成最终答案。
    SynthesisEmpty,
    /// 预留：§7.1 contract verifier 二次拒绝。
    VerifyRejected,
    /// 策略 / 权限阻断后，用户可见说明交给 composer 生成。
    PolicyBlock,
}

impl ClarifyFallbackSource {
    /// tracing / 日志 / 后续 metric 用的稳定 label（snake_case）。
    pub(crate) fn as_metric_label(self) -> &'static str {
        match self {
            Self::LlmUnavailable => "llm_unavailable",
            Self::EmptyResponse => "empty_response",
            Self::IntentUnresolved => "intent_unresolved",
            Self::PlannerFailed => "planner_failed",
            Self::ExecutionFailedPartial => "execution_failed_partial",
            Self::SynthesisEmpty => "synthesis_empty",
            Self::VerifyRejected => "verify_rejected",
            Self::PolicyBlock => "policy_block",
        }
    }

    /// i18n 字典 key。
    pub(crate) fn i18n_key(self) -> &'static str {
        match self {
            Self::LlmUnavailable => "clawd.msg.fallback.llm_unavailable",
            Self::EmptyResponse => "clawd.msg.fallback.empty_response",
            Self::IntentUnresolved => "clawd.msg.fallback.intent_unresolved",
            Self::PlannerFailed => "clawd.msg.fallback.planner_failed",
            Self::ExecutionFailedPartial => "clawd.msg.fallback.execution_failed_partial",
            Self::SynthesisEmpty => "clawd.msg.fallback.synthesis_empty",
            Self::VerifyRejected => "clawd.msg.fallback.verify_rejected",
            Self::PolicyBlock => "clawd.msg.fallback.policy_block",
        }
    }

    /// Machine fallback used only when the i18n resource is unavailable.
    ///
    /// Runtime must not carry user-visible prose templates; localized wording lives in
    /// `configs/i18n/*.toml`.
    pub(crate) fn machine_default_payload(self) -> String {
        json!({
            "message_key": self.i18n_key(),
            "reason_code": self.as_metric_label(),
        })
        .to_string()
    }

    /// 全部已知 source 列表（用于集合化比对端）。
    pub(crate) fn all() -> &'static [Self] {
        &[
            Self::LlmUnavailable,
            Self::EmptyResponse,
            Self::IntentUnresolved,
            Self::PlannerFailed,
            Self::ExecutionFailedPartial,
            Self::SynthesisEmpty,
            Self::VerifyRejected,
            Self::PolicyBlock,
        ]
    }
}

/// 旧的"超级 fallback" i18n key，保留用于历史 DB 兼容比对（写入端不再使用）。
pub(crate) const LEGACY_SUPER_FALLBACK_KEY: &str = "clawd.msg.clarify_question_fallback";

pub(crate) fn legacy_super_fallback_machine_payload() -> String {
    json!({
        "message_key": LEGACY_SUPER_FALLBACK_KEY,
        "reason_code": "legacy_super_fallback",
    })
    .to_string()
}

/// 渲染 fallback 文案 + 上报 trace（统一入口）。
///
/// `context_hint` 仅用于 `ExecutionFailedPartial` / `VerifyRejected` 等带 `{context_hint}`
/// 占位符的文案；其它 source 传 `None` 即可。
pub(crate) fn clarify_fallback_text_with_language_hint(
    state: &AppState,
    source: ClarifyFallbackSource,
    context_hint: Option<&str>,
    language_hint: &str,
) -> String {
    let hint = context_hint.unwrap_or("").trim();
    let default_payload = source.machine_default_payload();
    crate::bilingual_t_with_default_vars(
        state,
        source.i18n_key(),
        &default_payload,
        &default_payload,
        fallback_prefers_english_for_language_hint(state, language_hint),
        &[("context_hint", hint)],
    )
}

pub(crate) fn render_clarify_fallback_with_language_hint(
    state: &AppState,
    task_id: &str,
    source: ClarifyFallbackSource,
    context_hint: Option<&str>,
    language_hint: &str,
) -> String {
    let hint = context_hint.unwrap_or("").trim();
    tracing::info!(
        task_id = %task_id,
        fallback_source = source.as_metric_label(),
        context_hint = %hint,
        "clarify_fallback_emitted"
    );
    clarify_fallback_text_with_language_hint(state, source, context_hint, language_hint)
}

pub(crate) fn fallback_prefers_english_for_language_hint(
    state: &AppState,
    language_hint: &str,
) -> bool {
    let normalized = language_hint.trim().to_ascii_lowercase();
    if normalized.starts_with("en") {
        true
    } else if normalized.starts_with("zh") {
        false
    } else {
        state
            .policy
            .command_intent
            .default_locale
            .trim()
            .to_ascii_lowercase()
            .starts_with("en")
    }
}

/// 底层 helper：直接接受 `i18n_dict`，不依赖 `AppState`，便于单测。
pub(crate) fn all_clarify_fallback_texts_from_dict(dict: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for src in ClarifyFallbackSource::all() {
        let default_payload = src.machine_default_payload();
        push_fallback_text_variants(
            &mut out,
            &lookup_or_default(dict, src.i18n_key(), &default_payload),
        );
        push_fallback_text_variants(&mut out, &default_payload);
    }
    let legacy_default = legacy_super_fallback_machine_payload();
    push_fallback_text_variants(
        &mut out,
        &lookup_or_default(dict, LEGACY_SUPER_FALLBACK_KEY, &legacy_default),
    );
    push_fallback_text_variants(&mut out, &legacy_default);
    out.sort();
    out.dedup();
    out
}

fn push_fallback_text_variants(out: &mut Vec<String>, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    out.push(trimmed.to_string());
    let context_empty = trimmed.replace("{context_hint}", "").trim().to_string();
    if !context_empty.is_empty() {
        out.push(context_empty);
    }
}

/// 判定一段文本是不是已知的 clarify-fallback 占位符。
/// 用于跳过这类回答，不污染 recent context / memory 拼接。
pub(crate) fn is_known_clarify_fallback_text(state: &AppState, text: &str) -> bool {
    is_known_clarify_fallback_text_with_dict(&state.policy.schedule.i18n_dict, text)
}

/// 底层 helper：直接接受 `i18n_dict`，不依赖 `AppState`，便于单测。
pub(crate) fn is_known_clarify_fallback_text_with_dict(
    dict: &HashMap<String, String>,
    text: &str,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    all_clarify_fallback_texts_from_dict(dict)
        .iter()
        .any(|known| known == trimmed)
}

fn lookup_or_default(dict: &HashMap<String, String>, key: &str, default_text: &str) -> String {
    dict.get(key)
        .cloned()
        .unwrap_or_else(|| default_text.to_string())
        .trim()
        .to_string()
}

#[cfg(test)]
#[path = "fallback_tests.rs"]
mod tests;
