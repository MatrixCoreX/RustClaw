// Normalizer schema guard and coercion tests for intent_router.

use super::{OutputDeliveryIntent, OutputResponseShape, OutputSemanticKind};

#[test]
fn normalizer_schema_normalization_recovers_minimax_file_list_search_payload() {
    let raw = r#"{
          "resolved_user_intent":"列出 document 目录中所有 .md 文件，排除 README，返回剩余的 .md 文件列表",
          "answer_candidate":[],
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"directory listing",
          "confidence":0.7,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"list","semantic_kind":"file_list"},
          "execution_recipe":"list_md_files_excluding_readme",
          "turn_type":"file_query",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录里所有 .md 文件，但排除 README，告诉我还剩哪些",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("strict")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("file_names")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_infer_malformed_recipe_array() {
    let raw = r#"{
          "resolved_user_intent":"用户请求读取 README 文件的开头内容，并用通俗易懂的非技术语言进行一句话总结",
          "answer_candidate":"",
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"need file evidence",
          "confidence":0.78,
          "decision":"direct_answer",
          "output_contract":"summary",
          "execution_recipe":["READ","SUMMARIZE"],
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "读取 README.md 的开头，用一句非技术语言总结",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_infer_string_recipe_locator() {
    let raw = r#"{
          "resolved_user_intent":"读取 README.md 开头部分并用非技术语言一句话总结其核心含义",
          "answer_candidate":"",
          "resume_behavior":"continue",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"Assistant",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"clear file read and summary request",
          "confidence":0.92,
          "decision":"direct_answer",
          "output_contract":"非技术用户可理解的一句话总结",
          "execution_recipe":"read_file:/home/guagua/rustclaw/README.md,line_range:[1-50],summarize:one_sentence_non_technical",
          "turn_type":"execute",
          "target_task_policy":"read_and_summarize",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "读取 README 开头内容，再用非技术用户能听懂的话做一句总结",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|v| v.as_str()),
        Some("")
    );
}

#[test]
fn malformed_recipe_array_without_locator_stays_chat() {
    let raw = r#"{
          "resolved_user_intent":"总结刚才的对话",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"chat summary",
          "confidence":0.8,
          "decision":"direct_answer",
          "output_contract":"summary",
          "execution_recipe":["SUMMARIZE"]
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "总结刚才的对话");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn parse_output_contract_clears_inconsistent_inline_delivery_flag() {
    let contract = super::parse_output_contract(
        Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: "path".to_string(),
            delivery_intent: "response".to_string(),
            contract_marker: String::new(),
            semantic_kind: "file_names".to_string(),
            locator_hint: "logs".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        false,
    );

    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
}

#[test]
fn parse_output_contract_clears_file_delivery_for_inline_raw_output_contract() {
    let contract = super::parse_output_contract(
        Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: true,
            locator_kind: "path".to_string(),
            delivery_intent: "file_single".to_string(),
            contract_marker: String::new(),
            semantic_kind: "raw_command_output".to_string(),
            locator_hint: "/tmp/app.log".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        false,
    );

    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
}

#[test]
fn parse_output_contract_preserves_exact_sentence_count_as_strict_contract() {
    let contract = super::parse_output_contract(
        Some(super::IntentOutputContractOut {
            response_shape: "one_sentence".to_string(),
            exact_sentence_count: Some(serde_json::json!(3)),
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            contract_marker: String::new(),
            semantic_kind: "content_excerpt_summary".to_string(),
            locator_hint: "/tmp/report.md".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        false,
    );

    assert_eq!(contract.exact_sentence_count, Some(3));
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
    );
}

#[test]
fn parse_output_contract_preserves_list_selector_contract() {
    let contract = super::parse_output_contract(
        Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            contract_marker: String::new(),
            semantic_kind: "file_names".to_string(),
            locator_hint: "logs".to_string(),
            scalar_count_filter: Some(serde_json::json!(0)),
            list_selector: Some(serde_json::json!({
                "target_kind": "file",
                "limit": "3",
                "sort_by": "size_desc",
                "include_metadata": true,
                "include_hidden": true
            })),
            self_extension: None,
        }),
        false,
    );

    let selector = contract.self_extension.list_selector;
    assert_eq!(
        selector.target_kind,
        crate::OutputScalarCountTargetKind::File
    );
    assert!(selector.target_kind_specified);
    assert_eq!(selector.limit, Some(3));
    assert_eq!(selector.sort_by.as_deref(), Some("size_desc"));
    assert_eq!(selector.include_metadata, Some(true));
    assert_eq!(selector.include_hidden, Some(true));
}

