// Additional normalizer schema recovery tests for intent_router.

#[test]
fn normalizer_schema_normalization_coerces_hidden_files_check_synonym() {
    let raw = r#"{
          "resolved_user_intent": "检查当前目录是否存在隐藏文件并提供3个示例",
          "needs_clarify": false,
          "reason": "local hidden entries check",
          "confidence": 1.0,
          "decision":"planner_execute",
          "output_contract": {
            "response_shape": "object",
            "semantic_kind": "hidden_files_check"
          }
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
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("hidden_entries_check")
    );
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("strict")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_recover_filename_listing_from_string_contract() {
    let raw = r#"{
          "resolved_user_intent":"List first 10 filenames in logs directory without reading content",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"directory listing action",
          "confidence":0.98,
          "decision":"planner_execute",
          "output_contract":"filename_listing",
          "execution_recipe":{"bash":"ls -1 logs/ | head -10"}
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 logs 目录下的前 10 个文件名，不要读内容",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value.get("answer_candidate").and_then(|v| v.as_str()),
        Some("")
    );
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("none")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_preserves_list_selector_when_neighbor_field_is_malformed() {
    let raw = r#"{
          "resolved_user_intent":"列出 logs 目录下最大的 3 个文件，按大小从大到小，输出「文件名 大小」格式",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"metadata-ranked file listing",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "exact_sentence_count":0,
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"path",
            "delivery_intent":"none",
            "semantic_kind":"file_names",
            "locator_hint":"logs",
            "scalar_count_filter":0,
            "list_selector":{"target_kind":"file","limit":3,"sort_by":"size_desc","include_metadata":true,"include_hidden":false},
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none"},
          "turn_type":"task_request",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 logs 目录下最大的 3 个文件，按大小从大到小，输出\"文件名 大小\"",
    );
    let validated = crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation")
    .value;
    let contract = super::parse_output_contract(validated.output_contract, false);

    assert_eq!(contract.semantic_kind, crate::OutputSemanticKind::FileNames);
    assert_eq!(
        contract.self_extension.list_selector.target_kind,
        crate::OutputScalarCountTargetKind::File
    );
    assert_eq!(contract.self_extension.list_selector.limit, Some(3));
    assert_eq!(
        contract.self_extension.list_selector.sort_by.as_deref(),
        Some("size_desc")
    );
    assert_eq!(
        contract.self_extension.list_selector.include_metadata,
        Some(true)
    );
    assert_eq!(
        contract.self_extension.list_selector.include_hidden,
        Some(false)
    );
}

#[test]
fn normalizer_schema_preserves_name_desc_list_selector() {
    let raw = r#"{
          "resolved_user_intent":"list scripts entries by reverse name order",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"directory_entry_groups selector_sort_by=name_desc",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "exact_sentence_count":0,
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"path",
            "delivery_intent":"none",
            "semantic_kind":"directory_entry_groups",
            "locator_hint":"scripts",
            "scalar_count_filter":0,
            "list_selector":{"target_kind":"any","limit":5,"sort_by":"name_desc","include_metadata":false,"include_hidden":false},
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none"},
          "turn_type":"task_request",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "list scripts entries by reverse name order",
    );
    let validated = crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation")
    .value;
    let contract = super::parse_output_contract(validated.output_contract, false);

    assert_eq!(
        contract.semantic_kind,
        crate::OutputSemanticKind::DirectoryEntryGroups
    );
    assert_eq!(
        contract.self_extension.list_selector.sort_by.as_deref(),
        Some("name_desc")
    );
}

