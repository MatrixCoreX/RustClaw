//! 语义裁决（delivery_text_classifier）。
//!
//! # §3.4 调用面约束
//!
//! 本模块的两个 LLM 入口 [`is_meta_respond_instruction`] 与 [`is_publishable_raw`]
//! **只允许 finalize 层（`agent_engine::loop_finalize` 与
//! `agent_engine::observed_output::observed_answer_fallback`）调用**。
//!
//! 设计理由：
//! - 这两个判定本身需要发起 LLM 请求，把它放到 planning / skill_execution 等
//!   非最终阶段会让 per-task LLM 预算被早期阶段消耗光，并放大重复判定。
//! - finalize 层是"最终文本是否对外发布"的唯一仲裁点；planning / skill_execution
//!   等非最终阶段不再维护自然语言词表来预判 meta respond，应依赖 planner
//!   contract/prompt，并在最终发布前统一由本模块的 LLM classifier 仲裁。
//!
//! 守卫脚本：[`scripts/check_semantic_judge_callers.sh`](../../../../scripts/check_semantic_judge_callers.sh)
//! 用 grep 校验只有白名单文件 import 这两个 LLM 入口；新增调用方需先评审 §3.4。

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use crate::{llm_gateway, AppState, ClaimedTask};

const DELIVERY_TEXT_CLASSIFIER_PROMPT_LOGICAL_PATH: &str =
    "prompts/delivery_text_classifier_prompt.md";

#[derive(Debug, Clone, Deserialize)]
struct DeliveryTextClassifierOut {
    #[serde(default)]
    is_meta_instruction: bool,
    #[serde(default)]
    meta_reason: String,
    #[serde(default)]
    meta_confidence: f64,
    #[serde(default)]
    publishable: bool,
    #[serde(default)]
    publishable_reason: String,
    #[serde(default)]
    publishable_confidence: f64,
}

fn delivery_text_classifier_cache() -> &'static Mutex<HashMap<String, DeliveryTextClassifierOut>> {
    static CACHE: OnceLock<Mutex<HashMap<String, DeliveryTextClassifierOut>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 把文本归一化到一个稳定的形状：trim、折叠连续空白、剔除尾部标点。
///
/// 背景：`delivery_text_classifier_prompt` 在同一个 task 的 ask 流里可能被
/// 多个位置（planning / loop_finalize / observed_output / skill_execution）
/// 以"几乎相同但差一个换行/尾句号"的文本分别询问。未归一化时每个差异都会
/// 触发一次新的 LLM 调用，这里统一归一化后再做 cache key，从而把这些调用
/// 合并成 1 次。
fn normalize_classifier_text(text: &str) -> String {
    let trimmed = text.trim();
    let mut out = String::with_capacity(trimmed.len());
    let mut prev_whitespace = false;
    for ch in trimmed.chars() {
        if ch.is_whitespace() {
            if !prev_whitespace {
                out.push(' ');
            }
            prev_whitespace = true;
        } else {
            out.push(ch);
            prev_whitespace = false;
        }
    }
    while let Some(last) = out.chars().last() {
        if matches!(
            last,
            '.' | ',' | ';' | ':' | '!' | '?' | '。' | '，' | '；' | '：' | '！' | '？'
        ) {
            out.pop();
        } else {
            break;
        }
    }
    out
}

fn delivery_text_cache_key(task: &ClaimedTask, text: &str) -> String {
    format!("{}\n{}", task.task_id, normalize_classifier_text(text))
}

fn looks_like_concrete_delivery_artifact(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return false;
    }
    if crate::finalize::parse_delivery_file_token(trimmed).is_some() {
        return true;
    }
    Path::new(trimmed).is_absolute()
        || (trimmed.len() >= 3
            && trimmed.as_bytes()[1] == b':'
            && matches!(trimmed.as_bytes()[2], b'/' | b'\\')
            && trimmed.as_bytes()[0].is_ascii_alphabetic())
}