#[test]
fn normalizer_schema_normalization_coerces_string_bool_contract_fields() {
    let raw = r#"{"resolved_user_intent":"列出 document 目录文件名","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"strict","requires_content_evidence":"true","delivery_required":"filename_list","locator_kind":"path","delivery_intent":"返回 document 目录下的文件名列表","semantic_kind":"filename_list","locator_hint":"document"}}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录下有哪些文件，只输出文件名列表",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/delivery_required")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("file_names")
    );
}

#[test]
fn normalizer_schema_normalization_does_not_infer_custom_recipe_text() {
    let raw = r#"{
          "resolved_user_intent":"列出 document 目录下的所有文件名，仅输出文件名列表。",
          "answer_candidate":"",
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw测试助手",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"User explicitly requests file listing for the 'document' directory.",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":"list_of_strings",
          "execution_recipe":"list_files(directory='document', include_subdirs=False)",
          "turn_type":"command",
          "target_task_policy":"list_files",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录下有哪些文件，只输出文件名列表",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("strict")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_infer_shell_file_listing_recipe_text() {
    let raw = r#"{
          "resolved_user_intent":"列出 document 目录下所有文件的文件名列表",
          "answer_candidate":"",
          "resume_behavior":false,
          "schedule_kind":"immediate",
          "schedule_intent":"execute",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户明确要求列出 document 目录下的文件列表",
          "confidence":"high",
          "decision":"direct_answer",
          "output_contract":"text",
          "execution_recipe":"执行 ls -1 document/ 获取文件名列表",
          "turn_type":"act",
          "target_task_policy":"browse_local_fs",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录下有哪些文件，只输出文件名列表",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_infer_hidden_entries_recipe_text() {
    let raw = r#"{
          "resolved_user_intent":"检查当前工作目录是否存在隐藏文件，回答有或没有，并提供3个具体例子",
          "answer_candidate":"",
          "resume_behavior":"",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"clear local filesystem observation",
          "confidence":0.92,
          "decision":"planner_execute",
          "output_contract":"",
          "execution_recipe":"list_hidden_files",
          "turn_type":"task",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_infer_hidden_entries_recipe_array() {
    let raw = r#"{
          "resolved_user_intent":"检查当前工作目录是否存在隐藏文件，只回答有或没有，并提供3个具体例子",
          "answer_candidate":"有",
          "needs_clarify":false,
          "reason":"local filesystem observation",
          "confidence":0.92,
          "decision":"planner_execute",
          "output_contract":"json",
          "execution_recipe":[
            "ls -a /home/guagua/rustclaw | grep '^\\.'",
            "Check if any hidden files exist",
            "Return answer '有' and examples: .git, .gitignore, .rustfmt.toml"
          ]
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_keeps_structured_contract_over_recipe_text() {
    let raw = r#"{
          "resolved_user_intent":"检查当前目录 /home/guagua/rustclaw 是否有隐藏文件（以点开头的文件），若存在则回答“有”并提供3个示例",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"requires actual directory observation",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{"response_shape":"strict","contract_marker":"existence_with_path","requires_content_evidence":true},
          "execution_recipe":{
            "command":"ls -la /home/guagua/rustclaw | grep '^\\.' | head -3",
            "action_type":"list_hidden_files"
          },
          "turn_type":"task",
          "target_task_policy":"check_hidden_entries"
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查当前目录有没有隐藏文件，只回答有或没有，并补 3 个例子",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("existence_with_path")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_maps_command_payload_to_raw_output_when_semantic_missing() {
    let raw = r#"{
          "resolved_user_intent":"列出 logs 目录下前 10 个文件名，不读取内容",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"local command can list directory entries",
          "confidence":0.91,
          "decision":"planner_execute",
          "output_contract":null,
          "execution_recipe":{
            "action":"local_exec",
            "command":"ls -1 logs/ | head -n 10",
            "working_dir":"/home/guagua/rustclaw"
          }
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "列出 logs 目录下的前 10 个文件名");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_maps_health_check_tool_recipe_to_service_status() {
    let raw = r#"{
          "resolved_user_intent":"Run a basic health check on the current workspace and summarize findings",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"structured health check observation",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":"/home/guagua/rustclaw"
          },
          "execution_recipe":{
            "kind":"tool",
            "tool":"health_check",
            "requires_evidence":true
          }
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "run a basic health check here and summarize only the most important findings",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("service_status")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_service_status_observation"));
    assert!(report
        .detail_csv()
        .contains("execution_recipe_health_check_observation"));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_detects_nested_command_payload() {
    let raw = r#"{
          "resolved_user_intent":"Execute whoami and pwd commands, then reply with both outputs combined with a self-deprecating signature.",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"sequential command execution",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"combined command output",
          "execution_recipe":[
            {"step":"run_whoami","command":"whoami","capture":true},
            {"step":"run_pwd","command":"pwd","capture":true},
            {"step":"compose_response","template":"{whoami} {pwd}"}
          ],
          "turn_type":"execution",
          "target_task_policy":"default_execution"
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "先执行 whoami，再执行 pwd，然后把结果用一句自嘲签名回复我",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_command_payload"));
}

#[test]
fn normalizer_schema_normalization_maps_legacy_command_result_contract_to_raw_output() {
    let raw = r#"{
          "resolved_user_intent":"执行pwd命令获取当前工作目录",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"local command execution",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"command_execution_result",
          "execution_recipe":{
            "command":"pwd",
            "capture_stdout":true
          }
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "请执行 pwd，只输出命令结果");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_clears_empty_path_locator_for_command_payload() {
    let raw = r#"{
          "resolved_user_intent":"Run a local shell command and summarize the command output.",
          "answer_candidate":"",
          "resume_behavior":"",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"A local command payload supplies the evidence source.",
          "confidence":0.95,
          "decision":"planner_execute",
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"path",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":"",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"run_cmd","command":"df -h"},
          "turn_type":"task_request",
          "target_task_policy":"new",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "Run a local shell command and summarize the output",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|v| v.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_recovers_tool_payload_as_execution_signal() {
    let raw = r#"{
          "action": "search_files",
          "args": {
            "pattern": "README.md",
            "search_path": ".",
            "max_results": 1
          }
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "检查 README.md 是否存在，并只用 JSON 返回 path 和 size_bytes 两个字段。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert!(report.source_csv().contains("tool_payload"));
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_infer_check_file_recipe_text() {
    let raw = r#"{
          "resolved_user_intent":"检查仓库中是否存在 rustclaw.service 文件",
          "answer_candidate":"没有",
          "needs_clarify":false,
          "reason":"repo inspection",
          "confidence":0.87,
          "decision":"direct_answer",
          "output_contract":"strict",
          "execution_recipe":"check_file"
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "检查仓库里有没有 rustclaw.service");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("strict")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_infer_shell_find_recipe_text() {
    let raw = r#"{
          "resolved_user_intent":"检查仓库目录是否存在 rustclaw.service 文件，报告有或没有，并给出找到的路径",
          "answer_candidate":"有: /home/guagua/rustclaw/rustclaw.service",
          "needs_clarify":false,
          "reason":"file existence check",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"raw_text",
          "execution_recipe":"find /home/guagua/rustclaw -name 'rustclaw.service' 2>/dev/null",
          "turn_type":"ask",
          "target_task_policy":"filesystem_existence"
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_recover_semantic_kind_from_string_contract() {
    let raw = r#"{
          "resolved_user_intent":"列出 document 目录下的文件名",
          "needs_clarify":false,
          "reason":"local file listing",
          "confidence":0.9,
          "decision":"direct_answer",
          "output_contract":"file_names_only",
          "execution_recipe":"ls document/"
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录下有哪些文件，只输出文件名列表",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_maps_directory_entry_names_to_entry_groups() {
    let raw = r#"{
          "resolved_user_intent":"list direct child entry names for docs",
          "needs_clarify":false,
          "reason":"directory inventory",
          "confidence":0.9,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"path",
            "delivery_intent":"none",
            "semantic_kind":"directory_entry_names",
            "locator_hint":"docs"
          }
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "list direct child names for docs");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("directory_entry_groups")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_repair_file_listing_from_recipe_text() {
    let raw = r#"{
          "resolved_user_intent":"列出 /home/guagua/rustclaw/document 目录下的所有文件名",
          "answer_candidate":null,
          "schedule_kind":"immediate",
          "needs_clarify":false,
          "reason":"用户明确请求列出 document 目录下的文件，目标清晰，属于简单文件列表操作",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"scalar","semantic_kind":"scalar_path_only","requires_content_evidence":false},
          "execution_recipe":"LIST_FILES",
          "turn_type":"act"
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录下有哪些文件，只输出文件名列表",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("scalar")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("scalar_path_only")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_repairs_exact_format_scalar_comparison_contract() {
    let raw = r#"{"resolved_user_intent":"读取 UI/package.json 的 name 字段和 crates/clawd/Cargo.toml 的 package.name 字段，对比后单行输出：{UI名}, {Cargo名}, {一样|不一样}","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"一行字符串，格式为：{UI_name}, {Cargo_name}, {一样|不一样}","requires_content_evidence":false,"delivery_required":true,"locator_kind":"none","delivery_intent":"直接返回对比结果","semantic_kind":"key_value_comparison","locator_hint":""}}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，最后只用一行输出：前者、后者、一样或不一样",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("strict")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("recent_scalar_equality_check")
    );
    assert_eq!(
        value
            .pointer("/output_contract/delivery_intent")
            .and_then(|v| v.as_str()),
        Some("none")
    );

    let raw = r#"{"resolved_user_intent":"比较UI/package.json的name字段与crates/clawd/Cargo.toml的package.name字段，输出一行格式：<UI名>, <Cargo名>, <一样|不一样>","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"一行文字","requires_content_evidence":false,"delivery_required":"执行文件系统读取后输出单行结果","locator_hint":"UI/package.json, crates/clawd/Cargo.toml"}}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，最后只用一行输出：前者、后者、一样或不一样",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        value
            .pointer("/output_contract/delivery_required")
            .and_then(|v| v.as_bool()),
        Some(false)
    );

    let raw = r#"{"resolved_user_intent":"compare two names","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"single line string","requires_content_evidence":false,"delivery_required":true,"locator_kind":"file","delivery_intent":"comparison result line","semantic_kind":"comparison","locator_hint":"UI/package.json crates/clawd/Cargo.toml"}}"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "compare two fields in one line");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("free")
    );
}

