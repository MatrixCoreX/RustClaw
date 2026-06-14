use super::{
    first_clear_request_language_hint, mixed_language_prefers_cjk_response,
    preferred_response_language_hint, request_language_hint, task_language_source_text,
    task_language_source_text_with_active_clarify, task_user_request_for_prompt,
    text_is_language_neutral_artifact_only, text_language_conflicts_with_hint,
};

#[test]
fn request_language_hint_prefers_current_turn_text_shape() {
    assert_eq!(request_language_hint("写个两句短诗"), "zh-CN");
    assert_eq!(
        request_language_hint("do not run anything, just tell me a very short joke"),
        "en"
    );
    assert_eq!(request_language_hint("用 English 解释 README"), "mixed");
    assert_eq!(
        request_language_hint("读取 ./NO_SUCH_RUSTCLAW_TEST_987654.txt 的第一行"),
        "zh-CN"
    );
    assert_eq!(
        request_language_hint("读取 /home/guagua/rustclaw/configs/config.toml 的第一行"),
        "zh-CN"
    );
    assert_eq!(
        request_language_hint("只检查 configs/config.toml 是否是合法 TOML，别做语义风险判断。"),
        "zh-CN"
    );
    assert_eq!(request_language_hint("读取 AGENTS.md 的第一行"), "zh-CN");
    assert_eq!(
        request_language_hint("logs ディレクトリのファイル名を一覧して"),
        "ja"
    );
    assert_eq!(
        request_language_hint("logs 디렉터리의 파일명을 보여줘"),
        "ko"
    );
    assert_eq!(
        request_language_hint("покажи имена файлов в logs"),
        "und-Cyrl"
    );
    assert_eq!(request_language_hint("اكتب ملخصا قصيرا"), "und-Arab");
    assert_eq!(
        request_language_hint("résume le fichier README"),
        "und-Latn"
    );
    assert_eq!(
        request_language_hint("/home/guagua/rustclaw/configs/config.toml"),
        "config_default"
    );
    assert_eq!(
        request_language_hint("configs/app_config.toml"),
        "config_default"
    );
    assert_eq!(request_language_hint("12345"), "config_default");
}

#[test]
fn language_neutral_artifacts_do_not_force_english() {
    assert!(text_is_language_neutral_artifact_only(
        "/home/guagua/rustclaw/configs/config.toml"
    ));
    assert!(text_is_language_neutral_artifact_only(
        "configs/app_config.toml"
    ));
    assert!(text_is_language_neutral_artifact_only("README.md"));
    assert!(!text_is_language_neutral_artifact_only("读取 README.md"));
    assert_eq!(
        first_clear_request_language_hint([
            "/home/guagua/rustclaw/configs/app_config.toml",
            "把那个文件发给我，不要贴内容",
        ])
        .as_deref(),
        Some("zh-CN")
    );
}

#[test]
fn mixed_response_language_preference_uses_script_balance() {
    assert!(!mixed_language_prefers_cjk_response(
        "用 English language answer explain README please"
    ));
    assert!(mixed_language_prefers_cjk_response(
        "读取 README.md 的第一行"
    ));
    assert!(mixed_language_prefers_cjk_response(
        "先执行 echo BEFORE_BREAK，再执行 definitely_missing_command_rustclaw_user_ops_13579，只告诉我哪一步挂了"
    ));
    assert_eq!(
        request_language_hint(
            "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍"
        ),
        "zh-CN"
    );
    assert!(mixed_language_prefers_cjk_response(
        "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍"
    ));
}

#[test]
fn generated_text_language_conflict_allows_embedded_names() {
    assert!(text_language_conflicts_with_hint(
        "请具体说明您想继续什么任务或操作？",
        "en"
    ));
    assert!(!text_language_conflicts_with_hint(
        "Please confirm the target file 新加卷 before I continue.",
        "en"
    ));
    assert!(text_language_conflicts_with_hint(
        "I couldn't determine the requested action.",
        "zh-CN"
    ));
    assert!(text_language_conflicts_with_hint(
        "# Service Notes\n\nRustClaw test fixture service notes.",
        "ko"
    ));
    assert!(text_language_conflicts_with_hint(
        "# Service Notes\n\nRustClaw test fixture service notes.",
        "ja"
    ));
    assert!(!text_language_conflicts_with_hint(
        "서비스 노트의 핵심은 로컬 테스트 서비스 설명입니다.",
        "ko"
    ));
}

