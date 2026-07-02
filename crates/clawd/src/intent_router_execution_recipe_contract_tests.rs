// Execution-recipe contract repair tests for intent_router.

#[test]
fn scalar_runtime_tool_recipe_home_locator_repairs_to_current_user_patch() {
    let Ok(home) = std::env::var("HOME") else {
        return;
    };
    let raw = format!(
        r#"{{
          "resolved_user_intent":"获取当前系统用户名",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{{
            "response_shape":"scalar",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "contract_marker":"scalar_path_only",
            "locator_hint":"{}"
          }},
          "execution_recipe":{{"kind":"tool","tool_name":"system_basic","parameters":{{}}}},
          "turn_type":"status_query"
        }}"#,
        home
    );
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        &raw,
        "只输出当前用户名，不要解释",
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
            .pointer("/state_patch/runtime_status_query/kind")
            .and_then(|v| v.as_str()),
        Some("current_user")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_scalar_runtime_tool_observation"));
}

#[test]
fn scalar_path_runtime_tool_recipe_without_locator_repairs_to_raw_command_contract() {
    let raw = r#"{
          "resolved_user_intent":"获取当前操作系统用户名",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"scalar",
            "exact_sentence_count":0,
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "contract_marker":"scalar_path_only",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"system_info",
            "tool_name":"system_basic",
            "parameters":{"operation":"current_user"}
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
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("raw_command_output")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|v| v.as_str()),
        Some("none")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_scalar_runtime_tool_observation"));
}

#[test]
fn scalar_path_pwd_recipe_preserves_path_contract() {
    let raw = r#"{
          "resolved_user_intent":"用户询问当前工作目录路径",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"scalar",
            "exact_sentence_count":0,
            "requires_content_evidence":false,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "contract_marker":"scalar_path_only",
            "locator_hint":"",
            "self_extension":{"mode":"none","trigger":"none","execute_now":false}
          },
          "execution_recipe":{
            "kind":"tool",
            "tool":"system_basic",
            "command":"pwd",
            "args":{},
            "description":"获取当前工作目录路径"
          },
          "turn_type":"status_query",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let (normalized, _report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "今天的工作目录是哪个，告诉我路径就行",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|v| v.as_str()),
        Some("scalar_path_only")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|v| v.as_str()),
        Some("scalar")
    );
}

#[test]
fn command_payload_path_becomes_observable_locator_contract() {
    let raw = r#"{
          "resolved_user_intent":"读取 /tmp/clawd.log 文件的最后 2 行内容",
          "needs_clarify":false,
          "decision":"direct_answer",
          "decision":"direct_answer",
          "output_contract":"file_tail_lines",
          "execution_recipe":{
            "command":"tail -n 2 /tmp/clawd.log",
            "encoding":"utf-8",
            "path":"/tmp/clawd.log"
          }
        }"#;
    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "就第二个，看看最后 2 行",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

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
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|v| v.as_str()),
        Some("path")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|v| v.as_str()),
        Some("/tmp/clawd.log")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_command_payload"));
}

#[test]
fn contract_repair_report_marks_untrusted_recipe_as_conservative_none() {
    let raw = r#"{
          "resolved_user_intent":"总结刚才的对话",
          "needs_clarify":false,
          "decision":"direct_answer",
          "output_contract":"summary",
          "execution_recipe":["SUMMARIZE"]
        }"#;
    let (_normalized, report) =
        super::normalize_intent_normalizer_raw_for_schema_with_report(raw, "总结刚才的对话");

    assert!(report.source_csv().contains("conservative_none"));
    assert!(report
        .detail_csv()
        .contains("execution_recipe_untrusted_text_ignored"));
    assert!(
        !report.needs_llm_contract_integrity_repair(),
        "dropping an untrusted execution_recipe payload is schema cleanup; it must not rewrite an otherwise valid route contract"
    );
}

#[test]
fn package_manager_skill_recipe_repairs_to_detection_contract() {
    let raw = r#"{
          "resolved_user_intent":"inspect_package_manager",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"one_sentence",
            "requires_content_evidence":true,
            "locator_kind":"none",
            "contract_marker":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"skill",
            "capability":"package.detect_manager"
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "inspect system package manager",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert!(value
        .get("resolved_user_intent")
        .and_then(|value| value.as_str())
        .is_some_and(|intent| intent.contains("capability_ref=package.detect_manager")));
    assert!(!value
        .get("resolved_user_intent")
        .and_then(|value| value.as_str())
        .is_some_and(|intent| intent.contains("package.detect_manager_extra")));
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_package_detect_manager_capability"));
    assert!(!report
        .detail_csv()
        .contains("execution_recipe_untrusted_text_ignored"));
}

#[test]
fn package_manager_capability_recipe_wins_over_scalar_runtime_shape() {
    let raw = r#"{
          "resolved_user_intent":"observe host package manager",
          "answer_candidate":"",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"scalar",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"none",
            "delivery_intent":"none",
            "contract_marker":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"tool",
            "tool_name":"system_basic",
            "capability":"package.detect_manager"
          },
          "turn_type":"status_query"
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "observe host package manager",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert!(value
        .get("resolved_user_intent")
        .and_then(|value| value.as_str())
        .is_some_and(|intent| intent.contains("capability_ref=package.detect_manager")));
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_package_detect_manager_capability"));
    assert!(!report
        .detail_csv()
        .contains("execution_recipe_scalar_runtime_tool_observation"));
}

