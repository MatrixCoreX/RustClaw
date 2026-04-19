// §7.3 clarify reply / locator follow-up shortpath.
//
// 触发证据 (2026-04-19)：multi-turn case clarify_sqlite_schema_version_fixture /
// context_alias_switch_archive_chain / clarify_find_which_script 一类 ——
// turn N-1 系统问 "请提供路径"，turn N 用户只贴一个 path/单文件名，按理说
// normalizer 完全没必要再跑一次，因为：
//   1. "上一轮要 clarify 什么" 已经写在 last_turn_full / [clarification_requested]
//   2. "用户回了什么" 可以靠 prompt_looks_like_clarify_target_only 判定
// normalizer 那次 LLM 调用就是纯延迟（3-5s）+ token 预算，且偶尔自己抖手把
// 续答当成新请求降级 clarify_fallback。
//
// V1 故意收窄：
//   - 只跳过 normalizer LLM；不跳 planner（属于 §7.4 范畴）
//   - 不 resolve alias-deictic（"那个文件"指 X 已绑定）；alias 走 retrieval
//     型 fact，没有 KV，要做需要扩 schema/扩 retrieval，留 follow-up
//   - output_contract 取默认；§7.1 verifier 在出口兜底形态规范
//
// 收益：clarify 续答场景 normalizer 省一次 LLM；命中可观测（tracing），
// 不命中走原路（不破坏现有逻辑）。

use crate::{
    AskMode, IntentOutputContract, ResumeBehavior, RiskCeiling, RoutedMode, RouteResult,
    ScheduleKind,
};

/// Shortpath 探针命中后的最小路由信息。调用方据此构造 RouteResult。
#[derive(Debug, Clone)]
pub(crate) struct ClarifyShortpathHit {
    /// 拼接后的下游 prompt：prior + current（与 clarify_followup_routing_prompt 同样式）。
    pub(crate) resolved_intent: String,
    /// 上一轮用户原话（取 last_turn_full 第一段 "User: ..." 行）。
    pub(crate) prior_user_text: String,
    /// 本轮用户原话（trim 后的 prompt）。
    pub(crate) current_user_text: String,
    /// 命中原因 label（用于 route_reason + tracing）。
    pub(crate) reason: ShortpathReason,
}

/// 命中原因细分。V1 只有一种；预留位置给未来 alias-deictic / single-word fill。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShortpathReason {
    /// 上一轮 clarify + 当前消息明显是 locator/path/单文件名续答。
    ClarifyLocatorReply,
}

impl ShortpathReason {
    pub(crate) fn as_metric_label(self) -> &'static str {
        match self {
            ShortpathReason::ClarifyLocatorReply => "clarify_locator_reply",
        }
    }

    fn route_reason_text(self) -> &'static str {
        match self {
            ShortpathReason::ClarifyLocatorReply => "clarify_followup_shortpath_locator_reply_v1",
        }
    }
}

/// 上一轮助手是否问了 clarify？走 [clarification_requested] 占位符判定。
///
/// 占位符由 `memory::classify_assistant_context_reply_kind` 在 build_last_turn_full_context
/// 阶段写入，直接 contains 比 reflexive parse 更稳。
pub(crate) fn last_turn_was_clarify(last_turn_full: &str) -> bool {
    last_turn_full.contains("[clarification_requested]")
}

/// 提取 last_turn_full 里第一段 "User: ..." 行（即上一轮用户原话）。
pub(crate) fn extract_prior_user_text(last_turn_full: &str) -> Option<String> {
    last_turn_full
        .lines()
        .find_map(|line| line.trim().strip_prefix("User: "))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
}

