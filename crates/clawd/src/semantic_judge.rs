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
//! - finalize 层是"最终文本是否对外发布"的唯一仲裁点；其他层若要做"看起来类似"
//!   的判断，应使用本模块顶部的两个 _local_ 启发式函数：
//!   [`looks_like_meta_respond_directive_local`] 与 [`is_publishable_raw_local`]。
//!
//! 守卫脚本：[`scripts/check_semantic_judge_callers.sh`](../../../../scripts/check_semantic_judge_callers.sh)
//! 用 grep 校验只有白名单文件 import 这两个 LLM 入口；新增调用方需先评审 §3.4。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use serde::Deserialize;

use crate::{llm_gateway, AppState, ClaimedTask};

const DELIVERY_TEXT_CLASSIFIER_PROMPT_TEMPLATE: &str =
    include_str!("../../../prompts/layers/overlays/delivery_text_classifier_prompt.md");
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
    let resolved = crate::load_prompt_template_for_state_with_meta(
        state,
        DELIVERY_TEXT_CLASSIFIER_PROMPT_LOGICAL_PATH,
        DELIVERY_TEXT_CLASSIFIER_PROMPT_TEMPLATE,
    );
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
    let trimmed_out = llm_out.trim();
    let parsed_raw = serde_json::from_str::<DeliveryTextClassifierOut>(trimmed_out).ok();
    let parsed = parsed_raw.or_else(|| {
        crate::extract_first_json_object_any(&llm_out)
            .and_then(|json| serde_json::from_str::<DeliveryTextClassifierOut>(&json).ok())
    })?;
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
/// 其它层应使用 [`looks_like_meta_respond_directive_local`] 作为本地启发。
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
    classify_delivery_text_with_llm(state, task, trimmed)
        .await
        .map(|out| out.is_meta_instruction && out.meta_confidence >= 0.55)
        .unwrap_or(false)
}

