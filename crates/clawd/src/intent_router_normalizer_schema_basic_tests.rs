// Basic normalizer schema repair tests for intent_router.

#[test]
fn normalizer_schema_normalization_preserves_direct_answer_decision() {
    let raw = r#"{"resolved_user_intent":"client-like-continuous-123","needs_clarify":false,"clarify_question":"","reason":"recent memory recall","confidence":1.0,"decision":"direct_answer"}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value.get("resolved_user_intent").and_then(|v| v.as_str()),
        Some("client-like-continuous-123")
    );
}

#[test]
fn normalizer_schema_normalization_preserves_planner_and_direct_decisions() {
    let raw = r#"{"resolved_user_intent":"check then explain","needs_clarify":false,"decision":"planner_execute"}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );

    let raw = r#"{"resolved_user_intent":"not an execution request","needs_clarify":false,"decision":"direct_answer"}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
}

#[test]
fn normalizer_schema_normalization_preserves_filesystem_mutation_result_contract() {
    let raw = r#"{"resolved_user_intent":"create the target directory and report the result","needs_clarify":false,"decision":"planner_execute","output_contract":{"response_shape":"free","requires_content_evidence":true,"delivery_required":false,"locator_kind":"path","delivery_intent":"none","semantic_kind":"filesystem_mutation_result","locator_hint":"document/nl_skill_tmp"}}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|value| value.as_str()),
        Some("filesystem_mutation_result")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("one_sentence")
    );
    assert_eq!(
        value
            .pointer("/output_contract/delivery_required")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
}

#[test]
fn normalizer_schema_normalization_preserves_object_resolved_intent() {
    let raw = r#"{"resolved_user_intent":{"test_id":"client-like-continuous-123"},"needs_clarify":false,"decision":"direct_answer"}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("resolved_user_intent").and_then(|v| v.as_str()),
        Some(r#"{"test_id":"client-like-continuous-123"}"#)
    );
}

#[test]
fn normalizer_schema_normalization_accepts_percent_confidence() {
    let raw = r#"{"resolved_user_intent":"检查当前目录隐藏文件","needs_clarify":false,"clarify_question":"","reason":"local inspection","confidence":100,"decision":"planner_execute"}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "fallback");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(value.get("confidence").and_then(|v| v.as_f64()), Some(1.0));
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
}

#[test]
fn normalizer_schema_normalization_recovers_stray_quote_after_bool() {
    let raw = r#"{"resolved_user_intent":"检查仓库中是否存在 rustclaw.service","needs_clarify":false,"clarify_question":"","reason":"repo inspection","confidence":0.95,"decision":"planner_execute","should_refresh_long_term_memory":false","agent_display_name_hint":"","output_contract":{"response_shape":"strict","requires_content_evidence":true,"delivery_required":false,"locator_kind":"current_workspace","delivery_intent":"none","semantic_kind":"existence_with_path","locator_hint":"rustclaw.service","self_extension":{"mode":"none","trigger":"none","execute_now":false}},"execution_recipe":{"kind":"none","profile":"none","target_scope":"none"}}"#;
    assert!(serde_json::from_str::<serde_json::Value>(raw).is_err());
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("existence_with_path")
    );
    assert_eq!(
        value
            .get("should_refresh_long_term_memory")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
}

#[test]
fn normalizer_schema_normalization_recovers_minimax_output_contract_only_payload() {
    let raw = r#"{"output_contract":{"response_shape":"free","requires_content_evidence":false,"delivery_required":true,"locator_kind":"path","delivery_intent":"list_filenames","semantic_kind":"file_listing","locator_hint":"logs"}}"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 logs 目录下的前 10 个文件名，不要读内容",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("resolved_user_intent").and_then(|v| v.as_str()),
        Some("列出 logs 目录下的前 10 个文件名，不要读内容")
    );
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("file_names")
    );
    assert_eq!(
        value
            .pointer("/output_contract/delivery_required")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
}

#[test]
fn normalizer_schema_normalization_rewrites_free_directory_groups_to_purpose_summary() {
    let raw = r#"{
          "resolved_user_intent": "List documentation files under document/ and explain which one is most relevant",
          "needs_clarify": false,
          "reason": "directory listing plus explanation",
          "confidence": 0.85,
          "decision": "planner_execute",
          "output_contract": {
            "response_shape": "free",
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "directory_entry_groups",
            "locator_hint": "document/"
          }
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "list documentation files under document/ and explain what the most relevant one is for",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("free")
    );
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("directory_purpose_summary")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_preserves_meaningful_duplicate_route_fields() {
    let raw = r#"{
          "resolved_user_intent":"列出 workspace 下 document 目录中所有 .md 文件，排除 README.md，报告剩余的文件名列表",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"目录文件名过滤列表",
          "confidence":"high",
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "exact_sentence_count":0,
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"path",
            "delivery_intent":"none",
            "semantic_kind":"file_names",
            "locator_hint":"document",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{"kind":"none"},
          "turn_type":"task_request",
          "answer_candidate":"",
          "decision":"",
          "output_contract":"",
          "execution_recipe":"",
          "turn_type":"",
          "attachment_processing_required":""
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "列出 document 目录里所有 .md 文件，但排除 README，告诉我还剩哪些",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|value| value.as_str()),
        Some("file_names")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|value| value.as_str()),
        Some("document")
    );
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("task_request")
    );
}

