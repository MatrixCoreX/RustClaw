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
fn prompt_is_structural_locator_only_accepts_explicit_path() {
    assert!(prompt_is_structural_locator_only(
        "scripts/nl_tests/fixtures/test_contract.sqlite"
    ));
    assert!(prompt_is_structural_locator_only(
        "/home/guagua/rustclaw/Cargo.toml"
    ));
}

#[test]
fn prompt_is_structural_locator_only_accepts_bare_filename() {
    assert!(prompt_is_structural_locator_only("Cargo.toml"));
    assert!(prompt_is_structural_locator_only("README.md"));
}

#[test]
fn prompt_is_structural_locator_only_rejects_full_sentence() {
    // 一个长描述不像单纯 locator 续答，绝不能命中 rewrite
    assert!(!prompt_is_structural_locator_only(
        "我现在想知道我们项目里有几个 service 文件，你给我列一下"
    ));
    assert!(!prompt_is_structural_locator_only(""));
    assert!(!prompt_is_structural_locator_only("   "));
}

#[test]
fn try_locator_reply_rewrite_hits_when_prior_clarify_and_current_is_path() {
    let last_turn = last_turn_with_clarify("看一下那个 sqlite 文件的 schema version");
    let prompt = "scripts/nl_tests/fixtures/test_contract.sqlite";
    let hit = try_clarify_reply_rewrite(prompt, &last_turn)
        .expect("should rewrite when prior clarify + current path");
    assert_eq!(hit.reason, ClarifyRewriteReason::ClarifyLocatorReply);
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
fn try_locator_reply_rewrite_misses_when_prior_was_not_clarify() {
    let last_turn = last_turn_normal("你好", "你好，需要帮助吗？");
    let prompt = "Cargo.toml";
    assert!(
        try_clarify_reply_rewrite(prompt, &last_turn).is_none(),
        "上一轮不是 clarify 时 rewrite 必须 miss，避免误吃新请求"
    );
}

#[test]
fn try_locator_reply_rewrite_misses_when_current_is_full_sentence() {
    let last_turn = last_turn_with_clarify("看一下那个文件");
    let prompt = "请帮我读一下整个 README 然后总结成 5 条要点";
    assert!(
        try_clarify_reply_rewrite(prompt, &last_turn).is_none(),
        "完整描述句不能命中 rewrite，否则 normalizer 输入会被错误合并"
    );
}

#[test]
fn try_locator_reply_rewrite_misses_when_no_prior_user_line() {
    let last_turn = "[LAST_TURN_FULL]\nAssistant: [clarification_requested]\n";
    let prompt = "Cargo.toml";
    assert!(
        try_clarify_reply_rewrite(prompt, last_turn).is_none(),
        "拿不到 prior user text 时不能命中，否则 resolved_intent 会丢上下文"
    );
}

#[test]
fn clarify_rewrite_reason_metric_label_is_stable() {
    // 一旦发布就不能改 —— metric / log query 会 hard-code 它
    assert_eq!(
        ClarifyRewriteReason::ClarifyLocatorReply.as_metric_label(),
        "clarify_locator_reply"
    );
}
