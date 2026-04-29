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
use serde_json::json;

pub(crate) const USER_RESPONSE_COMPOSER_PROMPT_LOGICAL_PATH: &str =
    "prompts/user_response_composer_prompt.md";

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

    pub(crate) fn missing_file_delivery(
        original_user_request: &str,
        resolved_user_intent: &str,
        locator_hint: Option<&str>,
        language_hint: &str,
    ) -> Self {
        let mut observed_facts = vec![
            "file_delivery_required: true".to_string(),
            "fs_search_action: find_name".to_string(),
            "matched_files_count: 0".to_string(),
        ];
        if let Some(locator) = locator_hint
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            observed_facts.push(format!("locator_hint: {locator}"));
        }
        Self {
            kind: UserResponseKind::ToolFailure,
            reason_code: "missing_file_delivery_not_found".to_string(),
            missing_slots: Vec::new(),
            observed_facts,
            policy_boundary: vec![
                "Do not claim a file was found or delivered.".to_string(),
                "Do not invent alternative file paths or similar filenames.".to_string(),
                "Explain that the requested file delivery target was not found and give one concise recovery option."
                    .to_string(),
            ],
            original_user_request: original_user_request.trim().to_string(),
            resolved_user_intent: resolved_user_intent.trim().to_string(),
            response_shape: "brief_failure_with_next_step".to_string(),
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

pub(crate) fn missing_file_delivery_default_text(state: &AppState) -> String {
    crate::i18n_t_with_default(
        state,
        "clawd.msg.delivery.rule3_file_not_found",
        "File not found.",
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
    let default_text = missing_file_delivery_default_text(state);
    let contract = UserResponseContract::missing_file_delivery(
        original_user_request,
        resolved_user_intent,
        locator_hint,
        language_hint,
    );
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
                return render_clarify_fallback(
                    state,
                    &task.task_id,
                    ClarifyFallbackSource::LlmUnavailable,
                    Some(&err.to_string()),
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
                render_clarify_fallback(
                    state,
                    &task.task_id,
                    ClarifyFallbackSource::EmptyResponse,
                    None,
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
            render_clarify_fallback(
                state,
                &task.task_id,
                ClarifyFallbackSource::LlmUnavailable,
                Some(&hint),
            )
        }
    }
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
                "The model is temporarily unavailable (auth/network/circuit). Please retry later or switch to an available model."
            }
            Self::EmptyResponse => {
                "The model returned an empty answer this time. Please describe the goal more concretely and I'll try again."
            }
            Self::IntentUnresolved => {
                "I couldn't tell what this message wants me to do. Please add a target or context — for example, which file to look at or which action to perform."
            }
            Self::PlannerFailed => {
                "I couldn't break the request into executable steps. Please rephrase as \"do Y by using X\", or be more specific."
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

/// 渲染 fallback 文案 + 上报 trace（统一入口）。
///
/// `context_hint` 仅用于 `ExecutionFailedPartial` / `VerifyRejected` 等带 `{context_hint}`
/// 占位符的文案；其它 source 传 `None` 即可。
pub(crate) fn render_clarify_fallback(
    state: &AppState,
    task_id: &str,
    source: ClarifyFallbackSource,
    context_hint: Option<&str>,
) -> String {
    let hint = context_hint.unwrap_or("").trim();
    tracing::info!(
        task_id = %task_id,
        fallback_source = source.as_metric_label(),
        context_hint = %hint,
        "clarify_fallback_emitted"
    );
    crate::i18n_t_with_default_vars(
        state,
        source.i18n_key(),
        source.default_en(),
        &[("context_hint", hint)],
    )
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
    let mut out: Vec<String> = ClarifyFallbackSource::all()
        .iter()
        .map(|src| lookup_or_default(dict, src.i18n_key(), src.default_en()))
        .collect();
    out.push(lookup_or_default(
        dict,
        LEGACY_SUPER_FALLBACK_KEY,
        LEGACY_SUPER_FALLBACK_DEFAULT_EN,
    ));
    out.sort();
    out.dedup();
    out
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
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// 7 source 的 metric label / i18n key 互不冲突。
    #[test]
    fn metric_labels_and_i18n_keys_are_unique_per_source() {
        let labels: HashSet<&'static str> = ClarifyFallbackSource::all()
            .iter()
            .map(|s| s.as_metric_label())
            .collect();
        assert_eq!(labels.len(), ClarifyFallbackSource::all().len());

        let keys: HashSet<&'static str> = ClarifyFallbackSource::all()
            .iter()
            .map(|s| s.i18n_key())
            .collect();
        assert_eq!(keys.len(), ClarifyFallbackSource::all().len());
    }

    #[test]
    fn user_response_contract_renders_structured_clarify_context() {
        let contract = UserResponseContract::clarify_from_fallback_source(
            ClarifyFallbackSource::IntentUnresolved,
            "看一下这个",
            "missing target",
            Some("candidate_context"),
            "zh-CN",
        );
        let block = contract.to_prompt_context_block();
        assert!(block.contains("USER_RESPONSE_CONTRACT"));
        assert!(block.contains("\"kind\": \"clarify\""));
        assert!(block.contains("\"reason_code\": \"intent_unresolved\""));
        assert!(block.contains("\"original_user_request\": \"看一下这个\""));
        assert!(block.contains("\"language_hint\": \"zh-CN\""));
        assert!(block.contains("candidate_context"));
    }

    #[test]
    fn user_response_contract_renders_structured_tool_failure_context() {
        let contract = UserResponseContract::tool_failure(
            "execution_recipe_missing_success_marker",
            "继续验证直到出现 OK",
            "Validate until the required success marker appears.",
            vec![
                "required_success_marker: OK".to_string(),
                "marker_observed: false".to_string(),
            ],
            vec!["Do not mark the run as successful.".to_string()],
            "brief_failure_with_next_step",
            "zh-CN",
        );
        let block = contract.to_prompt_context_block();
        assert!(block.contains("\"kind\": \"tool_failure\""));
        assert!(block.contains("\"reason_code\": \"execution_recipe_missing_success_marker\""));
        assert!(block.contains("required_success_marker: OK"));
        assert!(block.contains("brief_failure_with_next_step"));
        assert!(block.contains("Do not mark the run as successful."));
    }

    #[test]
    fn user_response_contract_renders_missing_file_delivery_context() {
        let contract = UserResponseContract::missing_file_delivery(
            "把 definitely_missing.txt 发给我",
            "Deliver definitely_missing.txt",
            Some("definitely_missing.txt"),
            "zh-CN",
        );
        let block = contract.to_prompt_context_block();
        assert!(block.contains("\"kind\": \"tool_failure\""));
        assert!(block.contains("\"reason_code\": \"missing_file_delivery_not_found\""));
        assert!(block.contains("matched_files_count: 0"));
        assert!(block.contains("locator_hint: definitely_missing.txt"));
        assert!(block.contains("Do not claim a file was found or delivered."));
    }

    #[test]
    fn user_response_contract_renders_verifier_gate_context() {
        let contract = UserResponseContract::verifier_gate(
            "execution_confirmation_required",
            "删除 logs 目录",
            "delete logs directory",
            vec!["explicit_user_confirmation".to_string()],
            vec![
                "verification_detail: destructive filesystem action".to_string(),
                "needs_confirmation: true".to_string(),
            ],
            "one_short_confirmation_question",
            "zh-CN",
        );
        let block = contract.to_prompt_context_block();
        assert!(block.contains("\"kind\": \"clarify\""));
        assert!(block.contains("\"reason_code\": \"execution_confirmation_required\""));
        assert!(block.contains("explicit_user_confirmation"));
        assert!(block.contains("destructive filesystem action"));
        assert!(block.contains("Do not claim the blocked or unconfirmed action was executed."));
    }

    #[test]
    fn user_response_contract_renders_structured_policy_block_context() {
        let contract = UserResponseContract::policy_block(
            "path_outside_workspace",
            "读取 /etc/shadow 第一行",
            "Read the first line of /etc/shadow.",
            vec!["denied_path: /etc/shadow".to_string()],
            vec![
                "Do not claim the path was read.".to_string(),
                "Explain the permission boundary and one safe next step.".to_string(),
            ],
            "zh-CN",
        );
        let block = contract.to_prompt_context_block();
        assert!(block.contains("\"kind\": \"policy_block\""));
        assert!(block.contains("\"reason_code\": \"path_outside_workspace\""));
        assert!(block.contains("denied_path: /etc/shadow"));
        assert!(block.contains("brief_failure_with_next_step"));
        assert!(block.contains("Do not claim the path was read."));
    }

    /// 每个 source 的英文默认文案非空，且 i18n key 都在
    /// `clawd.msg.fallback.` 命名空间下，避免被误用为其它字典。
    #[test]
    fn default_en_text_nonempty_and_key_namespaced() {
        for src in ClarifyFallbackSource::all() {
            assert!(!src.default_en().trim().is_empty(), "source={src:?}");
            assert!(
                src.i18n_key().starts_with("clawd.msg.fallback."),
                "source={src:?} key={}",
                src.i18n_key()
            );
        }
    }

    /// 老 super-fallback key 的默认文案一定在
    /// `all_clarify_fallback_texts_from_dict` 集合里（即使字典没显式配置）；
    /// 这是历史 DB 兼容性守底。
    #[test]
    fn all_texts_includes_legacy_super_fallback_default() {
        let empty_dict = HashMap::new();
        let texts = all_clarify_fallback_texts_from_dict(&empty_dict);
        assert!(
            texts
                .iter()
                .any(|t| t == LEGACY_SUPER_FALLBACK_DEFAULT_EN.trim()),
            "legacy default text missing from {texts:?}"
        );
    }

    /// 老 super-fallback key 即使被字典 override 成自定义文案，也仍能被
    /// `is_known_clarify_fallback_text_with_dict` 识别 —— 关键的历史 DB 兼容契约。
    #[test]
    fn legacy_super_fallback_recognized_when_overridden_by_dict() {
        let mut dict = HashMap::new();
        dict.insert(
            LEGACY_SUPER_FALLBACK_KEY.to_string(),
            "我需要确认一下：你这条消息是针对哪件事情？请补充目标或上下文，我立刻继续处理。"
                .to_string(),
        );
        assert!(is_known_clarify_fallback_text_with_dict(
            &dict,
            "我需要确认一下：你这条消息是针对哪件事情？请补充目标或上下文，我立刻继续处理。"
        ));
    }

    /// 任意 source 的默认英文文案，都能被 `is_known_*` 识别回来（用空 dict 跑，
    /// 强制走 default）。这是比对端 should_skip_* 正确性的核心契约。
    #[test]
    fn default_text_per_source_is_recognized_by_is_known() {
        let dict = HashMap::new();
        for src in ClarifyFallbackSource::all() {
            // ExecutionFailedPartial / VerifyRejected 默认文案带 {context_hint}
            // 占位符；用 lookup_or_default 拿到的就是含占位符的字面字符串，
            // is_known 比对走的是字面 == ，所以仍可识别。
            let text = lookup_or_default(&dict, src.i18n_key(), src.default_en());
            assert!(
                is_known_clarify_fallback_text_with_dict(&dict, &text),
                "source={src:?} text={text:?} not recognized by is_known"
            );
        }
    }

    /// 字典里配置了某 source 文案，且历史 DB 里写入的是该 source 的渲染结果
    /// （含已替换的 {context_hint} → 空），可被识别。这是新 source 上线后
    /// 比对端"无字符串硬编码"契约的正向例。
    #[test]
    fn dict_overridden_source_text_is_recognized() {
        let mut dict = HashMap::new();
        dict.insert(
            ClarifyFallbackSource::SynthesisEmpty.i18n_key().to_string(),
            "我还没能根据现有证据生成可靠最终答案。请补充缺少的目标。".to_string(),
        );
        assert!(is_known_clarify_fallback_text_with_dict(
            &dict,
            "我还没能根据现有证据生成可靠最终答案。请补充缺少的目标。"
        ));
    }

    /// 空字符串 / 空白不应被识别为 fallback（避免误把"答案是空"当成 fallback 去跳过）。
    #[test]
    fn blank_text_is_not_recognized_as_fallback() {
        let dict = HashMap::new();
        assert!(!is_known_clarify_fallback_text_with_dict(&dict, ""));
        assert!(!is_known_clarify_fallback_text_with_dict(&dict, "   "));
        assert!(!is_known_clarify_fallback_text_with_dict(&dict, "\n\n"));
    }

    /// 普通成功答案不应被识别为 fallback（防止误伤）。
    #[test]
    fn normal_answer_text_is_not_recognized_as_fallback() {
        let dict = HashMap::new();
        for sample in [
            "有，路径：rustclaw.service",
            "/home/guagua/rustclaw/Cargo.toml",
            "README.md",
            "执行成功，已写入 3 个文件。",
        ] {
            assert!(
                !is_known_clarify_fallback_text_with_dict(&dict, sample),
                "sample={sample:?} unexpectedly recognized as fallback"
            );
        }
    }
}