#[test]
fn normalizer_schema_normalization_drops_null_list_selector_sort_by() {
    let raw = r#"{
          "resolved_user_intent":"列出 scripts/nl_tests/fixtures/device_local/docs 目录中的文件名，读取 release_checklist.md 文件开头内容，并根据内容判断该文件更像操作清单还是普通说明，用一句中文回答判断结果",
          "answer_candidate":"",
          "resume_behavior":"",
          "schedule_kind":"none",
          "schedule_intent":{},
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"需要执行文件系统操作来获取证据",
          "confidence":"high",
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "exact_sentence_count":1,
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"path",
            "delivery_intent":"none",
            "semantic_kind":"content_excerpt_with_summary",
            "locator_hint":"scripts/nl_tests/fixtures/device_local/docs",
            "scalar_count_filter":null,
            "list_selector":{
              "target_kind":"any",
              "limit":null,
              "sort_by":null,
              "include_metadata":null,
              "include_hidden":null
            },
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none"},
          "turn_type":"task_request",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "先列出 scripts/nl_tests/fixtures/device_local/docs 目录里的文件名，再读取 release_checklist.md 开头",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert!(
        value
            .pointer("/output_contract/list_selector/sort_by")
            .is_none(),
        "null sort_by should be removed instead of sent to schema validation"
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_does_not_recover_files_listing_from_string_contract() {
    let raw = r#"{
          "resolved_user_intent":"列出 /home/guagua/rustclaw/logs 目录下的前 10 个文件名，仅文件名，不读取文件内容",
          "answer_candidate":"",
          "resume_behavior":"proceed",
          "schedule_kind":"immediate",
          "schedule_intent":"list_directory",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"directory listing action",
          "confidence":1.0,
          "decision":"direct_answer",
          "output_contract":"files_listing",
          "execution_recipe":"ls -1 /home/guagua/rustclaw/logs | head -n 10",
          "turn_type":"request",
          "target_task_policy":"list_directory",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 logs 目录下的前 10 个文件名，不要读内容",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("answer_candidate").and_then(|v| v.as_str()),
        Some("")
    );
    let validated = crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation")
    .value;
    let contract = super::parse_output_contract(validated.output_contract, false);
    assert_eq!(contract.semantic_kind, crate::OutputSemanticKind::None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(
        super::parse_first_layer_decision_text(&validated.decision),
        Some(crate::FirstLayerDecision::DirectAnswer)
    );
}

#[test]
fn legacy_planner_unknown_scalar_output_contract_does_not_trigger_repair_without_machine_signal() {
    let raw = r#"{
          "resolved_user_intent":"查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
          "answer_candidate":null,
          "resume_behavior":"none",
          "schedule_kind":"immediate",
          "schedule_intent":"find_sh_scripts_directories",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"file_finder",
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户请求在当前仓库中搜索脚本文件，提取其所在目录并去重列出。这是明确且可执行的本地 FS 搜索任务。",
          "confidence":0.98,
          "decision":"planner_execute",
          "decision":"planner_execute",
          "output_contract":"structured_exec_state",
          "execution_recipe":"find /workspace -name \"*.sh\" -type f -exec dirname {} \\; | sort -u",
          "turn_type":"exec",
          "target_task_policy":"local_fs_search",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "查找当前仓库里所有 sh 脚本所在的目录，去重后列出来",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .get("answer_candidate")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert!(!report
        .details
        .contains("executable_route_unknown_scalar_output_contract"));
    assert!(!report.needs_llm_contract_integrity_repair());
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_recovers_detection_payload_as_planner_execute() {
    let raw = r#"{
          "resolved_user_intent":"检查仓库中是否存在 rustclaw.service 文件，只回答有或没有，并给出完整路径",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"repo existence check",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":{"response_shape":"strict","semantic_kind":"existence_with_path","requires_content_evidence":true}
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_preserves_recognized_execution_recipe_signal() {
    let raw = r#"{
          "resolved_user_intent":"列出 document 目录下所有文件的文件名列表",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"directory filename listing",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":{"type":"list","items":{"type":"string"}},
          "execution_recipe":{"kind":"ops_closed_loop","profile":"ops_service","target_scope":"current_repo"}
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录下有哪些文件，只输出文件名列表",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
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
        contract
            .get("requires_content_evidence")
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
fn normalizer_schema_normalization_does_not_treat_target_scope_as_execution() {
    let raw = r#"{
          "resolved_user_intent":"简单解释一下这个项目是什么",
          "answer_candidate":"",
          "needs_clarify":false,
          "reason":"chat explanation",
          "confidence":0.88,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"free"},
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"current_repo"}
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "简单解释一下这个项目是什么");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract
            .get("requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}