#[test]
fn preferred_response_language_hint_falls_back_to_session_locale_when_turn_is_ambiguous() {
    assert_eq!(
        preferred_response_language_hint("continue", Some("en-US")),
        "en"
    );
    assert_eq!(
        preferred_response_language_hint("继续", Some("en-US")),
        "zh-CN"
    );
    assert_eq!(
        preferred_response_language_hint("12345", Some("zh-CN")),
        "zh-CN"
    );
    assert_eq!(
        preferred_response_language_hint("12345", Some("fr-FR")),
        "fr-FR"
    );
    assert_eq!(
        preferred_response_language_hint("12345", Some("ja_JP")),
        "ja-JP"
    );
}

#[test]
fn task_language_source_prefers_original_payload_text_over_runtime_scaffold() {
    let task = crate::ClaimedTask {
        task_id: "task-language-source".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"写一段总结"}).to_string(),
    };
    let scaffolded =
        "Write a summary\n\n[AUTO_LOCATOR]\nResolved present workspace scope to: /tmp/project";
    assert_eq!(
        task_language_source_text(&task, scaffolded).as_ref(),
        "写一段总结"
    );
}

#[test]
fn task_language_source_prefers_explicit_current_text_over_placeholder_payload() {
    let task = crate::ClaimedTask {
        task_id: "task-language-current-text".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"placeholder"}).to_string(),
    };
    assert_eq!(
        task_language_source_text(&task, "明天提醒我检查部署").as_ref(),
        "明天提醒我检查部署"
    );
}

#[test]
fn task_language_source_prefers_original_payload_over_resolved_semantic_rewrite() {
    let task = crate::ClaimedTask {
        task_id: "task-language-original".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({
            "text":"读取 UI/package.json 里的 name，最后只用一行输出：一样或不一样"
        })
        .to_string(),
    };
    let resolved = "Read the name field from UI/package.json and answer same or different.";
    assert_eq!(
        task_language_source_text(&task, resolved).as_ref(),
        "读取 UI/package.json 里的 name，最后只用一行输出：一样或不一样"
    );
}

#[test]
fn task_language_source_uses_active_clarify_source_for_locator_only_reply() {
    let task = crate::ClaimedTask {
        task_id: "task-language-active-clarify".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"telegramd"}).to_string(),
    };
    let source_request = "看看那个服务现在是不是在运行";
    let resolved = "Check if the telegramd service is currently running";
    assert_eq!(
        task_language_source_text_with_active_clarify(&task, resolved, Some(source_request))
            .as_ref(),
        source_request
    );
}

#[test]
fn task_language_source_keeps_current_sentence_language_over_active_clarify_source() {
    let task = crate::ClaimedTask {
        task_id: "task-language-active-clarify-current-sentence".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"please continue with the task"}).to_string(),
    };
    assert_eq!(
        task_language_source_text_with_active_clarify(
            &task,
            "please continue with the task",
            Some("看看那个服务现在是不是在运行"),
        )
        .as_ref(),
        "please continue with the task"
    );
}

#[test]
fn task_user_request_for_prompt_keeps_original_language_and_resolved_semantics() {
    let task = crate::ClaimedTask {
        task_id: "task-request-for-prompt".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({
            "text":"读取 UI/package.json 里的 name，最后只用一行输出：一样或不一样"
        })
        .to_string(),
    };
    let rendered = task_user_request_for_prompt(
        &task,
        "Read the name field from UI/package.json and answer same or different.",
    );
    assert!(rendered.contains("Original user request:"));
    assert!(rendered.contains("一样或不一样"));
    assert!(rendered.contains("Resolved semantic request:"));
    assert!(rendered.contains("Read the name field"));
    assert!(rendered.contains("preserve the original user's language"));
}