#[test]
fn contract_repair_report_marks_structured_alias_repair() {
    let raw = r#"{"output_contract":{"response_shape":"free","requires_content_evidence":false,"delivery_required":true,"locator_kind":"path","delivery_intent":"list_filenames","semantic_kind":"file_listing","locator_hint":"logs"}}"#;
    let (_normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "列出 logs 目录下的前 10 个文件名，不要读内容",
    );

    assert!(report.source_csv().contains("enum_alias"));
    assert!(report.source_csv().contains("structured_contract"));
    assert!(report
        .detail_csv()
        .contains("output_contract_semantic_kind_normalized"));
    assert!(report
        .detail_csv()
        .contains("decision_promoted_by_output_contract"));
    assert_eq!(report.class_csv(), "schema_normalization");
}

#[test]
fn normalizer_schema_normalization_maps_process_status_alias_to_service_status() {
    let raw = r#"{
          "resolved_user_intent":"Check whether telegramd is currently running and explain the status briefly.",
          "needs_clarify":false,
          "decision":"direct_answer",
          "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"process_status",
            "locator_hint":""
          },
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"unknown"}
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "check whether telegramd is currently running and explain the status briefly",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("service_status")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(true)
    );
    assert!(report.source_csv().contains("enum_alias"));
    assert!(
        !report.needs_llm_contract_integrity_repair(),
        "process_status is a schema-token alias for service_status, not an unknown semantic"
    );
}

#[test]
fn contract_repair_report_marks_command_payload_as_raw_output() {
    let raw = r#"{
          "resolved_user_intent":"列出 logs 目录下前 10 个文件名，不读取内容",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":null,
          "execution_recipe":{
            "command":"ls -1 logs/ | head -n 10",
            "working_dir":"/home/guagua/rustclaw"
          }
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "列出 logs 目录下的前 10 个文件名",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert!(report.source_csv().contains("command_payload"));
    assert!(report
        .detail_csv()
        .contains("execution_recipe_command_payload"));
    assert_eq!(report.class_csv(), "schema_normalization");
}

#[test]
fn scalar_runtime_tool_recipe_repairs_to_raw_command_contract() {
    let raw = r#"{
          "resolved_user_intent":"输出来用户名",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"tool",
            "tool_name":"system_basic",
            "parameters":{},
            "requires_content_evidence":true
          },
          "turn_type":"status_query"
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "只输出当前用户名，不要解释",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("scalar")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_scalar_runtime_tool_observation"));
}

#[test]
fn scalar_runtime_tool_recipe_name_alias_preserves_status_query_patch() {
    let raw = r#"{
          "resolved_user_intent":"获取当前系统用户名",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"tool",
            "name":"system_basic",
            "params":{"operation":"whoami"}
          },
          "turn_type":"status_query"
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "只输出当前用户名，不要解释",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/state_patch/runtime_status_query/kind")
            .and_then(|v| v.as_str()),
        Some("current_user")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_scalar_runtime_tool_observation"));
}

#[test]
fn scalar_runtime_tool_recipe_kernel_alias_preserves_status_query_patch() {
    let raw = r#"{
          "resolved_user_intent":"current system kernel release",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"tool",
            "name":"system_basic",
            "params":{"operation":"uname_r"}
          },
          "turn_type":"status_query"
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "return current kernel release only",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/state_patch/runtime_status_query/kind")
            .and_then(|v| v.as_str()),
        Some("kernel_release")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_scalar_runtime_tool_observation"));
}

#[test]
fn scalar_run_cmd_args_recipe_preserves_status_query_patch() {
    let raw = r#"{
          "resolved_user_intent":"获取当前系统用户名",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"tool",
            "tool":"run_cmd",
            "args":["whoami"]
          }
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "只输出当前用户名，不要解释",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/state_patch/runtime_status_query/kind")
            .and_then(|v| v.as_str()),
        Some("current_user")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_scalar_runtime_tool_observation"));
}

#[test]
fn normalizer_schema_normalization_maps_file_basename_semantic() {
    let raw = r#"{
          "resolved_user_intent":"Return only the basename of the active file target.",
          "needs_clarify":false,
          "decision":"direct_answer",
          "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":false,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "semantic_kind":"single_file_basename",
            "locator_hint":""
          },
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"none"}
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "only return the active file target basename",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("direct_answer")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("file_basename")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("scalar")
    );
    assert!(report.source_csv().contains("enum_alias"));
}