#[test]
fn port_probe_tool_recipe_repairs_to_service_status_contract() {
    let raw = r#"{
          "resolved_user_intent":"inspect local listening ports",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "contract_marker":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"tool",
            "tool":"netstat_ss_ports",
            "phase":"observe"
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "inspect local listening ports",
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
        Some("service_status")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_service_status_observation"));
    assert!(!report
        .detail_csv()
        .contains("execution_recipe_untrusted_text_ignored"));
}

#[test]
fn structured_read_recipe_with_explicit_locator_repairs_content_contract() {
    let raw = r#"{
          "resolved_user_intent":"read_title_of_note_file",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{"format":"plain_text","schema":"title"},
          "execution_recipe":{
            "steps":[
              {"action":"read_file","target":"scripts/nl_tests/fixtures/device_local/docs/service_notes.md"},
              {"action":"extract_title","method":"first_heading_line"},
              {"action":"output","content":"title"}
            ]
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "Read the note file title and output only the title.",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value.get("decision").and_then(|value| value.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("scalar")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|value| value.as_str()),
        Some("path")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|value| value.as_str()),
        Some("scripts/nl_tests/fixtures/device_local/docs/service_notes.md")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_structured_read_observation"));
}

#[test]
fn compact_read_file_title_recipe_repairs_to_scalar_contract() {
    let raw = r#"{
          "resolved_user_intent":"read_title_of_note_file",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{"format":"text","content":"title_only"},
          "execution_recipe":{
            "kind":"read_file_title",
            "target":"scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "extraction":"title",
            "output":"title_only"
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "Read the note file title and output only the title.",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("scalar")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|value| value.as_str()),
        Some("path")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|value| value.as_str()),
        Some("scripts/nl_tests/fixtures/device_local/docs/service_notes.md")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_structured_read_observation"));
}

#[test]
fn structured_field_recipe_repairs_structured_keys_contract_to_scalar_value() {
    let raw = r#"{
          "resolved_user_intent":"read manifest package field",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"strict",
            "requires_content_evidence":true,
            "locator_kind":"path",
            "locator_hint":"crates/clawd/Cargo.toml",
            "contract_marker":"structured_keys"
          },
          "execution_recipe":{
            "kind":"tool",
            "tool_name":"doc_parse",
            "params":{
              "path":"crates/clawd/Cargo.toml",
              "target_key":"package.name",
              "format":"toml"
            }
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "Read the structured manifest field.",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("scalar")
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
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|value| value.as_str()),
        Some("path")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|value| value.as_str()),
        Some("crates/clawd/Cargo.toml")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_structured_read_observation"));
    assert!(!report
        .detail_csv()
        .contains("execution_recipe_untrusted_text_ignored"));
}

#[test]
fn compact_file_read_title_recipe_repairs_to_scalar_contract() {
    let raw = r#"{
          "resolved_user_intent":"read file and extract title",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":null,
          "execution_recipe":{
            "kind":"file_read_title",
            "target_path":"scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
            "extract":"title_only"
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "Read the note file title and output only the title.",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("scalar")
    );
    assert_eq!(
        value
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_kind")
            .and_then(|value| value.as_str()),
        Some("path")
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|value| value.as_str()),
        Some("scripts/nl_tests/fixtures/device_local/docs/service_notes.md")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_structured_read_observation"));
}

#[test]
fn file_read_recipe_does_not_trust_model_only_filename_semantic() {
    let raw = r#"{
          "resolved_user_intent":"Read the file and output a scalar value",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":"filename_only",
          "execution_recipe":{
            "kind":"file_read",
            "target":"scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "output":"filename_only"
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "Process the referenced document.",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("scalar")
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
        Some(true)
    );
    assert_eq!(
        value
            .pointer("/output_contract/locator_hint")
            .and_then(|value| value.as_str()),
        Some("scripts/nl_tests/fixtures/device_local/docs/release_checklist.md")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_structured_read_observation"));
}

#[test]
fn file_read_recipe_preserves_explicit_filename_only_schema_request() {
    let raw = r#"{
          "resolved_user_intent":"Read the file and output filename only",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":"filename_only",
          "execution_recipe":{
            "kind":"file_read",
            "target":"scripts/nl_tests/fixtures/device_local/docs/release_checklist.md",
            "output":"filename_only"
          }
        }"#;

    let (normalized, report) = super::normalize_intent_normalizer_raw_for_schema_with_report(
        raw,
        "Return filename_only for the referenced document.",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|value| value.as_str()),
        Some("file_names")
    );
    assert_ne!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("scalar")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_structured_read_observation"));
}

#[test]
fn contract_repair_report_still_repairs_unknown_semantic_contracts() {
    let raw = r#"{
          "resolved_user_intent":"检查当前目录状态",
          "needs_clarify":false,
          "decision":"planner_execute",
          "output_contract":{
            "response_shape":"free",
            "requires_content_evidence":true,
            "delivery_required":false,
            "locator_kind":"current_workspace",
            "delivery_intent":"none",
            "contract_marker":"unknown_observable_semantic",
            "locator_hint":""
          },
          "execution_recipe":{"kind":"none","profile":"none","target_scope":"unknown"}
        }"#;
    let (_normalized, report) =
        super::normalize_intent_normalizer_raw_for_schema_with_report(raw, "检查当前目录状态");

    assert!(report.source_csv().contains("conservative_none"));
    assert!(report
        .detail_csv()
        .contains("output_contract_unknown_semantic_ignored"));
    assert!(report.needs_llm_contract_integrity_repair());
}