fn is_publishable_raw_deterministic_guard(s: &str) -> bool {
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

/// §3.4 本地启发式：纯确定性的 "可对外发布" 预判，**不调 LLM**。
///
/// 给非 finalize 层（planning / skill_execution / agent loop 缓存）使用，
/// 用来决定 "这段输出值不值得缓存 / 让进 plan 步骤"。漏判（误判为可发布）
/// 会在 finalize 层被 [`is_publishable_raw`] 二次过滤；过滤过严（误判为
/// 不可发布）则跳过本地缓存，finalize 层照常生成兜底输出，行为正确仅是少
/// 享受一次 fast-path。
///
/// 与 `is_publishable_raw` 的关系：本函数 = 前者的"deterministic guard 部分"，
/// 即不含长度短路（>180 字直接 true）也不发任何 LLM 请求。
pub(crate) fn is_publishable_raw_local(s: &str) -> bool {
    is_publishable_raw_deterministic_guard(s)
}

/// §3.4 本地启发式：纯确定性的 "看起来像 meta-respond 指令" 预判，**不调 LLM**。
///
/// 覆盖 `delivery_text_classifier_prompt.md` 中文档化的高发模式（中英）。
/// 给 planning 阶段过滤明显的 "请告诉用户 ... / tell the user ..." 类
/// 占位 Respond 步骤使用；漏掉的非典型表达会在 finalize 层被
/// [`is_meta_respond_instruction`] 兜底剔除。
///
/// 设计取舍：宁可漏判也不要误判成 meta（误判会丢真实答案；漏判只是浪费
/// 一个 plan step，由 finalize 层兜底）。所以模式列表保守，只匹配高置信
/// 关键词组合。
pub(crate) fn looks_like_meta_respond_directive_local(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().count() > 600 {
        return false;
    }
    let lower = trimmed.to_lowercase();

    // 中文："请/麻烦 + 告诉/通知/回复/告知 + 用户/对方/他/她"
    // 例：请告诉用户 ... / 麻烦回复用户 ... / 请通知对方 ...
    const ZH_VERBS: &[&str] = &["告诉", "通知", "回复", "告知", "回答"];
    const ZH_TARGETS: &[&str] = &["用户", "对方", "他", "她", "TA", "ta"];
    const ZH_LEADS: &[&str] = &["请", "麻烦", "请你", "麻烦你"];
    for lead in ZH_LEADS {
        if !trimmed.starts_with(lead) {
            continue;
        }
        let head = &trimmed[..trimmed.len().min(lead.len() + 60)];
        for verb in ZH_VERBS {
            if !head.contains(verb) {
                continue;
            }
            for target in ZH_TARGETS {
                if head.contains(target) {
                    return true;
                }
            }
        }
    }

    // 中文："请阅读 ... 并 + 总结/告诉/回复/输出"，"请检查 ... 后告诉/告知"
    if trimmed.starts_with("请阅读") || trimmed.starts_with("请检查") || trimmed.starts_with("请查看")
    {
        for tail in ["并总结", "并告诉", "并回复", "并输出", "后告诉", "后告知"] {
            if trimmed.contains(tail) {
                return true;
            }
        }
    }

    // 中文："下一步我会 / 我将继续 / 请稍等 + 我先 / 让我先 + 分析/检查/查看/读取"
    const ZH_PROCESS_LEADS: &[&str] = &[
        "下一步我会",
        "下一步我将",
        "我将继续",
        "我会继续",
        "我先",
        "让我先",
        "请稍等",
    ];
    if ZH_PROCESS_LEADS.iter().any(|lead| trimmed.starts_with(lead))
        && ["分析", "检查", "查看", "读取", "处理", "确认"]
            .iter()
            .any(|w| trimmed.contains(w))
    {
        return true;
    }

    // 英文："tell/inform/notify/reply (to) the user ..."
    const EN_PHRASES: &[&str] = &[
        "tell the user",
        "tell user",
        "inform the user",
        "notify the user",
        "reply to the user",
        "respond to the user",
        "let the user know",
        "ask the user to",
    ];
    for p in EN_PHRASES {
        if lower.starts_with(p) || lower.starts_with(&format!("please {p}")) {
            return true;
        }
    }

    // 英文："i will / i'll / let me + analyze/check/read/look at/process ..."
    const EN_PROCESS_LEADS: &[&str] = &["i will ", "i'll ", "let me ", "next i will ", "next, i will "];
    if EN_PROCESS_LEADS.iter().any(|lead| lower.starts_with(lead))
        && [
            "analyze", "check", "read", "look at", "process", "investigate", "examine",
        ]
        .iter()
        .any(|w| lower.contains(w))
    {
        return true;
    }

    false
}

/// §3.4 finalize-tier LLM 入口：判定 s 是否值得作为最终答覆对外发布。
///
/// **调用面约束**：仅允许 `agent_engine::loop_finalize` 调用（finalize 决策）。
/// 其它层（planning / skill_execution / 缓存）应使用
/// [`is_publishable_raw_local`] 作为本地启发。
pub(crate) async fn is_publishable_raw(state: &AppState, task: &ClaimedTask, s: &str) -> bool {
    if !is_publishable_raw_deterministic_guard(s) {
        return false;
    }
    let trimmed = s.trim();
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
        is_publishable_raw_local, looks_like_meta_respond_directive_local,
        normalize_classifier_text,
    };

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
        assert_eq!(
            normalize_classifier_text("This is, fine."),
            "This is, fine"
        );
    }

    #[test]
    fn local_meta_respond_zh_tell_user_patterns() {
        assert!(looks_like_meta_respond_directive_local("请告诉用户当前进度"));
        assert!(looks_like_meta_respond_directive_local("麻烦回复用户结果"));
        assert!(looks_like_meta_respond_directive_local("请通知对方失败原因"));
        assert!(looks_like_meta_respond_directive_local("请你告知用户错误"));
    }

    #[test]
    fn local_meta_respond_zh_read_then_summarize() {
        assert!(looks_like_meta_respond_directive_local(
            "请阅读 /tmp/foo.md 并总结要点"
        ));
        assert!(looks_like_meta_respond_directive_local(
            "请检查日志后告诉我结果"
        ));
        assert!(looks_like_meta_respond_directive_local(
            "请查看配置后告知差异"
        ));
    }

    #[test]
    fn local_meta_respond_zh_process_phrases() {
        assert!(looks_like_meta_respond_directive_local("下一步我会分析这份日志"));
        assert!(looks_like_meta_respond_directive_local("我先检查一下文件内容"));
        assert!(looks_like_meta_respond_directive_local("让我先读取一下数据"));
        assert!(looks_like_meta_respond_directive_local("请稍等，我先确认配置"));
    }

    #[test]
    fn local_meta_respond_en_tell_user_patterns() {
        assert!(looks_like_meta_respond_directive_local("Tell the user the progress."));
        assert!(looks_like_meta_respond_directive_local(
            "Please reply to the user with the result."
        ));
        assert!(looks_like_meta_respond_directive_local(
            "Inform the user that the file is missing."
        ));
        assert!(looks_like_meta_respond_directive_local(
            "Let the user know we're processing."
        ));
    }

    #[test]
    fn local_meta_respond_en_process_phrases() {
        assert!(looks_like_meta_respond_directive_local(
            "I will analyze this file first."
        ));
        assert!(looks_like_meta_respond_directive_local("Let me read the log."));
        assert!(looks_like_meta_respond_directive_local(
            "I'll check the configuration."
        ));
    }

    #[test]
    fn local_meta_respond_does_not_misfire_on_real_answers() {
        // 真实答案不应被误判
        assert!(!looks_like_meta_respond_directive_local("已完成"));
        assert!(!looks_like_meta_respond_directive_local("没找到该文件"));
        assert!(!looks_like_meta_respond_directive_local("当前用户名是 alice"));
        assert!(!looks_like_meta_respond_directive_local(
            "Docker 容器和虚拟机的主要区别在于资源隔离和性能。"
        ));
        assert!(!looks_like_meta_respond_directive_local(
            "The container shares the host kernel."
        ));
        // delivery token 不应误判
        assert!(!looks_like_meta_respond_directive_local("FILE:/tmp/output.md"));
        assert!(!looks_like_meta_respond_directive_local("IMAGE_FILE:/tmp/x.png"));
    }

    #[test]
    fn local_meta_respond_handles_edge_cases() {
        assert!(!looks_like_meta_respond_directive_local(""));
        assert!(!looks_like_meta_respond_directive_local("   "));
        // 超长文本不再尝试匹配（避免大段最终答案被前缀误中）
        let long = "请告诉用户".to_string() + &"a".repeat(1000);
        assert!(!looks_like_meta_respond_directive_local(&long));
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
        assert!(is_publishable_raw_local("已完成任务，结果保存在 /tmp/out.md"));
        assert!(is_publishable_raw_local("The result is 42 with confidence 0.97."));
        assert!(is_publishable_raw_local("没找到该文件"));
    }
}