/// 当前 prompt 是否看起来"整段就是 clarify 槽位的 locator/path/单文件名"。
///
/// 故意比 ask_prepare::prompt_looks_like_clarify_target_only 更严：那个口径只
/// 判 "prompt 中含 locator token"（用来增强 normalizer 输入，错了代价小），
/// shortpath 跳 normalizer 错了代价大，必须 "整段都是 locator-like token"。
///
/// 命中规则（任一即可）：
///   1. 整段是合法 inline JSON value（结构化续答）
///   2. 整段（按 whitespace + 中英文逗号/分号分词）所有 token 都是 locator-like
///      （path / url / filename.ext / 大写裸 stem 如 README），且至少有一个
///      "明确路径/URL" token（含 / \ 或扩展名 / 协议头），且 token 总数 ≤ 4
pub(crate) fn prompt_looks_like_locator_only(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    if trimmed.is_empty() {
        return false;
    }
    if prompt_is_inline_json_value(trimmed) {
        return true;
    }
    let tokens: Vec<&str> = trimmed
        .split(|ch: char| ch.is_whitespace() || matches!(ch, ',' | ';' | '，' | '；' | '、'))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() || tokens.len() > 4 {
        return false;
    }
    let mut has_explicit_locator = false;
    for token in &tokens {
        let cleaned = trim_locator_punct(token);
        if cleaned.is_empty() {
            return false;
        }
        let explicit =
            cleaned.contains('/') || cleaned.contains('\\') || is_url_like(cleaned);
        let filename_like = looks_like_filename_token(cleaned);
        let bare_stem = looks_like_bare_uppercase_stem(cleaned);
        if !(explicit || filename_like || bare_stem) {
            return false;
        }
        if explicit || filename_like || bare_stem {
            // explicit / filename.ext / 大写 stem 都算 "locator-strong"；只要有一个就够
            has_explicit_locator = true;
        }
    }
    has_explicit_locator
}

fn prompt_is_inline_json_value(prompt: &str) -> bool {
    let trimmed = prompt.trim();
    !trimmed.is_empty()
        && crate::extract_first_json_value_any(trimmed).is_some_and(|value| value.trim() == trimmed)
}

/// 砍掉常见包裹/标点：反引号 / 括号 / 中英文引号 / 句末标点。
fn trim_locator_punct(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        matches!(
            ch,
            '`' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | '<' | '>'
                | '“' | '”' | '‘' | '’' | '《' | '》' | '【' | '】'
                | '。' | '！' | '？' | '!' | '?' | ':' | '：' | '.'
        )
    })
}

fn is_url_like(token: &str) -> bool {
    token.starts_with("http://")
        || token.starts_with("https://")
        || token.starts_with("file://")
        || token.starts_with("ftp://")
}

/// 形如 `foo.toml` / `Cargo.lock` —— 含 `.` 且后缀是 1-12 位字母数字。
fn looks_like_filename_token(token: &str) -> bool {
    if token.is_empty() {
        return false;
    }
    let Some((base, ext)) = token.rsplit_once('.') else {
        return false;
    };
    if base.is_empty() || ext.is_empty() {
        return false;
    }
    ext.chars().all(|ch| ch.is_ascii_alphanumeric()) && ext.len() <= 12
}

/// 形如 `README` / `LICENSE` —— 全 ASCII 字母数字下划线连字符且包含至少一个大写字母。
/// 中文句子里的 "service"（全小写）不命中，因此不会误吃叙述性句子。
fn looks_like_bare_uppercase_stem(token: &str) -> bool {
    if token.is_empty()
        || token.contains('/')
        || token.contains('\\')
        || token.contains('.')
        || is_url_like(token)
    {
        return false;
    }
    if token.chars().count() < 2 {
        return false;
    }
    if !token
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
    {
        return false;
    }
    token.chars().any(|ch| ch.is_ascii_uppercase())
}

/// V1 入口：判断当前 (prompt, last_turn_full) 是否能走 clarify shortpath。
///
/// 命中条件（全部满足）：
///   1. 上一轮 [clarification_requested]
///   2. 当前 prompt 只贴 locator/path/单文件名（prompt_looks_like_locator_only）
///   3. 上一轮能解析出非空的 User 原话（extract_prior_user_text Some）
///
/// 返回 None 时，调用方按原路径继续走 normalizer。
pub(crate) fn try_clarify_reply_shortpath(
    prompt: &str,
    last_turn_full: &str,
) -> Option<ClarifyShortpathHit> {
    if !last_turn_was_clarify(last_turn_full) {
        return None;
    }
    if !prompt_looks_like_locator_only(prompt) {
        return None;
    }
    let prior_user_text = extract_prior_user_text(last_turn_full)?;
    let current_user_text = prompt.trim().to_string();
    let resolved_intent = format!(
        "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target/content: {}",
        prior_user_text.trim(),
        current_user_text
    );
    Some(ClarifyShortpathHit {
        resolved_intent,
        prior_user_text,
        current_user_text,
        reason: ShortpathReason::ClarifyLocatorReply,
    })
}

