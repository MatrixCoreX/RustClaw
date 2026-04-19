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
                "I got results but couldn't settle on a definitive answer. Which specific item do you want?"
            }
            Self::VerifyRejected => {
                "The model's answer didn't match the expected shape ({context_hint}). Could you tell me the exact form you want?"
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
pub(crate) fn all_clarify_fallback_texts_from_dict(
    dict: &HashMap<String, String>,
) -> Vec<String> {
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

fn lookup_or_default(
    dict: &HashMap<String, String>,
    key: &str,
    default_text: &str,
) -> String {
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
            "我需要确认一下：你这条消息是针对哪件事情？请补充目标或上下文，我立刻继续处理。".to_string(),
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
            "我拿到结果了但没整理出确定答案。你最想看的是哪一项？".to_string(),
        );
        assert!(is_known_clarify_fallback_text_with_dict(
            &dict,
            "我拿到结果了但没整理出确定答案。你最想看的是哪一项？"
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
