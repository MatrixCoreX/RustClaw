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
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UserResponseKind {
    Clarify,
    PolicyBlock,
    ToolFailure,
    LlmUnavailable,
    SchemaInvalid,
    FinalAnswer,
}

impl UserResponseKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Clarify => "clarify",
            Self::PolicyBlock => "policy_block",
            Self::ToolFailure => "tool_failure",
            Self::LlmUnavailable => "llm_unavailable",
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
        Self {
            kind: UserResponseKind::Clarify,
            reason_code: source.as_metric_label().to_string(),
            missing_slots: vec![source.as_metric_label().to_string()],
            observed_facts,
            policy_boundary: vec![
                "Do not expose internal route reasons, schema names, prompt names, or raw provider errors."
                    .to_string(),
                "Ask one concise, situation-specific clarification or recovery question.".to_string(),
            ],
            original_user_request: original_user_request.trim().to_string(),
            resolved_user_intent: String::new(),
            response_shape: "one_short_clarification".to_string(),
            language_hint: language_hint.trim().to_string(),
        }
    }

    pub(crate) fn verify_rejected(
        original_user_request: &str,
        resolved_user_intent: &str,
        response_shape: &str,
        semantic_kind: &str,
        verifier_reason: &str,
        language_hint: &str,
    ) -> Self {
        let mut observed_facts = Vec::new();
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
                "Do not expose internal verifier names, schema names, prompt names, or raw model output."
                    .to_string(),
                "Ask for the smallest missing delivery constraint only if the correct answer cannot be safely reshaped."
                    .to_string(),
                "If the user requested an exact output shape, acknowledge that shape in natural language without adding internal details."
                    .to_string(),
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
                "Do not expose internal verifier names, schema names, prompt names, or raw model output."
                    .to_string(),
                "Do not claim the blocked or unconfirmed action was executed.".to_string(),
                "If explicit confirmation is required, ask for confirmation in one concise sentence."
                    .to_string(),
                "If clarification is required, ask only for the smallest missing detail needed before execution."
                    .to_string(),
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

pub(crate) fn missing_file_delivery_response_text_for_language(
    state: &AppState,
    language_hint: &str,
    locator_hint: Option<&str>,
) -> String {
    let prefer_english = fallback_prefers_english_for_language_hint(state, language_hint);
    if let Some(locator) = locator_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return crate::app_helpers::bilingual_t_with_default_vars(
            state,
            "clawd.msg.delivery.file_not_found_path_next_step",
            "未找到文件：{path}，所以无法发送。请确认完整路径或上传该文件。",
            "File not found: {path}, so I cannot send it. Please confirm the full path or upload the file.",
            prefer_english,
            &[("path", locator)],
        );
    }
    crate::app_helpers::bilingual_t_with_default(
        state,
        "clawd.msg.delivery.file_not_found_next_step",
        "未找到该文件。请确认完整路径或上传该文件。",
        "File not found. Please confirm the full path or upload the file.",
        prefer_english,
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
    let _ = (task, original_user_request, resolved_user_intent);
    missing_file_delivery_response_text_for_language(state, language_hint, locator_hint)
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
    #[allow(dead_code)]
    PlannerFailed,
    /// 预留：执行链中途失败但有部分有效 step 输出。
    #[allow(dead_code)]
    ExecutionFailedPartial,
    /// finalize 判定 requires_clarify 或 delivery 全空，无法合成最终答案。
    SynthesisEmpty,
    /// 预留：§7.1 contract verifier 二次拒绝。
    #[allow(dead_code)]
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

    /// 默认英文文案（i18n 字典缺该 key 时兜底）。
    pub(crate) fn default_en(self) -> &'static str {
        match self {
            Self::LlmUnavailable => {
                "I could not reach the model service for this turn. Please retry, or switch to another available model."
            }
            Self::EmptyResponse => {
                "The model returned an empty answer this time. Please describe the goal more concretely and I'll try again."
            }
            Self::IntentUnresolved => {
                "I couldn't determine the requested action. Please add the target, context, and action you want."
            }
            Self::PlannerFailed => {
                "I couldn't break the request into executable steps. Please restate the goal, target, and constraints more concretely."
            }
            Self::ExecutionFailedPartial => {
                "I hit a problem partway through. Already done: {context_hint}. Want me to try a different path?"
            }
            Self::SynthesisEmpty => {
                "I couldn't produce a reliable final answer from the available evidence. Please add the missing target or ask me to retry the synthesis."
            }
            Self::VerifyRejected => {
                "The model's answer didn't match the expected shape ({context_hint}). Could you tell me the exact form you want?"
            }
            Self::PolicyBlock => {
                "This request is blocked by the current runtime policy. Adjust the policy or provide a safer target, then retry."
            }
        }
    }

    /// 默认中文文案（当前请求明确是中文、但运行时 i18n 字典不是中文时使用）。
    pub(crate) fn default_zh(self) -> &'static str {
        match self {
            Self::LlmUnavailable => "这次没有连上模型服务。请重试，或切换到其它可用模型继续。",
            Self::EmptyResponse => "模型这次没给出回答。请把目标说得更具体一点，我立刻再试。",
            Self::IntentUnresolved => {
                "我没看出这条消息要做什么。请补充目标、上下文或你希望我采取的动作。"
            }
            Self::PlannerFailed => "我没能把请求拆成可执行步骤。请补充目标、操作边界和关键约束。",
            Self::ExecutionFailedPartial => {
                "执行到一半遇到问题。已经完成的部分：{context_hint}。要不要换条路继续？"
            }
            Self::SynthesisEmpty => {
                "我还没能根据现有证据生成可靠最终答案。请补充缺少的目标，或让我重新整理一次。"
            }
            Self::VerifyRejected => {
                "模型给的答案不符合预期格式（{context_hint}）。能告诉我你最想要的形式吗？"
            }
            Self::PolicyBlock => "当前运行策略阻止了这个请求。请调整策略或提供更安全的目标后再试。",
        }
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
pub(crate) const LEGACY_SUPER_FALLBACK_DEFAULT_EN: &str =
    "I need to clarify: what task is this message about? Please provide the target or context.";
pub(crate) const LEGACY_SUPER_FALLBACK_DEFAULT_ZH: &str =
    "我需要确认一下：你这条消息是针对哪件事情？请补充目标或上下文。";

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
    crate::bilingual_t_with_default_vars(
        state,
        source.i18n_key(),
        source.default_zh(),
        source.default_en(),
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

/// 集合：当前可能出现在历史 task `result_json.text` 里的所有 fallback 文案
/// （新 7 个 source + 老 super-fallback）。比对端用以判定"上一轮回答是不是 fallback
/// 占位符"，决定要不要把它喂给 recent context / memory 上下文拼接。
///
/// 当前所有生产调用点都走更高层的 [`is_known_clarify_fallback_text`]，本函数留作
/// 调试与未来 inspect 工具的入口（例如 `inspect_task.sh --fallback-set`）。
#[allow(dead_code)]
pub(crate) fn all_clarify_fallback_texts(state: &AppState) -> Vec<String> {
    all_clarify_fallback_texts_from_dict(&state.policy.schedule.i18n_dict)
}

/// 底层 helper：直接接受 `i18n_dict`，不依赖 `AppState`，便于单测。
pub(crate) fn all_clarify_fallback_texts_from_dict(dict: &HashMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for src in ClarifyFallbackSource::all() {
        push_fallback_text_variants(
            &mut out,
            &lookup_or_default(dict, src.i18n_key(), src.default_en()),
        );
        push_fallback_text_variants(&mut out, src.default_en());
        push_fallback_text_variants(&mut out, src.default_zh());
    }
    push_fallback_text_variants(
        &mut out,
        &lookup_or_default(
            dict,
            LEGACY_SUPER_FALLBACK_KEY,
            LEGACY_SUPER_FALLBACK_DEFAULT_EN,
        ),
    );
    push_fallback_text_variants(&mut out, LEGACY_SUPER_FALLBACK_DEFAULT_EN);
    push_fallback_text_variants(&mut out, LEGACY_SUPER_FALLBACK_DEFAULT_ZH);
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