/// 命中后构造最小 RouteResult，跳过 normalizer。
///
/// 字段策略（与 direct_classifier_route_result 风格对齐）：
///   - routed_mode = Act：clarify 续答几乎全部是"现在去做 X"，给 chat 兜底既无证据又
///     违反 plan §7.1 verifier；保守拍 Act 让 planner 出 read/run/list/find 计划
///   - needs_clarify = false：明确不再追问
///   - resolved_intent = hit.resolved_intent：拼好的 prior+current 喂给 planner
///   - output_contract = default：§7.1 verifier 出口兜底（典型 case 是 free shape）
///   - 其它字段都按 default / 0 / "" / None，让现有 post_route / auto_locator /
///     guard 链自然接管
pub(crate) fn synthesize_route_result_from_hit(hit: &ClarifyShortpathHit) -> RouteResult {
    RouteResult {
        routed_mode: RoutedMode::Act,
        ask_mode: AskMode::from_routed_mode(RoutedMode::Act),
        resolved_intent: hit.resolved_intent.clone(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: hit.reason.route_reason_text().to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
        direct_reply_candidate: String::new(),
        direct_reply_confidence: 0.0,
    }
}

/// 命中时的 tracing 事件 —— 与 fallback.rs 的结构化字段风格对齐，便于
/// inspect_task.sh / 日志管道按 event name 过滤。
pub(crate) fn emit_shortpath_hit_event(task_id: &str, hit: &ClarifyShortpathHit) {
    tracing::info!(
        task_id = %task_id,
        shortpath_reason = hit.reason.as_metric_label(),
        prior_user_text = %crate::truncate_for_log(&hit.prior_user_text),
        current_user_text = %crate::truncate_for_log(&hit.current_user_text),
        "clarify_shortpath_hit"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn last_turn_with_clarify(prior_user: &str) -> String {
        format!(
            "[LAST_TURN_FULL]\nUser: {}\nAssistant: [clarification_requested]\n",
            prior_user
        )
    }

    fn last_turn_normal(prior_user: &str, prior_assistant: &str) -> String {
        format!(
            "[LAST_TURN_FULL]\nUser: {}\nAssistant: {}\n",
            prior_user, prior_assistant
        )
    }

    #[test]
    fn last_turn_was_clarify_detects_placeholder() {
        assert!(last_turn_was_clarify(&last_turn_with_clarify(
            "看一下那个文件的内容"
        )));
        assert!(!last_turn_was_clarify(&last_turn_normal(
            "你好",
            "你好，需要什么帮助？"
        )));
        assert!(!last_turn_was_clarify(""));
        assert!(!last_turn_was_clarify("<none>"));
    }

    #[test]
    fn extract_prior_user_text_returns_first_user_line() {
        let ctx = last_turn_with_clarify("看一下那个文件 schema version");
        assert_eq!(
            extract_prior_user_text(&ctx).as_deref(),
            Some("看一下那个文件 schema version")
        );
    }

    #[test]
    fn extract_prior_user_text_none_on_empty_or_no_user_line() {
        assert!(extract_prior_user_text("").is_none());
        assert!(extract_prior_user_text("[LAST_TURN_FULL]\nAssistant: 没有 User 行\n").is_none());
        assert!(
            extract_prior_user_text("[LAST_TURN_FULL]\nUser: \nAssistant: x\n").is_none(),
            "空 User 行不应该当成有效 prior"
        );
    }

    #[test]
    fn prompt_looks_like_locator_only_accepts_explicit_path() {
        assert!(prompt_looks_like_locator_only(
            "scripts/nl_tests/fixtures/test_contract.sqlite"
        ));
        assert!(prompt_looks_like_locator_only(
            "/home/guagua/rustclaw/Cargo.toml"
        ));
    }

    #[test]
    fn prompt_looks_like_locator_only_accepts_bare_filename() {
        assert!(prompt_looks_like_locator_only("Cargo.toml"));
        assert!(prompt_looks_like_locator_only("README.md"));
    }

    #[test]
    fn prompt_looks_like_locator_only_rejects_full_sentence() {
        // 一个长描述不像单纯 locator 续答，绝不能命中 shortpath
        assert!(!prompt_looks_like_locator_only(
            "我现在想知道我们项目里有几个 service 文件，你给我列一下"
        ));
        assert!(!prompt_looks_like_locator_only(""));
        assert!(!prompt_looks_like_locator_only("   "));
    }

    #[test]
    fn try_shortpath_hits_when_prior_clarify_and_current_is_path() {
        let last_turn = last_turn_with_clarify(
            "看一下那个 sqlite 文件的 schema version",
        );
        let prompt = "scripts/nl_tests/fixtures/test_contract.sqlite";
        let hit = try_clarify_reply_shortpath(prompt, &last_turn)
            .expect("should hit shortpath when prior clarify + current path");
        assert_eq!(hit.reason, ShortpathReason::ClarifyLocatorReply);
        assert_eq!(hit.current_user_text, prompt);
        assert!(hit.prior_user_text.contains("schema version"));
        assert!(
            hit.resolved_intent.contains("schema version")
                && hit.resolved_intent.contains("test_contract.sqlite"),
            "resolved_intent 必须串起 prior 和 current：{}",
            hit.resolved_intent
        );
    }

    #[test]
    fn try_shortpath_misses_when_prior_was_not_clarify() {
        let last_turn = last_turn_normal("你好", "你好，需要帮助吗？");
        let prompt = "Cargo.toml";
        assert!(
            try_clarify_reply_shortpath(prompt, &last_turn).is_none(),
            "上一轮不是 clarify 时 shortpath 必须 miss，避免误吃新请求"
        );
    }

    #[test]
    fn try_shortpath_misses_when_current_is_full_sentence() {
        let last_turn = last_turn_with_clarify("看一下那个文件");
        let prompt = "请帮我读一下整个 README 然后总结成 5 条要点";
        assert!(
            try_clarify_reply_shortpath(prompt, &last_turn).is_none(),
            "完整描述句不能命中 shortpath，否则跳过 normalizer 后 plan 会乱"
        );
    }

    #[test]
    fn try_shortpath_misses_when_no_prior_user_line() {
        let last_turn = "[LAST_TURN_FULL]\nAssistant: [clarification_requested]\n";
        let prompt = "Cargo.toml";
        assert!(
            try_clarify_reply_shortpath(prompt, last_turn).is_none(),
            "拿不到 prior user text 时不能命中，否则 resolved_intent 会丢上下文"
        );
    }

    #[test]
    fn synthesize_route_result_uses_act_mode_and_clean_clarify_state() {
        let hit = ClarifyShortpathHit {
            resolved_intent: "Continue ...".to_string(),
            prior_user_text: "看一下".to_string(),
            current_user_text: "Cargo.toml".to_string(),
            reason: ShortpathReason::ClarifyLocatorReply,
        };
        let route = synthesize_route_result_from_hit(&hit);
        assert_eq!(route.routed_mode, RoutedMode::Act);
        assert!(!route.needs_clarify);
        assert!(route.clarify_question.is_empty());
        assert_eq!(route.resolved_intent, "Continue ...");
        assert_eq!(
            route.route_reason,
            "clarify_followup_shortpath_locator_reply_v1"
        );
        assert_eq!(route.route_confidence, Some(1.0));
        assert_eq!(route.resume_behavior, ResumeBehavior::None);
        assert_eq!(route.schedule_kind, ScheduleKind::None);
    }

    #[test]
    fn shortpath_reason_metric_label_is_stable() {
        // 一旦发布就不能改 —— metric / log query 会 hard-code 它
        assert_eq!(
            ShortpathReason::ClarifyLocatorReply.as_metric_label(),
            "clarify_locator_reply"
        );
    }
}