#[test]
fn normalizer_schema_normalization_coerces_non_object_self_extension_contract() {
    let raw = r#"{
          "resolved_user_intent": "用户请求一个 harmless chat deliverable",
          "needs_clarify": false,
          "clarify_question": "",
          "reason": "task is clear",
          "confidence": "high",
          "decision":"direct_answer",
          "output_contract": {
            "response_shape": "free",
            "requires_content_evidence": false,
            "delivery_required": "deliverable content",
            "locator_kind": "none",
            "delivery_intent": "provide answer",
            "semantic_kind": "entertainment",
            "locator_hint": "no locator is needed",
            "self_extension": "not requested"
          }
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "tell me something short");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value.get("needs_clarify").and_then(|value| value.as_bool()),
        Some(false)
    );
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract
            .get("delivery_required")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        contract
            .get("semantic_kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        contract
            .get("locator_hint")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        contract
            .get("self_extension")
            .and_then(|value| value.get("mode"))
            .and_then(|value| value.as_str()),
        Some("none")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_coerces_model_contract_synonyms() {
    let raw = r#"{
          "resolved_user_intent": {"action":"find_file","target":"rustclaw.service","scope":"repository"},
          "needs_clarify": false,
          "reason": "repo inspection",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": {
            "response_shape": "inline",
            "semantic_kind": "existence_boolean_with_path",
            "locator_kind": "repository",
            "delivery_intent": "list_directory",
            "extra_model_field": "ignored"
          },
          "action": "find_file"
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "检查仓库里有没有 rustclaw.service");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let resolved_value: serde_json::Value = serde_json::from_str(
        value
            .get("resolved_user_intent")
            .and_then(|v| v.as_str())
            .expect("resolved intent string"),
    )
    .expect("resolved intent json");
    assert_eq!(
        resolved_value
            .get("target")
            .and_then(|value| value.as_str()),
        Some("rustclaw.service")
    );
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("strict")
    );
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("existence_with_path")
    );
    assert_eq!(
        contract.get("locator_kind").and_then(|v| v.as_str()),
        Some("current_workspace")
    );
    assert!(!contract.contains_key("extra_model_field"));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_repairs_current_workspace_path_alias_contract() {
    let raw = r#"{
          "resolved_user_intent":"只输出当前工作目录的绝对路径，不要解释",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"plain_text",
            "semantic_kind":"filesystem_locator",
            "locator_kind":"directory_path",
            "locator_hint":"current_working_directory",
            "delivery_intent":"show",
            "requires_content_evidence":false,
            "delivery_required":"current_working_directory",
            "self_extension":"none"
          }
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "只输出当前工作目录的绝对路径，不要解释",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("scalar")
    );
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("scalar_path_only")
    );
    assert_eq!(
        contract.get("locator_kind").and_then(|v| v.as_str()),
        Some("current_workspace")
    );
    assert_eq!(
        contract.get("locator_hint").and_then(|v| v.as_str()),
        Some("")
    );
    assert_eq!(
        contract.get("delivery_required").and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_coerces_null_locator_hint() {
    let raw = r#"{
          "turn_type": "standalone",
          "needs_clarify": false,
          "decision":"direct_answer",
          "output_contract": {
            "response_shape": "scalar",
            "semantic_kind": "none",
            "requires_content_evidence": false,
            "delivery_required": false,
            "locator_hint": null,
            "locator_kind": "none",
            "self_extension": {"mode":"none","trigger":"none","execute_now":false},
            "delivery_intent": "none"
          },
          "schedule_kind": null,
          "execution_recipe": {
            "kind":"none",
            "profile": null,
            "target_scope":"none",
            "repair_policies":[]
          },
          "resolved_user_intent": "输出测试编号 mimo-small-20260429_203108。",
          "clarify_question": null
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "只回答测试编号");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract
            .get("locator_hint")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .get("clarify_question")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value.get("schedule_kind").and_then(|value| value.as_str()),
        Some("none")
    );
    let recipe = value
        .get("execution_recipe")
        .and_then(|value| value.as_object())
        .expect("execution recipe");
    assert_eq!(
        recipe.get("profile").and_then(|value| value.as_str()),
        Some("none")
    );
    assert!(!recipe.contains_key("repair_policies"));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_coerces_none_schedule_intent_string() {
    let raw = r#"{
          "resolved_user_intent":"用户想获取刚才记住的测试编号 RC-CONT-CN-0428-A",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户请求已记住的编号",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":"text",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false,
          "answer":"RC-CONT-CN-0428-A"
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "刚才让你记住的连续测试编号是什么？只回答编号。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert!(value
        .get("schedule_intent")
        .is_some_and(|value| value.is_null()));
    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .get("resolved_user_intent")
            .and_then(|value| value.as_str()),
        Some("用户想获取刚才记住的测试编号 RC-CONT-CN-0428-A")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_drops_none_schedule_intent_object_with_string_nested_fields() {
    let raw = r#"{
          "resolved_user_intent":"logs ディレクトリのファイル名を3つだけ一覧して。",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":{
            "kind":"none",
            "timezone":"",
            "schedule":"",
            "task":"",
            "target_job_id":"",
            "raw":"",
            "reason":"",
            "needs_clarify":false,
            "clarify_question":"",
            "confidence":0.0
          },
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "semantic_kind":"file_names",
            "locator_hint":"logs",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"unknown"},
          "turn_type":"task_request",
          "target_task_policy":"standalone",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert!(value
        .get("schedule_intent")
        .is_some_and(|value| value.is_null()));
    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .get("output_contract")
            .and_then(|value| value.get("semantic_kind"))
            .and_then(|value| value.as_str()),
        Some("file_names")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_preserves_create_schedule_intent_task_text() {
    let raw = r#"{
          "resolved_user_intent":"Create a daily reminder in the current conversation.",
          "resume_behavior":"none",
          "schedule_kind":"create",
          "schedule_intent":{
            "kind":"create",
            "timezone":"Asia/Shanghai",
            "schedule":{"type":"daily","run_at":"","time":"08:00","weekday":1,"every_minutes":0,"cron":""},
            "task":"daily reminder message",
            "target_job_id":null,
            "raw":"Create a daily reminder in the current conversation.",
            "reason":"",
            "needs_clarify":false,
            "clarify_question":"",
            "confidence":0.95
          },
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"schedule fields are complete",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{"response_shape":"one_sentence","requires_content_evidence":false,"delivery_required":false,"locator_kind":"none","delivery_intent":"none","semantic_kind":"none","locator_hint":"","self_extension":{"mode":"none","trigger":"none","execute_now":false}},
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"unknown"},
          "turn_type":"task_request",
          "target_task_policy":"standalone",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "");
    let parsed = crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("normalized schedule intent should validate")
    .value;
    let intent = parsed
        .schedule_intent
        .expect("create schedule intent should be preserved");
    assert_eq!(intent.target_job_id, "");
    assert_eq!(intent.task.kind, "ask");
    assert_eq!(
        intent
            .task
            .payload
            .get("message")
            .and_then(|value| value.as_str()),
        Some("daily reminder message")
    );
}

