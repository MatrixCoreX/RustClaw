use super::*;
use std::collections::HashSet;

fn test_state(locale: &str, schedule_locale: &str) -> AppState {
    let mut state = AppState::test_default_with_fixture_provider();
    state.policy.command_intent.default_locale = locale.to_string();
    state.policy.schedule.locale = schedule_locale.to_string();
    state.policy.schedule.i18n_dict.clear();
    state
}

#[test]
fn render_clarify_fallback_follows_language_hint_before_config_locale() {
    let state = test_state("en-US", "en-US");
    let text = render_clarify_fallback_with_language_hint(
        &state,
        "task-test",
        ClarifyFallbackSource::IntentUnresolved,
        None,
        "zh-CN",
    );
    assert!(text.contains("我没看出这条消息要做什么"));

    let state = test_state("zh-CN", "zh-CN");
    let text = render_clarify_fallback_with_language_hint(
        &state,
        "task-test",
        ClarifyFallbackSource::IntentUnresolved,
        None,
        "en",
    );
    assert!(text.contains("I couldn't determine the requested action"));
}

#[test]
fn missing_file_delivery_response_text_keeps_locator_hint() {
    let state = test_state("en-US", "en-US");
    let text = missing_file_delivery_response_text_for_language(
        &state,
        "zh-CN",
        Some("definitely_missing_named_file.txt"),
    );
    assert!(text.contains("definitely_missing_named_file.txt"));
    assert!(text.contains("未找到"));

    let state = test_state("zh-CN", "zh-CN");
    let text = missing_file_delivery_response_text_for_language(
        &state,
        "en",
        Some("/tmp/definitely-missing.txt"),
    );
    assert!(text.contains("/tmp/definitely-missing.txt"));
    assert!(text.contains("File not found"));
}

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
fn one_short_clarification_shape_rejects_multiline_composer_output() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        ..UserResponseContract::default()
    };

    assert!(user_response_contract_local_shape_satisfied(
        &contract,
        "请提供这个配置文件的完整路径。"
    ));
    assert!(!user_response_contract_local_shape_satisfied(
        &contract,
        "我会遵循当前规则来回答。\n\n请问你接下来希望我处理什么具体任务？"
    ));
}

#[test]
fn one_short_clarification_shape_rejects_overlong_composer_output() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        ..UserResponseContract::default()
    };
    let long_reply = "请补充目标。".repeat(30);

    assert!(!user_response_contract_local_shape_satisfied(
        &contract,
        &long_reply
    ));
}

#[test]
fn one_short_clarification_local_shape_leaves_generic_meta_to_llm_validator() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        original_user_request: "查一下那个 sqlite 里有哪些表".to_string(),
        ..UserResponseContract::default()
    };

    assert!(user_response_contract_local_shape_satisfied(
        &contract,
        "请提供 sqlite 数据库的完整路径。"
    ));
    assert!(user_response_contract_local_shape_satisfied(
        &contract,
        "我已准备好处理后续任务，请告诉我接下来需要我做什么。"
    ));
}

#[test]
fn one_short_clarification_local_shape_leaves_false_claims_to_llm_validator() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        original_user_request: "读一下那个 README 开头 3 行".to_string(),
        ..UserResponseContract::default()
    };

    assert!(user_response_contract_local_shape_satisfied(
        &contract,
        "请问你指的是哪个目录下的 README 文件？我目前没有直接访问你本地文件系统的权限。"
    ));
}

#[test]
fn one_short_clarification_shape_accepts_deictic_missing_target_question() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        original_user_request: "把它发给我".to_string(),
        ..UserResponseContract::default()
    };

    assert!(user_response_contract_local_shape_satisfied(
        &contract,
        "请问您想发送哪个文件或内容？请提供具体的文件名或路径。"
    ));
}

#[test]
fn user_response_contract_local_shape_rejects_internal_trace() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        ..UserResponseContract::default()
    };

    assert!(!user_response_contract_local_shape_satisfied(
        &contract,
        "fallback_source=intent_unresolved，请补充目标。"
    ));
}

#[test]
fn user_response_contract_validator_rejects_high_confidence_false_claims() {
    let contract = UserResponseContract {
        response_shape: "brief_failure_with_next_step".to_string(),
        ..UserResponseContract::default()
    };
    let validation = UserResponseContractValidationOut {
        satisfies_contract: true,
        false_claims: true,
        asks_for_missing_target: false,
        mentions_internal_details: false,
        confidence: 0.9,
        reason: "claims unavailable access".to_string(),
    };

    assert!(!user_response_contract_validation_accepts(
        &contract,
        &validation
    ));
}

#[test]
fn user_response_contract_validator_rejects_clarification_without_missing_target() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        ..UserResponseContract::default()
    };
    let validation = UserResponseContractValidationOut {
        satisfies_contract: true,
        false_claims: false,
        asks_for_missing_target: false,
        mentions_internal_details: false,
        confidence: 0.86,
        reason: "generic follow-up".to_string(),
    };

    assert!(!user_response_contract_validation_accepts(
        &contract,
        &validation
    ));
}

#[test]
fn user_response_contract_validator_low_confidence_fails_open() {
    let contract = UserResponseContract {
        response_shape: "one_short_clarification".to_string(),
        ..UserResponseContract::default()
    };
    let validation = UserResponseContractValidationOut {
        satisfies_contract: false,
        false_claims: true,
        asks_for_missing_target: false,
        mentions_internal_details: true,
        confidence: 0.2,
        reason: "uncertain".to_string(),
    };

    assert!(user_response_contract_validation_accepts(
        &contract,
        &validation
    ));
}

#[test]
fn user_response_contract_validator_schema_drift() {
    const SCHEMA_RAW: &str =
        include_str!("../../../prompts/schemas/user_response_contract_validator.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("schema JSON should parse");
    let props = schema
        .get("properties")
        .and_then(|value| value.as_object())
        .expect("schema should define object properties");
    for field in [
        "satisfies_contract",
        "false_claims",
        "asks_for_missing_target",
        "mentions_internal_details",
        "confidence",
        "reason",
    ] {
        assert!(props.contains_key(field), "missing schema field {field}");
    }

    let raw = r#"{"satisfies_contract":true,"false_claims":false,"asks_for_missing_target":true,"mentions_internal_details":false,"confidence":0.9,"reason":"ok"}"#;
    crate::prompt_utils::validate_against_schema::<UserResponseContractValidationOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::UserResponseContractValidator,
    )
    .expect("schema should validate typed contract validator output");
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
        assert!(!src.default_zh().trim().is_empty(), "source={src:?}");
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
    assert!(
        texts
            .iter()
            .any(|t| t == LEGACY_SUPER_FALLBACK_DEFAULT_ZH.trim()),
        "legacy zh default text missing from {texts:?}"
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
        // is_known 同时识别模板和空 context 渲染后的结果。
        let text = lookup_or_default(&dict, src.i18n_key(), src.default_en());
        assert!(
            is_known_clarify_fallback_text_with_dict(&dict, &text),
            "source={src:?} text={text:?} not recognized by is_known"
        );
        let zh_text = src.default_zh().replace("{context_hint}", "");
        assert!(
            is_known_clarify_fallback_text_with_dict(&dict, &zh_text),
            "source={src:?} zh_text={zh_text:?} not recognized by is_known"
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