async fn classify_delivery_text_with_llm(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
) -> Option<DeliveryTextClassifierOut> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Some(DeliveryTextClassifierOut {
            is_meta_instruction: false,
            meta_reason: "empty".to_string(),
            meta_confidence: 1.0,
            publishable: false,
            publishable_reason: "empty".to_string(),
            publishable_confidence: 1.0,
        });
    }
    let cache_key = delivery_text_cache_key(task, trimmed);
    if let Ok(cache) = delivery_text_classifier_cache().lock() {
        if let Some(cached) = cache.get(&cache_key) {
            return Some(cached.clone());
        }
    }
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        DELIVERY_TEXT_CLASSIFIER_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            tracing::info!(
                "delivery_text_classifier prompt_missing task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    let prompt = crate::render_prompt_template(&resolved.template, &[("__TEXT__", trimmed)]);
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "delivery_text_classifier_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out =
        llm_gateway::run_with_fallback_with_prompt_source(state, task, &prompt, &prompt_source)
            .await
            .ok()?;
    let parsed = match crate::prompt_utils::validate_against_schema::<DeliveryTextClassifierOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::DeliveryTextClassifier,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok {
                tracing::info!(
                    "delivery_text_classifier schema_parse_recovery task_id={} schema_normalized={}",
                    task.task_id,
                    validated.schema_normalized
                );
            }
            validated.value
        }
        Err(err) => {
            tracing::info!(
                "delivery_text_classifier schema_validation_failed task_id={} err={}",
                task.task_id,
                err
            );
            return None;
        }
    };
    let normalized = DeliveryTextClassifierOut {
        is_meta_instruction: parsed.is_meta_instruction,
        meta_reason: parsed.meta_reason,
        meta_confidence: parsed.meta_confidence.clamp(0.0, 1.0),
        publishable: parsed.publishable,
        publishable_reason: parsed.publishable_reason,
        publishable_confidence: parsed.publishable_confidence.clamp(0.0, 1.0),
    };
    if let Ok(mut cache) = delivery_text_classifier_cache().lock() {
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(cache_key, normalized.clone());
    }
    Some(normalized)
}

/// §3.4 finalize-tier LLM 入口：判定 text 是否是"过程指令/占位"而非可对外发布。
///
/// **调用面约束**：仅允许 `agent_engine::loop_finalize` 与
/// `agent_engine::observed_output::observed_answer_fallback`（finalize 兜底）调用。
/// 其它层不要维护自然语言词表来预判 meta respond。
pub(crate) async fn is_meta_respond_instruction(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() > 600 {
        return false;
    }
    if looks_like_concrete_delivery_artifact(trimmed) {
        return false;
    }
    classify_delivery_text_with_llm(state, task, trimmed)
        .await
        .map(|out| out.is_meta_instruction && out.meta_confidence >= 0.55)
        .unwrap_or(false)
}

fn is_publishable_raw_local_guard(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() || t.len() <= 2 {
        return false;
    }
    if crate::finalize::looks_like_planner_artifact(t) {
        return false;
    }
    if t.chars()
        .all(|c| c.is_ascii_digit() || c.is_ascii_punctuation() || c.is_whitespace())
    {
        return false;
    }
    true
}

/// 测试专用：验证 [`is_publishable_raw`] 的本地 guard 部分，不发任何 LLM 请求。
#[cfg(test)]
fn is_publishable_raw_local(s: &str) -> bool {
    is_publishable_raw_local_guard(s)
}