#[test]
fn normalizer_schema_normalization_discards_scalar_output_contract_answer_candidate() {
    let raw = r#"{
          "resolved_user_intent":"查询之前记住的测试编号",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户请求回忆之前存储的信息",
          "confidence":1.0,
          "decision":"direct_answer",
          "output_contract":"client-like-continuous-20260430_094246",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "刚才我让你记住的测试编号是什么？只回答编号。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .get("answer_candidate")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("free")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_treats_mime_output_contract_as_schema_token() {
    let raw = r#"{
          "resolved_user_intent":"修改目标用户为开发者，仅输出修正后的正文",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"direct chat output",
          "confidence":0.95,
          "decision":"direct_answer",
          "output_contract":"text/plain",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "不对，目标用户改成开发者，不是老板。只输出修正后的正文。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .get("answer_candidate")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("free")
    );
}

#[test]
fn normalizer_schema_normalization_discards_object_answer_candidate_and_ignores_json_contract() {
    let raw = r#"{
          "resolved_user_intent":"retrieve test ID",
          "answer_candidate":{"content":"client-like-continuous-20260430_095834"},
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"User asked for the stored test ID, which is available in short-term memory.",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":"json",
          "execution_recipe":{"kind":"none"},
          "turn_type":"memory",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "刚才我让你记住的测试编号是什么？只回答编号。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .get("answer_candidate")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("free")
    );
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_recovers_command_output_contract_with_unknown_recipe_kind() {
    let raw = r#"{
          "resolved_user_intent":"execute pwd command to get current working directory",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"User explicitly requests direct command execution with raw output, no summary.",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":"raw",
          "execution_recipe":{"kind":"shell","command":"pwd","requires_content_evidence":false,"locator_kind":"none"},
          "turn_type":"task_request",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "执行 pwd，直接输出命令结果，不要总结",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|value| value.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/execution_recipe/kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .get("answer_candidate")
            .and_then(|value| value.as_str()),
        Some("")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_keeps_chat_raw_contract_non_executing() {
    let raw = r#"{
          "resolved_user_intent":"User wants a very short joke and explicitly requests no execution.",
          "answer_candidate":"Why don't scientists trust atoms? Because they make up everything.",
          "resume_behavior":null,
          "schedule_kind":null,
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"MiniMax-M2.1",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"User explicitly requested a joke and no execution.",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":"raw",
          "execution_recipe":null,
          "turn_type":"chat",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "do not run anything, just tell me a very short joke",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_trusts_explicit_none_recipe_for_skill_plan() {
    let raw = r#"{
          "resolved_user_intent":"只生成一个全新可复用技能方案，不执行、不启用。",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"纯设计交付物；execution_recipe.kind=none。",
          "confidence":0.92,
          "decision":"direct_answer",
          "output_contract":{
            "response_shape":"strict",
            "requires_content_evidence":false,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":"",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none","profile":"skill_authoring","target_scope":"none"},
          "turn_type":"task_request",
          "target_task_policy":"standalone",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "请为一个 action=ping 的全新可复用能力生成技能方案，先不要执行，也不要启用。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/execution_recipe/kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/execution_recipe/profile")
            .and_then(|value| value.as_str()),
        Some("skill_authoring")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}