/// §3.4 finalize-tier LLM 入口：判定 s 是否值得作为最终答覆对外发布。
///
/// 本地 guard 只过滤空值、明显 planner artifact、纯数字/符号这类结构性非答案；
/// 自然语言可发布性仍交给 delivery_text_classifier。
pub(crate) async fn is_publishable_raw(state: &AppState, task: &ClaimedTask, s: &str) -> bool {
    if !is_publishable_raw_local_guard(s) {
        return false;
    }
    let trimmed = s.trim();
    if looks_like_concrete_delivery_artifact(trimmed) {
        return true;
    }
    if trimmed.chars().count() > 180 {
        return true;
    }
    classify_delivery_text_with_llm(state, task, trimmed)
        .await
        .map(|out| {
            if out.publishable_confidence >= 0.55 {
                out.publishable
            } else {
                true
            }
        })
        .unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::{
        is_publishable_raw_local, looks_like_concrete_delivery_artifact, normalize_classifier_text,
    };
    use serde_json::Value;

    #[test]
    fn delivery_text_classifier_schema_drift() {
        const SCHEMA_RAW: &str =
            include_str!("../../../prompts/schemas/delivery_text_classifier.schema.json");
        let schema: Value = serde_json::from_str(SCHEMA_RAW)
            .expect("delivery_text_classifier.schema.json must be valid JSON");
        let properties = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("schema.properties must be an object");
        for field in [
            "is_meta_instruction",
            "meta_reason",
            "meta_confidence",
            "publishable",
            "publishable_reason",
            "publishable_confidence",
        ] {
            assert!(
                properties.contains_key(field),
                "schema missing parser field `{field}` under properties — sync prompts/schemas/delivery_text_classifier.schema.json with DeliveryTextClassifierOut",
            );
        }

        let probe = serde_json::json!({
            "is_meta_instruction": false,
            "meta_reason": "user_facing_result",
            "meta_confidence": 0.8,
            "publishable": true,
            "publishable_reason": "meaningful_result",
            "publishable_confidence": 0.9
        });
        let validated = crate::prompt_utils::validate_against_schema::<Value>(
            &probe.to_string(),
            crate::prompt_utils::PromptSchemaId::DeliveryTextClassifier,
        )
        .expect("classifier probe should validate");
        assert_eq!(
            validated.value.get("publishable").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn normalize_collapses_whitespace_and_trims_trailing_punct() {
        let a = normalize_classifier_text("  Hello\nworld.  ");
        let b = normalize_classifier_text("Hello  world.\n");
        let c = normalize_classifier_text("Hello\tworld");
        assert_eq!(a, "Hello world");
        assert_eq!(b, "Hello world");
        assert_eq!(c, "Hello world");
    }

    #[test]
    fn normalize_handles_cjk_punctuation() {
        assert_eq!(normalize_classifier_text("好的。"), "好的");
        assert_eq!(normalize_classifier_text("好的，"), "好的");
        assert_eq!(normalize_classifier_text("好的！？"), "好的");
    }

    #[test]
    fn normalize_preserves_internal_punctuation() {
        assert_eq!(normalize_classifier_text("This is, fine."), "This is, fine");
    }

    #[test]
    fn local_publishable_rejects_empty_and_filler() {
        assert!(!is_publishable_raw_local(""));
        assert!(!is_publishable_raw_local("  "));
        assert!(!is_publishable_raw_local("a"));
        // 纯标点
        assert!(!is_publishable_raw_local(".....!?"));
        assert!(!is_publishable_raw_local("123 456"));
    }

    #[test]
    fn local_publishable_accepts_real_content() {
        assert!(is_publishable_raw_local(
            "已完成任务，结果保存在 /tmp/out.md"
        ));
        assert!(is_publishable_raw_local(
            "The result is 42 with confidence 0.97."
        ));
        assert!(is_publishable_raw_local("没找到该文件"));
    }

    #[test]
    fn local_delivery_artifact_guard_accepts_paths_and_file_tokens() {
        assert!(looks_like_concrete_delivery_artifact(
            "/home/guagua/rustclaw/document/pwd_line.txt"
        ));
        assert!(looks_like_concrete_delivery_artifact(
            "FILE:/home/guagua/rustclaw/document/pwd_line.txt"
        ));
        assert!(looks_like_concrete_delivery_artifact(
            "C:\\Users\\demo\\pwd_line.txt"
        ));
        assert!(!looks_like_concrete_delivery_artifact("pwd_line.txt"));
        assert!(!looks_like_concrete_delivery_artifact(
            "read pwd_line.txt and summarize it"
        ));
    }
}
