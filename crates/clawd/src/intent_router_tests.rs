use super::{
    apply_self_contained_payload_direct_answer_contract_repair, normalizer_output_from_fallback,
    parse_execution_recipe_hint, ClarifyQuestionPolicy, IntentExecutionRecipeOut,
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, RouteDecision, ScheduleKind, TargetTaskPolicy, TurnType,
};
use crate::{
    execution_recipe::{ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeTargetScope},
    ActFinalizeStyle, FirstLayerDecision,
};
use serde_json::Value;

#[test]
fn parse_execution_recipe_hint_accepts_explicit_ops_service_contract() {
    let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
        kind: "ops_closed_loop".to_string(),
        profile: "ops_service".to_string(),
        target_scope: "system".to_string(),
    }))
    .expect("execution recipe spec");
    assert_eq!(spec.profile, ExecutionRecipeProfile::OpsService);
    assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::System);
    assert!(spec.inspect_first);
    assert!(spec.validation_required);
}

#[test]
fn inline_json_answer_candidate_can_repair_to_direct_answer_contract() {
    let request =
        r#"解释这个 JSON 代表什么：[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/tmp/rustclaw".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize = crate::ActFinalizeStyle::ChatWrapped;

    let repair = apply_self_contained_payload_direct_answer_contract_repair(
        &mut contract,
        request,
        &surface,
        false,
        ScheduleKind::None,
        None,
        false,
        "beta, alpha",
        &mut decision,
        &mut finalize,
    );

    assert_eq!(
        repair,
        Some("self_contained_payload_direct_answer_contract")
    );
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize, crate::ActFinalizeStyle::Plain);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn inline_json_transform_repair_keeps_planner_contract() {
    let request = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"sort","by":"score","order":"desc"},{"op":"project","fields":["name"]}]}"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/tmp/rustclaw".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize = crate::ActFinalizeStyle::ChatWrapped;

    let repair = apply_self_contained_payload_direct_answer_contract_repair(
        &mut contract,
        request,
        &surface,
        false,
        ScheduleKind::None,
        None,
        false,
        "beta, alpha",
        &mut decision,
        &mut finalize,
    );

    assert_eq!(repair, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(finalize, crate::ActFinalizeStyle::ChatWrapped);
}

#[test]
fn inline_json_transform_direct_answer_candidate_promotes_to_planner_contract() {
    let request = r#"把这个 JSON 数组按 score 从高到低排序，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract::default();
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize = crate::ActFinalizeStyle::Plain;

    let repair = super::apply_inline_structured_transform_direct_answer_repair(
        &mut contract,
        &surface,
        false,
        ScheduleKind::None,
        None,
        false,
        "| name | score |\n|------|-------|\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |",
        &mut decision,
        &mut finalize,
    );

    assert_eq!(repair, Some("inline_structured_transform_contract_repair"));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize, crate::ActFinalizeStyle::ChatWrapped);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

#[test]
fn inline_json_direct_answer_repair_rejects_explicit_path() {
    let request = r#"读取 data/input.json 并按 [{"field":"score"}] 排序"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize = crate::ActFinalizeStyle::ChatWrapped;

    let repair = apply_self_contained_payload_direct_answer_contract_repair(
        &mut contract,
        request,
        &surface,
        false,
        ScheduleKind::None,
        None,
        false,
        "beta, alpha",
        &mut decision,
        &mut finalize,
    );

    assert_eq!(repair, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
}

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
        !report.needs_llm_semantic_repair(),
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
            "semantic_kind":"scalar_path_only",
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
fn unobserved_runtime_status_answer_candidate_promotes_to_evidence_query() {
    let Some(current_user) = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let mut answer_candidate = current_user.trim().to_string();
    let mut state_patch = None;
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = ActFinalizeStyle::Plain;
    let mut turn_type = None;
    let mut target_policy = None;

    let reason = super::apply_unobserved_runtime_status_answer_candidate_repair(
        &mut contract,
        &mut answer_candidate,
        &mut state_patch,
        false,
        false,
        ScheduleKind::None,
        Some(crate::execution_recipe::ExecutionRecipeSpec::default()),
        &mut decision,
        &mut finalize_style,
        &mut turn_type,
        &mut target_policy,
    );

    assert_eq!(
        reason,
        Some("unobserved_runtime_status_answer_candidate_requires_evidence")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, ActFinalizeStyle::Plain);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert!(contract.requires_content_evidence);
    assert_eq!(turn_type, Some(TurnType::StatusQuery));
    assert_eq!(target_policy, Some(TargetTaskPolicy::Standalone));
    assert!(answer_candidate.is_empty());
    assert_eq!(
        state_patch
            .as_ref()
            .and_then(|value| value.pointer("/runtime_status_query/kind"))
            .and_then(Value::as_str),
        Some("current_user")
    );
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
            "semantic_kind":"scalar_path_only",
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
            .pointer("/output_contract/semantic_kind")
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
            "semantic_kind":"scalar_path_only",
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
            .pointer("/output_contract/semantic_kind")
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
            !report.needs_llm_semantic_repair(),
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
            "semantic_kind":"none",
            "locator_hint":""
          },
          "execution_recipe":{
            "kind":"skill",
            "name":"package_manager",
            "action":"detect"
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
            .pointer("/output_contract/semantic_kind")
            .and_then(|value| value.as_str()),
        Some("package_manager_detection")
    );
    assert!(report
        .detail_csv()
        .contains("execution_recipe_package_manager_detection"));
    assert!(!report
        .detail_csv()
        .contains("execution_recipe_untrusted_text_ignored"));
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
            "semantic_kind":"none",
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
            .pointer("/output_contract/semantic_kind")
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
            "semantic_kind":"structured_keys"
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            "semantic_kind":"unknown_observable_semantic",
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
    assert!(report.needs_llm_semantic_repair());
}

#[test]
fn current_turn_anchor_drift_repair_discards_contextual_path_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::SqliteSchemaVersion,
        locator_hint: "/tmp/rustclaw-anchor-test/data/db-basic-contract.sqlite".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "查询 /tmp/rustclaw-anchor-test/data/db-basic-contract.sqlite 的 schema version",
        "/tmp/rustclaw-anchor-test/logs",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_hint, "/tmp/rustclaw-anchor-test/logs");
}

#[test]
fn current_turn_anchor_drift_repair_preserves_file_delivery_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "/tmp/rustclaw-anchor-test/old.md".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "Send me /tmp/rustclaw-anchor-test/old.md",
        "/tmp/rustclaw-anchor-test/LICENSE.zh-CN.md",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(!contract.requires_content_evidence);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(
        contract.locator_hint,
        "/tmp/rustclaw-anchor-test/LICENSE.zh-CN.md"
    );
}

#[test]
fn current_turn_anchor_drift_repair_preserves_raw_command_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_hint: "/tmp/rustclaw-anchor-test/README.md".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "执行 ls scripts，把结果按字母倒序排，只输出前 5 个",
        "/tmp/rustclaw-anchor-test/scripts",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn current_turn_anchor_drift_repair_preserves_quantity_comparison_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::QuantityComparison,
        locator_hint: "/tmp/rustclaw-anchor-test/Cargo.toml".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "比较 Cargo.lock 和 Cargo.toml 的大小比例",
        "/tmp/rustclaw-anchor-test/Cargo.lock",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::QuantityComparison
    );
    assert_eq!(contract.locator_hint, workspace.display().to_string());
}

#[test]
fn current_turn_anchor_drift_repair_keeps_compatible_child_path() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::FileNames,
        locator_hint: "/tmp/rustclaw-anchor-test/logs/clawd.log".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "列出 /tmp/rustclaw-anchor-test/logs/clawd.log 的基本信息",
        "/tmp/rustclaw-anchor-test/logs",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
    assert_eq!(
        contract.locator_hint,
        "/tmp/rustclaw-anchor-test/logs/clawd.log"
    );
}

#[test]
fn current_turn_anchor_drift_repair_keeps_multi_target_locator_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_hint: "README.md, README.zh-CN.md, Cargo.toml".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "Check README.md, README.zh-CN.md, and Cargo.toml in the current workspace",
        "/tmp/rustclaw-anchor-test/README.md",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(
        contract.locator_hint,
        "README.md, README.zh-CN.md, Cargo.toml"
    );
}

#[test]
fn current_turn_anchor_drift_repair_preserves_archive_pair_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        locator_hint:
            "/tmp/rustclaw-anchor-test/tmp/test_bundle.zip | /tmp/rustclaw-anchor-test/out"
                .to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "archive unpack path pair",
        "/tmp/rustclaw-anchor-test/tmp/test_bundle.zip",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(
        contract.locator_hint,
        "/tmp/rustclaw-anchor-test/tmp/test_bundle.zip | /tmp/rustclaw-anchor-test/out"
    );
}

#[test]
fn current_turn_anchor_repair_stays_off_for_executionless_chat() {
    let contract = IntentOutputContract::default();
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::DirectAnswer,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_allowed_for_structured_evidence_contract() {
    let contract = IntentOutputContract {
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::DirectAnswer,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_allowed_for_explicit_act_route() {
    let contract = IntentOutputContract::default();
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_structured_config_contract_with_locator() {
    let contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::ConfigRiskAssessment,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_current_workspace_root_identity() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");
    let contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "RustClaw".to_string(),
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_current_workspace_absolute_hint() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");
    let contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/tmp/rustclaw-anchor-test/rustclaw".to_string(),
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn contract_repair_judge_schema_accepts_canonical_payload() {
    let raw = r#"{
          "apply": true,
          "reason": "malformed_contract_semantically_requires_directory_listing",
          "confidence": 0.91,
          "decision": "planner_execute",
          "needs_clarify": false,
          "clarify_question": "",
          "resolved_user_intent": "列出 logs 目录下前 3 个文件名，不读取内容",
          "output_contract": {
            "response_shape": "strict",
            "exact_sentence_count": null,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "file_names",
            "locator_hint": "logs",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
          },
          "execution_recipe": {"kind": "none", "profile": "none", "target_scope": "unknown"},
          "turn_type": "task_request",
          "target_task_policy": "standalone"
        }"#;

    crate::prompt_utils::validate_against_schema::<super::ContractRepairJudgeOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::ContractRepairJudge,
    )
    .expect("contract repair judge payload should validate");
}

#[test]
fn contract_repair_judge_scalar_semantic_token_normalizes_to_scalar_contract() {
    let raw = r#"{
          "apply": true,
          "reason": "memory_only_answer_candidate_conflict_with_current_file_read_request",
          "confidence": 0.91,
          "decision": "planner_execute",
          "needs_clarify": false,
          "clarify_question": "",
          "resolved_user_intent": "read package.json name field",
          "output_contract": {
            "response_shape": "strict",
            "exact_sentence_count": null,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "scalar",
            "locator_hint": "scripts/nl_tests/fixtures/device_local/package.json",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
          },
          "execution_recipe": {
            "kind": "structured_read",
            "profile": "read_only",
            "target_scope": "explicit_path"
          },
          "turn_type": "task_request",
          "target_task_policy": "standalone"
        }"#;

    let validated = crate::prompt_utils::validate_against_schema::<super::ContractRepairJudgeOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::ContractRepairJudge,
    )
    .expect("contract repair judge payload should validate");

    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "read package.json name field".to_string(),
        answer_candidate: "rustclaw-nl-fixture".to_string(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "direct answer candidate lacked current evidence".to_string(),
        confidence: 0.5,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(super::apply_contract_repair_judge_output(
        &mut out,
        validated.value
    ));

    assert_eq!(out.decision, "planner_execute");
    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/package.json"
    );
}

#[test]
fn contract_repair_judge_output_applies_semantic_contract() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "列出 document 目录下的所有文件名".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "malformed recipe text was ignored".to_string(),
        confidence: 0.5,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "malformed_contract_semantically_requires_directory_listing".to_string(),
        confidence: 0.91,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "列出 document 目录下所有文件名，只输出文件名列表".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "file_names".to_string(),
            locator_hint: "document".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: None,
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    assert_eq!(out.decision, "planner_execute");
    assert_eq!(out.confidence, 0.91);
    assert!(out.reason.contains("llm_semantic_contract_repair"));
    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_hint, "document");
    assert!(out.state_patch.is_none());
}

#[test]
fn contract_repair_judge_preserves_structured_config_key_contract() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "读取 configs/config.toml 的顶层键名".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "normalizer chose structured keys".to_string(),
        confidence: 0.9,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "structured_keys".to_string(),
            locator_hint: "configs/config.toml".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "fresh_file_observation_required".to_string(),
        confidence: 0.95,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "读取 configs/config.toml 的顶层键名列表".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "configs/config.toml".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: None,
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    let contract = super::parse_output_contract(out.output_contract, false);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::StructuredKeys);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
    assert!(out
        .reason
        .contains("structured_config_key_contract_preserved"));
}

#[test]
fn contract_repair_judge_missing_turn_binding_forces_missing_locator_clarify() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "read remembered log alias".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "memory alias selected path".to_string(),
        confidence: 0.5,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "deictic_reference": {"target": "current_turn_locator"}
        })),
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "execution_recipe_untrusted_text_ignored_and_turn_binding_missing_for_content_read"
            .to_string(),
        confidence: 0.95,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "read remembered log alias".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "/repo/logs/app.log".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "fs_basic".to_string(),
            profile: "read_only".to_string(),
            target_scope: "explicit_path".to_string(),
        }),
        turn_type: "task_request".to_string(),
        target_task_policy: "standalone".to_string(),
        state_patch: Some(serde_json::json!({
            "deictic_reference": {"target": "current_turn_locator"}
        })),
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    assert_eq!(out.decision, "clarify");
    assert!(out.needs_clarify);
    let contract = super::parse_output_contract(out.output_contract, false);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert_eq!(
        out.state_patch
            .as_ref()
            .and_then(|patch| patch.get("deictic_reference"))
            .and_then(|value| value.get("target"))
            .and_then(serde_json::Value::as_str),
        Some("missing_locator")
    );
}

#[test]
fn contract_repair_judge_output_clears_stale_file_delivery_flag() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "Write a short release note for RustClaw".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: true,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: "RustClaw".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "malformed output contract".to_string(),
        confidence: 0.5,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "file_token".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: true,
            locator_kind: "path".to_string(),
            delivery_intent: "file_single".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "inline_text_contract".to_string(),
        confidence: 0.85,
        decision: "direct_answer".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "Write a short release note for RustClaw as inline content."
            .to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "none".to_string(),
            profile: "none".to_string(),
            target_scope: "none".to_string(),
        }),
        turn_type: String::new(),
        target_task_policy: String::new(),
        state_patch: None,
    };

    assert!(super::apply_contract_repair_judge_output(&mut out, repair));

    assert!(!out.wants_file_delivery);
    let contract = super::parse_output_contract(out.output_contract, out.wants_file_delivery);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn semantic_suspect_flags_chat_with_observable_contract() {
    let out = super::IntentNormalizerOut {
        resolved_user_intent: "check README.md exists".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "scalar".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "filename".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "existence_with_path".to_string(),
            locator_hint: "README.md".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, None),
        Some("chat_route_requires_content_evidence")
    );
}

#[test]
fn semantic_suspect_reviews_planner_file_names_contract() {
    let out = super::IntentNormalizerOut {
        resolved_user_intent: "List matching workspace files and summarize their purpose."
            .to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "file_names".to_string(),
            locator_hint: String::new(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, None),
        Some("file_names_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_planner_file_paths_contract() {
    let out = super::IntentNormalizerOut {
            resolved_user_intent:
                "List matching workspace file paths, then identify the largest file and summarize its role."
                    .to_string(),
            answer_candidate: String::new(),
            resume_behavior: "none".to_string(),
            schedule_kind: "none".to_string(),
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: String::new(),
            confidence: 0.8,
            decision: "planner_execute".to_string(),
            schedule_intent: None,
            output_contract: Some(super::IntentOutputContractOut {
                response_shape: "strict".to_string(),
                exact_sentence_count: None,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: "current_workspace".to_string(),
                delivery_intent: "none".to_string(),
                semantic_kind: "file_paths".to_string(),
                locator_hint: String::new(),
                self_extension: None,
            }),
            execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
            turn_type: String::new(),
            target_task_policy: String::new(),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, None),
        Some("file_paths_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_planner_existence_summary_contract() {
    let out = super::IntentNormalizerOut {
        resolved_user_intent: "Check whether AGENTS.md exists and return its absolute path."
            .to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "existence_with_path_summary".to_string(),
            locator_hint: "AGENTS.md".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, None),
        Some("existence_summary_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_planner_multi_path_generic_contract() {
    let req = "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert!(surface.locator_target_pair.is_some());
    let out = super::IntentNormalizerOut {
        resolved_user_intent: req.to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, Some(&surface)),
        Some("multi_path_generic_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_planner_multi_path_generic_contract_before_evidence_repair() {
    let req = "比较 Cargo.lock 和 Cargo.toml 的大小，告诉我 lock 大概是 toml 的几倍";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert!(surface.locator_target_pair.is_some());
    let out = super::IntentNormalizerOut {
        resolved_user_intent: req.to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "scalar".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, Some(&surface)),
        Some("multi_path_generic_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_planner_single_path_generic_metadata_contract() {
    let req = "看一下 target 大概多大";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let out = super::IntentNormalizerOut {
        resolved_user_intent: req.to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "one_sentence".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "target".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, Some(&surface)),
        Some("single_path_generic_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_planner_single_path_generic_free_contract() {
    let req = "Inspect prompts/schemas and produce a grounded directory summary.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let out = super::IntentNormalizerOut {
        resolved_user_intent: req.to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: "prompts/schemas".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, Some(&surface)),
        Some("single_path_generic_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_planner_single_path_scalar_count_contract() {
    let req = "看一下 target 大概多大";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let out = super::IntentNormalizerOut {
        resolved_user_intent: req.to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "planner_execute".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "scalar".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "scalar_count".to_string(),
            locator_hint: "target".to_string(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(&out, Some(&surface)),
        Some("single_path_scalar_count_contract_needs_semantic_shape_review")
    );
}

#[test]
fn contract_repair_judge_output_rejects_low_confidence() {
    let mut out = super::IntentNormalizerOut {
        resolved_user_intent: "总结刚才的对话".to_string(),
        answer_candidate: String::new(),
        resume_behavior: "none".to_string(),
        schedule_kind: "none".to_string(),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut::default()),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let repair = super::ContractRepairJudgeOut {
        apply: true,
        reason: "uncertain".to_string(),
        confidence: 0.59,
        decision: "planner_execute".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        resolved_user_intent: "bad".to_string(),
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "current_workspace".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "file_names".to_string(),
            locator_hint: String::new(),
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        state_patch: None,
    };

    assert!(!super::apply_contract_repair_judge_output(&mut out, repair));
    assert_eq!(out.decision, "direct_answer");
    assert_eq!(out.resolved_user_intent, "总结刚才的对话");
}

#[test]
fn observed_context_summary_followup_does_not_force_fresh_evidence() {
    let mut contract = IntentOutputContract::default();
    contract.response_shape = OutputResponseShape::OneSentence;
    contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    contract.locator_kind = OutputLocatorKind::Filename;
    contract.locator_hint = "app.log".to_string();

    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "in one sentence tell me if anything looks abnormal",
    );
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        "in one sentence tell me if anything looks abnormal",
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::DirectAnswer,
        "",
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
    );

    assert_eq!(reason, Some("existing_observed_context_synthesis"));
    assert!(!contract.requires_content_evidence);
}

#[test]
fn explicit_locator_summary_still_requires_fresh_evidence() {
    let mut contract = IntentOutputContract::default();
    contract.response_shape = OutputResponseShape::OneSentence;
    contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    contract.locator_kind = OutputLocatorKind::Filename;
    contract.locator_hint = "app.log".to_string();

    let surface =
        crate::intent::surface_signals::analyze_prompt_surface("summarize app.log briefly");
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        "summarize app.log briefly",
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::DirectAnswer,
        "",
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
    );

    assert_eq!(reason, Some("semantic_contract_requires_evidence"));
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_config_field_value_repairs_to_config_mutation_contract() {
    let request = "run/nl_eval_tmp/config_edit_smoke/config.toml skills.skill_switches.config_edit_nl_smoke = true";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("config_mutation_structural_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ConfigMutation);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "run/nl_eval_tmp/config_edit_smoke/config.toml"
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
}

#[test]
fn structural_config_field_without_value_does_not_repair_to_mutation() {
    let request =
        "run/nl_eval_tmp/config_edit_smoke/config.toml skills.skill_switches.config_edit_nl_smoke";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

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
          "decision":"planner_execute",
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
            .pointer("/output_contract/semantic_kind")
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
            semantic_kind: "file_names".to_string(),
            locator_hint: "logs".to_string(),
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
            semantic_kind: "raw_command_output".to_string(),
            locator_hint: "/tmp/app.log".to_string(),
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
            semantic_kind: "content_excerpt_summary".to_string(),
            locator_hint: "/tmp/report.md".to_string(),
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/requires_content_evidence")
            .and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
          "output_contract":{"response_shape":"strict","semantic_kind":"existence_with_path","requires_content_evidence":true},
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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
fn normalizer_schema_normalization_recovers_file_names_only_contract() {
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
            .pointer("/output_contract/semantic_kind")
            .and_then(|v| v.as_str()),
        Some("file_names")
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
fn normalizer_schema_normalization_recovers_scalar_output_contract_answer_candidate() {
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
        Some("client-like-continuous-20260430_094246")
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
fn normalizer_schema_normalization_recovers_object_answer_candidate_and_ignores_json_contract() {
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
        Some("client-like-continuous-20260430_095834")
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
            .pointer("/output_contract/semantic_kind")
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
            .pointer("/output_contract/semantic_kind")
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

#[test]
fn normalizer_resolved_intent_includes_answer_candidate_for_chat_stage() {
    let resolved = super::merge_answer_candidate_into_resolved_intent(
        "查询之前记住的测试编号".to_string(),
        "client-like-continuous-20260430_094246",
    );
    assert_eq!(
        resolved,
        "查询之前记住的测试编号\nanswer_candidate: client-like-continuous-20260430_094246"
    );
    assert_eq!(
        super::merge_answer_candidate_into_resolved_intent(
            resolved.clone(),
            "client-like-continuous-20260430_094246",
        ),
        resolved
    );
}

#[test]
fn answer_candidate_binding_reports_memory_only_without_phrase_matching() {
    let request = "Return the marker value only.";
    let answer = "client-like-continuous-20260501_054730";
    let memory_only = crate::task_context_builder::RouteContextView {
        memory_context: format!("#### RELEVANT_FACTS\n- remembered marker {answer}"),
        recent_assistant_replies: "<none>".to_string(),
        recent_turns_full: "<none>".to_string(),
        last_turn_full: "<none>".to_string(),
        recent_execution_context: "<none>".to_string(),
        ..Default::default()
    };

    let report = super::analyze_answer_candidate_binding(request, answer, &memory_only)
        .expect("candidate should produce binding report");
    assert!(report.is_memory_only_binding());
    assert!(report.is_distinctive());
    assert!(!report.in_current_request);

    let recent_bound = crate::task_context_builder::RouteContextView {
        memory_context: memory_only.memory_context,
        recent_assistant_replies: format!("已记录。测试编号 `{answer}` 已记住。"),
        ..Default::default()
    };

    let report = super::analyze_answer_candidate_binding(request, answer, &recent_bound)
        .expect("candidate should produce binding report");
    assert!(!report.is_memory_only_binding());
    assert!(report.has_current_or_recent_binding());
}

#[test]
fn answer_candidate_binding_context_is_structural_not_language_specific() {
    let request = "For this continuous test, remember marker RC-CONT-EN-0428-B. Reply with one short confirmation.";
    let answer = "client-like-continuous-20260501_054730";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older marker {answer}"),
        ..Default::default()
    };

    let report = super::analyze_answer_candidate_binding(request, answer, &route_view)
        .expect("candidate should produce binding report");
    let context = super::answer_candidate_binding_repair_context(&report, true);
    assert!(context.contains("should_refresh_long_term_memory: true"));
    assert!(context.contains("memory_only_binding: true"));
    assert!(context.contains("distinctive_candidate: true"));
}

#[test]
fn memory_only_answer_candidate_clears_when_recent_context_has_conflicting_scalar() {
    let answer = "client-like-continuous-20260516_043255";
    let recent_marker = "RC-CONT-CN-0428-A";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older remembered marker {answer}"),
        recent_assistant_replies: format!("好的，已记住编号 {recent_marker}。"),
        recent_turns_full: format!(
            "### RECENT_TURNS_FULL\n[TURN -1]\nUser: remember {recent_marker}\nAssistant: {recent_marker}\n[/TURN]\n"
        ),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "Return the marker value only.",
        answer,
        &route_view,
    )
    .expect("candidate should produce binding report");
    let conflicts = super::recent_distinctive_scalar_conflict_tokens(&binding, &route_view);
    assert_eq!(conflicts, vec![recent_marker.to_string()]);

    let mut out: super::IntentNormalizerOut = serde_json::from_value(serde_json::json!({
        "resolved_user_intent": "recall marker",
        "answer_candidate": answer,
        "reason": "memory candidate",
        "decision": "direct_answer"
    }))
    .expect("valid normalizer out");
    assert_eq!(
        super::clear_memory_only_answer_candidate_if_recent_context_conflicts(
            &mut out,
            Some(&binding),
            &route_view,
        ),
        Some("memory_only_answer_candidate_recent_scalar_conflict_cleared")
    );
    assert!(out.answer_candidate.is_empty());
    assert!(out
        .reason
        .contains("memory_only_answer_candidate_recent_scalar_conflict_cleared"));
}

#[test]
fn memory_only_answer_candidate_does_not_clear_for_recent_paths_only() {
    let answer = "client-like-continuous-20260516_043255";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older remembered marker {answer}"),
        recent_turns_full: "### RECENT_TURNS_FULL\n[TURN -1]\nUser: read /tmp/report-2026.md\nAssistant: ok\n[/TURN]\n".to_string(),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "Return the marker value only.",
        answer,
        &route_view,
    )
    .expect("candidate should produce binding report");
    assert!(super::recent_distinctive_scalar_conflict_tokens(&binding, &route_view).is_empty());

    let mut out: super::IntentNormalizerOut = serde_json::from_value(serde_json::json!({
        "resolved_user_intent": "recall marker",
        "answer_candidate": answer,
        "reason": "memory candidate",
        "decision": "direct_answer"
    }))
    .expect("valid normalizer out");
    assert_eq!(
        super::clear_memory_only_answer_candidate_if_recent_context_conflicts(
            &mut out,
            Some(&binding),
            &route_view,
        ),
        None
    );
    assert_eq!(out.answer_candidate, answer);
}

#[test]
fn memory_only_answer_candidate_rebinds_to_latest_user_memory_scalar() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "recent-user-memory-scalar".to_string(),
        user_id: 91,
        chat_id: 202,
        user_key: Some("user:recent-memory-scalar".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"recall marker"}).to_string(),
    };
    let stale = "client-like-continuous-20260516_043255";
    let latest = "RC-CONT-CN-0428-A";
    {
        let db = state.core.db.get().expect("db");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            201,
            task.user_key.as_deref().unwrap(),
            1,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            stale,
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1000,
        )
        .expect("index stale memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            203,
            task.user_key.as_deref().unwrap(),
            2,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            &format!("已记住编号 {latest}。"),
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1010,
        )
        .expect("index latest memory");
    }

    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older remembered marker {stale}"),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "Return the marker value only.",
        stale,
        &route_view,
    )
    .expect("candidate should produce binding report");

    assert_eq!(
        super::latest_user_memory_distinctive_scalar_candidate(&state, &task, &binding).as_deref(),
        Some(latest)
    );

    let mut out: super::IntentNormalizerOut = serde_json::from_value(serde_json::json!({
        "resolved_user_intent": "recall marker",
        "answer_candidate": stale,
        "reason": "memory candidate",
        "decision": "clarify",
        "needs_clarify": true,
        "clarify_question": "which marker?"
    }))
    .expect("valid normalizer out");
    assert_eq!(
        super::rebind_memory_only_answer_candidate_to_recent_user_memory(
            &state,
            &task,
            &mut out,
            Some(&binding),
        ),
        Some("memory_only_answer_candidate_rebound_to_recent_user_memory")
    );
    assert_eq!(out.answer_candidate, latest);
    assert_eq!(out.decision, "direct_answer");
    assert!(!out.needs_clarify);
    assert!(out.clarify_question.is_empty());
}

#[test]
fn memory_only_answer_candidate_rebinds_locator_only_to_latest_locator() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "recent-user-memory-locator".to_string(),
        user_id: 92,
        chat_id: 204,
        user_key: Some("user:recent-memory-locator".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"recall note file"}).to_string(),
    };
    let stale = "scripts/nl_tests/fixtures/device_local/docs/service_notes.md";
    let latest = "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md";
    {
        let db = state.core.db.get().expect("db");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            301,
            task.user_key.as_deref().unwrap(),
            1,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            stale,
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1000,
        )
        .expect("index stale path memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            302,
            task.user_key.as_deref().unwrap(),
            2,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            &format!("note file -> {latest}"),
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1010,
        )
        .expect("index latest path memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            303,
            task.user_key.as_deref().unwrap(),
            3,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            "remembered marker RC-CONT-EN-0428-B",
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1020,
        )
        .expect("index unrelated marker memory");
    }

    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older note file mapping {stale}"),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "What file does the note file refer to now?",
        stale,
        &route_view,
    )
    .expect("candidate should produce binding report");

    assert_eq!(
        super::latest_user_memory_distinctive_scalar_candidate(&state, &task, &binding).as_deref(),
        Some(latest)
    );
}

#[test]
fn memory_only_answer_candidate_does_not_rebind_locator_to_marker() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "recent-user-memory-locator-no-cross-class".to_string(),
        user_id: 93,
        chat_id: 206,
        user_key: Some("user:recent-memory-locator-no-cross-class".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"recall note file"}).to_string(),
    };
    let stale = "scripts/nl_tests/fixtures/device_local/docs/service_notes.md";
    {
        let db = state.core.db.get().expect("db");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            401,
            task.user_key.as_deref().unwrap(),
            1,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            stale,
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1000,
        )
        .expect("index stale path memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            402,
            task.user_key.as_deref().unwrap(),
            2,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            "remembered marker RC-CONT-EN-0428-B",
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1010,
        )
        .expect("index marker memory");
    }

    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older note file mapping {stale}"),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "What file does the note file refer to now?",
        stale,
        &route_view,
    )
    .expect("candidate should produce binding report");

    assert_eq!(
        super::latest_user_memory_distinctive_scalar_candidate(&state, &task, &binding),
        None
    );
}

#[test]
fn normalizer_schema_normalization_coerces_invalid_schedule_kind_to_none() {
    let raw = r#"{
          "resolved_user_intent":"修改方案目标用户为开发者，输出正文",
          "resume_behavior":"resume",
          "schedule_kind":"immediate",
          "schedule_intent":"deliver",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户修正目标受众约束并明确要求输出正文",
          "confidence":1.0,
          "decision":"direct_answer",
          "output_contract":{"kind":"text","text_content":"示例正文","media_type":"text/plain"},
          "execution_recipe":{"kind":"none"},
          "turn_type":"task_append",
          "target_task_policy":"reuse_active",
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
        value.get("schedule_kind").and_then(|value| value.as_str()),
        Some("none")
    );
    assert!(value
        .get("schedule_intent")
        .is_some_and(|value| value.is_null()));
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("task_append")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("reuse_active")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_recovers_minimax_text_only_append_payload() {
    let raw = r#"{
          "resolved_user_intent":"调整字数约束：RustClaw 连续会话可靠性技术博客风格，100字以内",
          "answer_candidate":"RustClaw 以多层容错与自动状态恢复机制，在网络抖动或进程异常时快速回到上一状态，保障关键业务链路不中断。",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"none",
          "needs_clarify":false,
          "clarify_question":"none",
          "reason":"字数约束更新，主题、受众、语气不变，直接输出精简版本",
          "confidence":"0.97",
          "decision":"direct_answer",
          "output_contract":"text_only",
          "execution_recipe":{"kind":"none","requires_content_evidence":false,"locator_kind":"none"},
          "turn_type":"task_append",
          "target_task_policy":"reuse_active",
          "should_interrupt_active_run":"no",
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "不对，不是 200 字，是 100 字以内");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("task_append")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("reuse_active")
    );
    assert_eq!(
        value
            .get("should_interrupt_active_run")
            .and_then(|value| value.as_bool()),
        Some(false)
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
fn normalizer_schema_normalization_treats_one_line_comparison_as_strict_shape() {
    let raw = r#"{
          "resolved_user_intent":"比较两个字段并输出一行",
          "resume_behavior":null,
          "schedule_kind":"immediate",
          "schedule_intent":"read_and_compare",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"clear comparison",
          "confidence":0.95,
          "decision":"planner_execute",
          "output_contract":"one_line_comparison",
          "execution_recipe":{"kind":"file_read_two"},
          "turn_type":"task_request",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "读取两个字段，最后只用一行输出：前者、后者、一样或不一样",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("strict")
    );
    assert_eq!(
        value
            .pointer("/output_contract/semantic_kind")
            .and_then(|value| value.as_str()),
        Some("recent_scalar_equality_check")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_rejects_invalid_resume_and_turn_tokens() {
    let raw = r#"{
          "resolved_user_intent":"用户希望在80字以内生成一份面向开发者的简短方案，缺少主题信息。",
          "resume_behavior":"unsupported_resume_token",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"assistant",
          "needs_clarify":true,
          "clarify_question":"请告诉我这份方案的主题是什么？",
          "reason":"缺少核心主题",
          "confidence":0.95,
          "decision":"direct_answer",
          "output_contract":"clarification",
          "execution_recipe":{"kind":"none"},
          "turn_type":"unsupported_turn_token",
          "target_task_policy":"reuse active",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "不对，目标用户改成开发者，不是老板。只输出修正后的正文。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value
            .get("resume_behavior")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("reuse_active")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_coerces_empty_state_patch_string() {
    let raw = r#"{
          "resolved_user_intent":"用户询问刚才记住的测试编号",
          "resume_behavior":"",
          "schedule_kind":"",
          "schedule_intent":"",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"上下文中已有编号",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":"",
          "execution_recipe":{"kind":"none"},
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":"",
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "刚才让你记住的连续测试编号是什么？只回答编号。",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert!(value
        .get("state_patch")
        .is_some_and(|value| value.is_null()));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_promotes_misnested_turn_analysis_fields() {
    let raw = r#"{
          "resolved_user_intent":"用一句中文确认当前测试正在进行",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"pure confirmation",
          "confidence":0.99,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"one_sentence"},
          "execution_recipe":{
            "kind":"none",
            "profile":"none",
            "target_scope":"none",
            "turn_type":"task_request",
            "target_task_policy":"standalone",
            "state_patch":{"constraints":{"tone":"brief"}},
            "attachment_processing_required":true
          },
          "turn_type":"",
          "target_task_policy":"",
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "请用一句中文回复确认。");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("turn_type").and_then(|value| value.as_str()),
        Some("task_request")
    );
    assert_eq!(
        value
            .get("target_task_policy")
            .and_then(|value| value.as_str()),
        Some("standalone")
    );
    assert_eq!(
        value
            .pointer("/state_patch/constraints/tone")
            .and_then(|value| value.as_str()),
        Some("brief")
    );
    assert_eq!(
        value
            .get("attachment_processing_required")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert!(value
        .get("execution_recipe")
        .and_then(|value| value.as_object())
        .is_some_and(|recipe| !recipe.contains_key("turn_type")
            && !recipe.contains_key("target_task_policy")
            && !recipe.contains_key("state_patch")));
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_recovers_commands_payload_as_execution_signal() {
    let raw = r#"{
          "resolved_user_intent":"User wants to know approximate size of the target directory",
          "answer_candidate":"",
          "resume_behavior":"none",
          "schedule_kind":"none",
          "schedule_intent":"none",
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":"RustClaw",
          "needs_clarify":false,
          "clarify_question":"",
          "reason":"User requested local directory size and provided a command payload.",
          "confidence":0.95,
          "decision":"direct_answer",
          "output_contract":"json",
          "execution_recipe":{"commands":[{"executor":"local","command":"du -sh target","purpose":"Get approximate size"}]},
          "turn_type":"task_request",
          "target_task_policy":"none",
          "should_interrupt_active_run":false,
          "state_patch":{},
          "attachment_processing_required":false
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "看一下 target 大概多大");
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
            .pointer("/execution_recipe/kind")
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
fn normalizer_schema_normalization_ignores_response_recipe_as_execution_signal() {
    let raw = r#"{
          "resolved_user_intent":"确认用户正在进行 RustClaw 真实客户端连续会话测试",
          "answer_candidate":"好的，我已确认你正在进行 RustClaw 的真实客户端连续会话测试。",
          "resume_behavior":null,
          "schedule_kind":null,
          "wants_file_delivery":false,
          "should_refresh_long_term_memory":false,
          "agent_display_name_hint":null,
          "needs_clarify":false,
          "clarify_question":null,
          "reason":"用户明确要求用一句中文回复确认其正在进行 RustClaw 真实客户端连续会话测试",
          "confidence":0.98,
          "decision":"direct_answer",
          "output_contract":{"response_shape":"text","semantic_kind":"confirmation"},
          "execution_recipe":"respond_with_simple_chinese_confirmation",
          "turn_type":"greeting",
          "target_task_policy":null,
          "should_interrupt_active_run":false,
          "state_patch":null,
          "attachment_processing_required":false
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "你好，我正在做 RustClaw 的真实客户端连续会话测试，请用一句中文回复确认。",
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
            .pointer("/output_contract/semantic_kind")
            .and_then(|value| value.as_str()),
        Some("none")
    );
    assert_eq!(
        value
            .pointer("/execution_recipe/kind")
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
fn structured_preference_ack_is_detached_from_active_task() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    assert!(super::should_detach_bare_acknowledgement_from_active_task(
        Some(TurnType::PreferenceOrMemory),
        Some(TargetTaskPolicy::ReuseActive),
        FirstLayerDecision::DirectAnswer,
        &contract,
        None,
        false,
    ));
    assert!(!super::should_detach_bare_acknowledgement_from_active_task(
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        FirstLayerDecision::DirectAnswer,
        &contract,
        None,
        false,
    ));
    assert!(!super::should_detach_bare_acknowledgement_from_active_task(
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        FirstLayerDecision::DirectAnswer,
        &contract,
        Some(&serde_json::json!({"output_refinement":"one_sentence_only"})),
        false,
    ));
}

#[test]
fn orphan_output_shape_clarify_downgrades_to_standalone_chat() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: crate::OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: crate::OutputLocatorKind::None,
        delivery_intent: crate::OutputDeliveryIntent::None,
        semantic_kind: crate::OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let snapshot_without_primary = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState::default()),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(
        super::should_downgrade_orphan_output_shape_clarify_to_direct_answer(
            Some(&snapshot_without_primary),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::Clarify,
            &contract,
            None,
            false,
            false,
        )
    );

    let snapshot_with_primary = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我写个方案".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(
        !super::should_downgrade_orphan_output_shape_clarify_to_direct_answer(
            Some(&snapshot_with_primary),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::Clarify,
            &contract,
            None,
            false,
            false,
        )
    );
}

#[test]
fn missing_turn_type_with_standalone_policy_infers_primary_task_request() {
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::Standalone),
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskRequest)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            Some(TurnType::PreferenceOrMemory),
            Some(TargetTaskPolicy::Standalone),
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            true,
        ),
        Some(TurnType::PreferenceOrMemory)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::Standalone),
            FirstLayerDecision::Clarify,
            true,
            crate::ScheduleKind::None,
            false,
        ),
        None
    );
}

#[test]
fn missing_turn_type_with_active_task_policy_infers_mutation_type() {
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskAppend)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::ReplaceActive),
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskReplace)
    );
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            true,
        ),
        None
    );
}

#[test]
fn missing_policy_with_strict_chat_deliverable_infers_standalone_task() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let policy = super::infer_missing_target_policy_from_contract(
        None,
        None,
        FirstLayerDecision::DirectAnswer,
        false,
        crate::ScheduleKind::None,
        false,
        &contract,
    );
    assert_eq!(policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(
        super::infer_missing_turn_type_from_policy(
            None,
            policy,
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            false,
        ),
        Some(TurnType::TaskRequest)
    );
}

#[test]
fn missing_policy_with_non_strict_chat_does_not_promote_generic_chat() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    assert_eq!(
        super::infer_missing_target_policy_from_contract(
            None,
            None,
            FirstLayerDecision::DirectAnswer,
            false,
            crate::ScheduleKind::None,
            false,
            &contract,
        ),
        None
    );
}

#[test]
fn empty_nested_state_patch_is_not_meaningful() {
    assert!(!super::is_meaningful_state_patch(&serde_json::json!({
        "alias_bindings": [],
        "notes": ""
    })));
    assert!(super::is_meaningful_state_patch(&serde_json::json!({
        "audience": "developers"
    })));
}

#[test]
fn compact_normalizer_prompt_pins_output_contract_schema() {
    let route_view = crate::task_context_builder::RouteContextView {
        request_surface_hints: "locator_target_pair: Cargo.toml | Cargo.lock".to_string(),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        "list current toml files and briefly explain them",
    );

    assert!(prompt.contains("Allowed output_contract keys only"));
    assert!(prompt.contains("output_contract as a JSON object, never as a string token"));
    assert!(prompt.contains("Use ALIASES only for temporary references"));
    assert!(prompt.contains("ALIASES: <none>"));
    assert!(prompt.contains("CAPABILITIES:"));
    assert!(
        prompt.contains("Allowed response_shape: free, one_sentence, strict, scalar, file_token")
    );
    assert!(prompt.contains("Allowed semantic_kind: none, raw_command_output"));
    assert!(prompt.contains("semantic_kind=\"hidden_entries_check\""));
    assert!(prompt.contains("semantic_kind=\"existence_with_path\""));
    assert!(prompt.contains("file/path metadata comparisons"));
    assert!(prompt.contains("semantic_kind=\"quantity_comparison\""));
    assert!(prompt.contains("Text drafting/composition is not file delivery by default"));
    assert!(prompt.contains("Write a long article about RustClaw"));
    assert!(prompt.contains("presence judgment is not numeric counting"));
    assert!(prompt.contains("Do not emit exact_format, required_evidence, fields"));
    assert!(prompt.contains("instead of inventing enum values"));
    assert!(prompt.contains("Every enum field must be exactly one listed schema token"));
    assert!(prompt.contains("clarify is a decision, never a turn_type or resume_behavior"));
    assert!(prompt.contains("state_patch must be a JSON object or null"));
    assert!(prompt.contains("Use decision=\"planner_execute\" when the request inspects"));
    assert!(prompt.contains("generic baseline diagnostics"));
    assert!(prompt.contains("semantic_kind=\"service_status\""));
    assert!(prompt.contains("Never ask the user to paste local file contents"));
    assert!(prompt.contains("Output exactly one raw JSON object and then stop"));
    assert!(prompt.contains("Normalizer protocol is internal only"));
    assert!(prompt.contains("Inline-data transform invariant"));
    assert!(prompt.contains("Always include all top-level schema keys"));
    assert!(prompt.contains("If ACTIVE_TASK is <none>, do not use task_append"));
    assert!(prompt.contains("turn_type=\"task_append\", target_task_policy=\"reuse_active\""));
    assert!(prompt.contains("never force planner_execute for a presentation-only constraint"));
    assert!(prompt.contains("Current REQUEST overrides RECENT/MEMORY"));
    assert!(prompt.contains("Do not import a prior directory/path scope"));
    assert!(prompt.contains("Fresh unresolved deictic filesystem targets are missing locators"));
    assert!(prompt.contains(
        "Do not resolve a fresh deictic filesystem/log/document target from MEMORY alone"
    ));
}

#[test]
fn compact_prompt_slot_preserves_head_and_tail_when_truncated() {
    let value = format!(
            "project background: {}\nvalidation goal: continuous state memory context should remain usable",
            "long middle context ".repeat(80)
        );
    let slot = super::compact_prompt_slot("MEMORY", &value, 180);

    assert!(slot.contains("MEMORY: project background"));
    assert!(slot.contains("...<snip>..."));
    assert!(slot.contains("validation goal:"));
    assert!(slot.contains("state memory context"));
}

#[test]
fn compact_normalizer_prompt_keeps_followup_anchor_next_to_request_tail() {
    let route_view = crate::task_context_builder::RouteContextView {
            active_execution_anchor_context:
                "### ACTIVE_EXECUTION_ANCHOR\nfollowup_source_request: list logs\nfollowup_ordered_entries: 1:act_plan.log | 2:clawd.log | 3:clawd.run.log"
                    .to_string(),
            memory_context: "older document list memory ".repeat(160),
            recent_assistant_replies: "older assistant document list ".repeat(160),
            ..Default::default()
        };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        "inspect the second item from the latest list",
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

    assert!(compact_tail.contains("FOLLOWUP_ANCHOR_PRIORITY"));
    assert!(compact_tail.contains("RUNTIME_STATUS"));
    assert!(compact_tail.contains("followup_ordered_entries"));
    assert!(compact_tail.contains("2:clawd.log"));
    assert!(compact_tail.contains("REQUEST: inspect the second item from the latest list"));
}

#[test]
fn compact_normalizer_prompt_keeps_summary_recall_guard_in_head_and_tail() {
    let route_view = crate::task_context_builder::RouteContextView {
        recent_turns_full: "recent turn noise ".repeat(120),
        memory_context: "memory noise ".repeat(120),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "请用一句话总结这个连续会话测试主要验证什么。";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        request,
    );
    let compact_head = crate::providers::utf8_safe_prefix(&prompt, 1485);
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

    assert!(compact_head.contains("High-priority"));
    assert!(compact_head.contains("mainly verifies or means"));
    assert!(compact_tail.contains("SUMMARY_RECALL"));
    assert!(compact_tail.contains(request));
}

#[test]
fn compact_normalizer_prompt_tail_preserves_memory_recall_near_request() {
    let test_id = "client-like-continuous-20260430_134427";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("STABLE_FACTS: test number is {test_id}"),
        recent_turns_full: "recent turn noise ".repeat(120),
        last_turn_full: "last turn noise ".repeat(40),
        recent_assistant_replies: "assistant noise ".repeat(20),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "刚才我让你记住的测试编号是什么？只回答编号。";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1485);

    assert!(compact_tail.contains(test_id));
    assert!(compact_tail.contains(request));
    assert!(compact_tail.find("MEMORY:").is_some_and(|memory_idx| {
        compact_tail
            .find("REQUEST:")
            .is_some_and(|request_idx| memory_idx < request_idx)
    }));
}

#[test]
fn compact_normalizer_prompt_tail_keeps_assistant_scalar_and_marks_scores_metadata() {
    let test_id = "client-like-continuous-20260430_174102";
    let route_view = crate::task_context_builder::RouteContextView {
            memory_context: "### MEMORY_CONTEXT\n#### RECENT_RELATED_EVENTS\n- 0.55 user asked to remember a long context\n- 0.70 unrelated relevance score".to_string(),
            recent_assistant_replies: format!(
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] relative_index=-1 short_preview=已收到 has_code_block=false\n- turn_id=assistant[-2] relative_index=-2 short_preview=已记录。测试编号 `{test_id}` 已记住，后续询问时可直接使用。 has_code_block=false"
            ),
            recent_turns_full: "recent turn noise ".repeat(120),
            last_turn_full: "last turn noise ".repeat(40),
            ..Default::default()
        };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "刚才我让你记住的测试编号是什么？只回答编号。";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "zh-CN",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1815);

    assert!(compact_tail.contains("memory scores are metadata"));
    assert!(compact_tail.contains("ASSISTANT:"));
    assert!(compact_tail.contains(test_id));
    assert!(compact_tail.contains(request));
    assert!(compact_tail.find("MEMORY:").is_some_and(|memory_idx| {
        compact_tail
            .find("ASSISTANT:")
            .is_some_and(|assistant_idx| memory_idx < assistant_idx)
    }));
    assert!(compact_tail
        .find("ASSISTANT:")
        .is_some_and(|assistant_idx| {
            compact_tail
                .find("REQUEST:")
                .is_some_and(|request_idx| assistant_idx < request_idx)
        }));
}

#[test]
fn compact_normalizer_prompt_tail_preserves_long_memory_goal_near_request() {
    let goal = "validation goal: continuous messages should keep recent turns, memory context, and clarification state usable";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!(
            "project background: {}\n{goal}",
            "multi-channel agent console context ".repeat(80)
        ),
        recent_turns_full: "recent turn noise ".repeat(120),
        last_turn_full: "last turn noise ".repeat(40),
        recent_assistant_replies: "assistant noise ".repeat(20),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "Please summarize what this continuous conversation test validates.";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true current_process_cwd=/home/guagua/rustclaw",
        "en",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

    assert!(compact_tail.contains("MEMORY:"));
    assert!(compact_tail.contains("validation goal:"));
    assert!(compact_tail.contains("clarification state usable"));
    assert!(compact_tail.contains(request));
}

#[test]
fn compact_normalizer_prompt_tail_preserves_runtime_context_near_request() {
    let route_view = crate::task_context_builder::RouteContextView {
        recent_turns_full: "recent turn noise ".repeat(120),
        last_turn_full: "last turn noise ".repeat(40),
        recent_assistant_replies: "assistant noise ".repeat(20),
        memory_context: "memory noise ".repeat(40),
        ..Default::default()
    };
    let runtime_context = "### RUNTIME_CONTEXT\ncurrent_process_cwd: /tmp/rustclaw-workspace\nworkspace_root: /tmp/rustclaw-workspace";
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: Some(crate::task_context_builder::ExecutionContextView {
            budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Full,
            memory_ctx: crate::memory::service::PromptMemoryContext {
                prompt_with_memory: String::new(),
                chat_prompt_context: String::new(),
                memory_trace: None,
                long_term_summary: None,
                preferences: Vec::new(),
                recalled: Vec::new(),
                similar_triggers: Vec::new(),
                relevant_facts: Vec::new(),
                recent_related_events: Vec::new(),
            },
            runtime_context: runtime_context.to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };
    let request = "只输出当前工作目录的绝对路径，不要解释";
    let prompt = super::render_compact_intent_normalizer_prompt(
        &route_view,
        &context_bundle,
        "admin=true",
        "zh-CN",
        request,
    );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

    assert!(prompt.contains("CONTRACT: output_contract must be a JSON object"));
    assert!(compact_tail.contains("LOCAL_EXEC"));
    assert!(compact_tail.contains("no cannot-access-FS reply"));
    assert!(compact_tail.contains("RUNTIME:"));
    assert!(compact_tail.contains("current_process_cwd: /tmp/rustclaw-workspace"));
    assert!(compact_tail.contains("workspace_root: /tmp/rustclaw-workspace"));
    assert!(compact_tail.contains(request));
    assert!(compact_tail.find("RUNTIME:").is_some_and(|runtime_idx| {
        compact_tail
            .find("REQUEST:")
            .is_some_and(|request_idx| runtime_idx < request_idx)
    }));
}

#[test]
fn compact_normalizer_prompt_falls_back_to_auth_runtime_context() {
    let route_view = crate::task_context_builder::RouteContextView {
        recent_turns_full: "recent turn noise ".repeat(120),
        memory_context: "memory noise ".repeat(40),
        ..Default::default()
    };
    let context_bundle = crate::task_context_builder::TaskContextBundle {
        raw_sources: crate::task_context_builder::TaskContextRawSources::default(),
        planner_view: crate::task_context_builder::PlannerContextView::default(),
        route_view: Some(route_view.clone()),
        execution_view: None,
    };
    let request = "只输出当前工作目录的绝对路径，不要解释";
    let prompt = super::render_compact_intent_normalizer_prompt(
            &route_view,
            &context_bundle,
            "current_auth_role: admin\nallow_path_outside_workspace_for_task: true\nworkspace_root: /home/guagua/rustclaw\ncurrent_process_cwd: /home/guagua/rustclaw",
            "zh-CN",
            request,
        );
    let compact_tail = crate::providers::utf8_safe_suffix(&prompt, 1700);

    assert!(compact_tail.contains("RUNTIME:"));
    assert!(compact_tail.contains("current_process_cwd: /home/guagua/rustclaw"));
    assert!(compact_tail.contains("workspace_root: /home/guagua/rustclaw"));
    assert!(!compact_tail.contains("RUNTIME: <none>"));
    assert!(compact_tail.contains(request));
}

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
fn normalizer_schema_normalization_recovers_planner_decision_and_filename_listing_contract() {
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
        Some("planner_execute")
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
        Some("file_names")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_recovers_files_listing_contract_from_chat_drift() {
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
    assert_eq!(contract.semantic_kind, crate::OutputSemanticKind::FileNames);
    assert!(contract.requires_content_evidence);
    assert_eq!(
        super::parse_first_layer_decision_text(&validated.decision),
        Some(crate::FirstLayerDecision::PlannerExecute)
    );
}

#[test]
fn executable_unknown_scalar_output_contract_triggers_semantic_repair_not_answer_candidate() {
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
    assert!(report
        .details
        .contains("executable_route_unknown_scalar_output_contract"));
    assert!(report.needs_llm_semantic_repair());
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

#[test]
fn structural_contract_repair_routes_file_field_scalar_to_evidence() {
    let req = "读取 Cargo.toml 的 package.name，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert!(
        matches!(
            reason,
            Some("structured_file_scalar_repair") | Some("scalar_locator_requires_evidence")
        ),
        "unexpected repair reason: {reason:?}"
    );
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "Cargo.toml");
}

#[test]
fn structural_contract_repair_preserves_directory_scoped_scalar_path_lookup() {
    let req =
        "In scripts/nl_tests/fixtures/locator_smart/case_only, where's report.md? only the path";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("structured_file_scalar_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
}

#[test]
fn structural_contract_repair_keeps_workspace_summary_on_workspace_root_name() {
    let req = "把 RustClaw 当成当前项目来介绍，先查证 README 和 Cargo.toml";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        locator_hint: "RustClaw".to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let workspace_root = std::path::Path::new("/tmp/rustclaw");
    let _ = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "RustClaw");
}

#[test]
fn structural_contract_repair_preserves_chat_workspace_name_without_evidence() {
    let req = "用一句话介绍 RustClaw 是什么，不要查询文件";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/tmp/rustclaw"),
        FirstLayerDecision::DirectAnswer,
        "RustClaw 是一个面向自然语言自动化的本地 agent 项目。",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn structural_contract_repair_preserves_file_path_only_delivery() {
    let req =
            "Run pwd, write one short line based on it into pwd_line.txt, and output only the file path.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "pwd_line.txt".to_string(),
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("scalar_locator_requires_evidence"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "pwd_line.txt");
}

#[test]
fn structural_contract_repair_promotes_file_token_delivery_to_generated_artifact() {
    let req =
        "创建一个文本文件到 tmp/对抗测试_笔记.txt，内容是「adversarial v1」，然后把文件发给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("file_token_delivery_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_contract_repair_downgrades_filename_only_generated_delivery_to_existing_file() {
    let req = "把 definitely_missing_named_file_golden_001.txt 发给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("generated_file_delivery_filename_only_existing_target_repair")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(
        contract.locator_hint,
        "definitely_missing_named_file_golden_001.txt"
    );
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_contract_repair_converts_existing_generated_delivery_with_counted_summary() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let target = workspace_root.join("README.md");
    assert!(target.is_file());
    let req = format!(
        "把 {path} 发给我，并用一句话说明它主要是做什么的",
        path = target.display()
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_hint: target.display().to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        &req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("generated_file_delivery_existing_content_summary_repair")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentExcerptWithSummary
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.exact_sentence_count, Some(1));
    assert!(contract.requires_content_evidence);
}

#[test]
fn semantic_contract_repair_ignores_invented_answer_candidate_for_observation() {
    let req = "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "没有 (路径未找到)",
        None,
        None,
    );

    assert_eq!(reason, Some("semantic_contract_requires_evidence"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "rustclaw.service");
}

#[test]
fn semantic_contract_repair_promotes_empty_path_locator_for_multi_path_facts() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new("/workspace");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "/workspace");
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
}

#[test]
fn semantic_contract_repair_replaces_combined_path_hint_for_multi_path_facts() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new("/workspace");
    let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "scripts/nl_tests/fixtures/device_local/package.json, scripts/nl_tests/fixtures/device_local/nope.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        };

    super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "/workspace");
}

#[test]
fn scalar_file_contract_repair_ignores_invented_answer_candidate() {
    let req = "读取 package.json 里的 name 字段，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "rustclaw",
        None,
        None,
    );

    assert_eq!(reason, Some("scalar_locator_requires_evidence"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn dotted_structured_field_repair_overrides_structured_keys_contract() {
    let req =
        "读取 scripts/nl_tests/fixtures/device_local/configs/app_config.toml 中 app.name，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/configs/app_config.toml".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_field_selector_requires_scalar_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
    );
}

#[test]
fn dotted_structured_field_repair_overrides_config_validation_contract() {
    let req = "读取 configs/config.toml 中 skills.skill_switches.config_basic 的值；若该字段不存在，说明它未显式配置";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::ConfigValidation,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("config_validation_field_selector_requires_scalar_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn structured_config_keys_repair_overrides_file_names_contract() {
    let req = "读取 configs/config.toml 的顶层键名，只输出键名列表";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("structured_config_keys_overrides_file_names"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::StructuredKeys);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn structured_identifier_presence_repair_overrides_file_existence_contract() {
    let req = "Read docker/config/skills_registry.toml and answer whether fs_basic is registered.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "docker/config/skills_registry.toml".to_string(),
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_identifier_presence_requires_content_evidence")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "docker/config/skills_registry.toml");
}

#[test]
fn structured_identifier_presence_repair_overrides_config_validation_contract() {
    let req = "Read docker/config/skills_registry.toml and answer whether fs_basic is registered.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "docker/config/skills_registry.toml".to_string(),
        semantic_kind: OutputSemanticKind::ConfigValidation,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_identifier_presence_requires_content_evidence")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "docker/config/skills_registry.toml");
}

#[test]
fn scalar_structured_keys_contract_repairs_to_field_value_contract() {
    let req = "去 package.json 里把项目名找出来，只把 name 的值回给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "package.json".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_keys_scalar_response_requires_field_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn current_workspace_scalar_structured_keys_contract_repairs_to_field_value_contract() {
    let req = "package.json 里的 name 到底是什么，只给值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "package.json".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_keys_scalar_response_requires_field_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn planner_locator_contract_repair_requires_evidence_for_sparse_contract() {
    let req = "Read configs/config.toml and output the selected_vendor field and value";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("planner_locator_requires_evidence"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn inline_json_payload_context_is_not_repaired_as_path_content() {
    let req = r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("inline_structured_payload_context_execute"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

#[test]
fn inline_json_transform_repairs_misclassified_content_excerpt_contract() {
    let req = r#"Sort this JSON array by score descending and output only a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::ContentExcerptWithSummary,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("inline_structured_transform_contract_repair"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
}

#[test]
fn scalar_direct_answer_candidate_is_not_promoted_by_filename_like_text() {
    let req = "Literal text: app.log, answer only acknowledged.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        FirstLayerDecision::DirectAnswer,
        "acknowledged",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn answer_candidate_existing_relative_path_routes_to_evidence() {
    let workspace = make_temp_workspace_with_child("answer_candidate_path", "docs");
    let relative = "docs/report.md";
    std::fs::write(workspace.join(relative), "report").expect("write report");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_answer_candidate_path_evidence_repair(
        &mut contract,
        &format!("`{relative}`"),
        None,
        &workspace,
        false,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("answer_candidate_path_requires_evidence"));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, relative);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
    std::fs::remove_dir_all(workspace).ok();
}

#[test]
fn answer_candidate_existing_bare_filename_stays_chat_without_evidence_repair() {
    let workspace = make_temp_workspace_with_child("answer_candidate_bare_filename", "docs");
    std::fs::write(workspace.join("README.md"), "readme").expect("write readme");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_answer_candidate_path_evidence_repair(
        &mut contract,
        "README.md",
        None,
        &workspace,
        false,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    std::fs::remove_dir_all(workspace).ok();
}

#[test]
fn answer_candidate_plain_scalar_stays_chat_without_evidence() {
    let workspace = make_temp_workspace_with_child("answer_candidate_scalar", "docs");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_answer_candidate_path_evidence_repair(
        &mut contract,
        "my_abcd",
        None,
        &workspace,
        false,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    std::fs::remove_dir_all(workspace).ok();
}

#[test]
fn deictic_filename_answer_candidate_stays_chat_without_path_evidence_repair() {
    let workspace = make_temp_workspace_with_child("answer_candidate_deictic", "docs");
    std::fs::write(workspace.join("README.md"), "readme").expect("write readme");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let patch = serde_json::json!({"output_format":"filename_only"});

    let reason = super::apply_answer_candidate_path_evidence_repair(
        &mut contract,
        "README.md",
        Some(&patch),
        &workspace,
        false,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    std::fs::remove_dir_all(workspace).ok();
}

#[test]
fn structural_contract_repair_does_not_bind_workspace_child_mentions() {
    let workspace_root = make_temp_workspace_with_child("workspace_child_mentions", "document");
    let req = "列出document目录下有哪些文件，只输出文件名列表";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        &workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn structural_contract_repair_does_not_bind_case_mismatched_product_name() {
    let workspace_root = make_temp_workspace_with_child("workspace_child_product_name", "rustclaw");
    let req = "你好，我正在做 RustClaw 的真实客户端连续会话测试，请用一句中文回复确认。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        &workspace_root,
        FirstLayerDecision::DirectAnswer,
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn executionless_chat_wrapped_execute_is_downgraded_to_direct_answer() {
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };

    let reason = super::downgrade_executionless_route_to_direct_answer(
        &mut decision,
        &mut finalize_style,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
    );

    assert_eq!(
        reason,
        Some("executionless_route_downgraded_to_direct_answer")
    );
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
}

#[test]
fn explicit_command_execution_repair_prevents_executionless_downgrade() {
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: vec![],
        execute_prefixes: vec!["请执行".to_string()],
        standalone_commands: vec![],
        default_locale: "zh-CN".to_string(),
        verify_enforce_enabled: true,
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/home/guagua/rustclaw".to_string(),
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        &runtime,
        "请执行 git rev-parse --abbrev-ref HEAD，只输出命令结果",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut decision,
        &mut finalize_style,
    );
    let downgrade = super::downgrade_executionless_route_to_direct_answer(
        &mut decision,
        &mut finalize_style,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert_eq!(downgrade, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_preserves_failed_step_contract() {
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: vec![],
        execute_prefixes: vec!["执行".to_string()],
        standalone_commands: vec![],
        default_locale: "zh-CN".to_string(),
        verify_enforce_enabled: true,
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::ExecutionFailedStep,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
            &runtime,
            "执行一个会失败的只读检查命令：cat /definitely_missing_rustclaw_contract_case，然后说明失败原因。",
            &mut needs_clarify,
            &mut clarify_question,
            &mut contract,
            &mut decision,
            &mut finalize_style,
        );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExecutionFailedStep
    );
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_respects_pure_direct_answer_contract() {
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: vec![],
        execute_prefixes: vec!["execute".to_string()],
        standalone_commands: vec![],
        default_locale: "en-US".to_string(),
        verify_enforce_enabled: true,
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        &runtime,
        "execute ls -la: explain what this command means, do not run it",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(repair, None);
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_clears_spurious_clarify() {
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: vec![],
        execute_prefixes: vec!["请执行".to_string(), "执行".to_string()],
        standalone_commands: vec![],
        default_locale: "zh-CN".to_string(),
        verify_enforce_enabled: true,
    };
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        &runtime,
        "请执行 pwd，只输出命令结果",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn embedded_standalone_command_execution_repair_clears_spurious_clarify() {
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: vec![],
        execute_prefixes: vec!["执行".to_string()],
        standalone_commands: vec!["pwd".to_string()],
        default_locale: "zh-CN".to_string(),
        verify_enforce_enabled: true,
    };
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        &runtime,
        "运行 pwd -P，只返回物理工作目录路径",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn command_payload_contract_repair_clears_spurious_locatorless_clarify() {
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_command_payload_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("command_payload_requires_raw_output_execution")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn raw_output_explicit_locator_repair_restores_path_for_non_command_read() {
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: Vec::new(),
        execute_prefixes: vec!["run ".to_string()],
        standalone_commands: vec!["pwd".to_string()],
        default_locale: "zh-CN".to_string(),
        verify_enforce_enabled: true,
    };
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_raw_output_explicit_locator_repair(
        &mut contract,
        "读 /etc/shadow 第一行，告诉我里面是什么",
        &runtime,
    );

    assert_eq!(repair, Some("raw_output_explicit_locator_contract_repair"));
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "/etc/shadow");
}

#[test]
fn raw_output_explicit_locator_repair_skips_literal_command_requests() {
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: Vec::new(),
        execute_prefixes: vec!["run ".to_string()],
        standalone_commands: vec!["pwd".to_string()],
        default_locale: "en-US".to_string(),
        verify_enforce_enabled: true,
    };
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_raw_output_explicit_locator_repair(
        &mut contract,
        "run cat /etc/shadow",
        &runtime,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn plain_execute_is_not_downgraded_when_contract_is_sparse() {
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };

    let reason = super::downgrade_executionless_route_to_direct_answer(
        &mut decision,
        &mut finalize_style,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
}

#[test]
fn execution_signal_act_route_stays_executable() {
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };

    let reason = super::downgrade_executionless_route_to_direct_answer(
        &mut decision,
        &mut finalize_style,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
}

#[test]
fn structured_observation_clarify_repair_routes_concrete_file_request_to_act() {
    let req = "读取 package.json 里的 name 字段，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供 package.json 文件内容".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn structured_observation_clarify_repair_fills_file_delivery_filename_locator() {
    let req = "把 definitely_missing_named_file_phase0_runtime_20260515.txt 发给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question =
        "请提供 definitely_missing_named_file_phase0_runtime_20260515.txt 的完整路径".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(
        contract.locator_hint,
        "definitely_missing_named_file_phase0_runtime_20260515.txt"
    );
    assert!(contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_routes_multi_filename_request_to_workspace_act() {
    let req = "检查 README.md, README.zh-CN.md, Cargo.toml, and no_such_file_20260513.txt 是否存在，用表格返回";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供具体的文件夹路径".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, workspace_root.display().to_string());
}

#[test]
fn workspace_default_observation_clarify_repair_routes_listing_without_absolute_path_to_act() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Please provide the full UI directory path.".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_workspace_default_observation_clarify_repair(
        &mut contract,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("workspace_default_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, workspace_root.display().to_string());
}

#[test]
fn workspace_default_observation_clarify_repair_routes_docker_contract_to_act() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::DockerContainerLifecycle,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_workspace_default_observation_clarify_repair(
        &mut contract,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("workspace_default_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::DockerContainerLifecycle
    );
}

#[test]
fn structured_contract_hint_repair_recovers_git_contract_without_nl_matching() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "检查这个仓库当前是否有未提交改动，用一句话说明。\n",
        "[CONTRACT_TEST_HINT]\n",
        "contract_id=git_repository_state\n",
        "semantic_kind=git_repository_state\n",
        "required_evidence_json=[\"field_value\"]\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: workspace_root.display().to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let surface_req = super::request_without_contract_test_hint(req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut wants_file_delivery = false;
    let reason = super::apply_structured_contract_hint_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        &mut wants_file_delivery,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_contract_hint_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GitRepositoryState
    );
    assert!(contract.requires_content_evidence);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(!needs_clarify);
}

#[test]
fn structured_contract_hint_repair_keeps_package_manager_locatorless() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "检测这台机器可用的包管理器，并说明依据。\n",
        "[CONTRACT_TEST_HINT]\n",
        "contract_id=package_manager_detection\n",
        "semantic_kind=package_manager_detection\n",
        "required_evidence_json=[\"field_value\"]\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "model-supplied-background-locator".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let surface_req = super::request_without_contract_test_hint(req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut wants_file_delivery = false;
    let reason = super::apply_structured_contract_hint_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        &mut wants_file_delivery,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_contract_hint_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::PackageManagerDetection
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
}

#[test]
fn structured_contract_hint_repair_sets_generated_file_delivery_defaults() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
            "写一个简单文本文件到 tmp/contract_matrix_generated_note.txt，内容是 RustClaw contract matrix test，然后把文件路径发给我。\n",
            "[CONTRACT_TEST_HINT]\n",
            "contract_id=generated_file_delivery\n",
            "semantic_kind=generated_file_delivery\n",
            "required_evidence_json=[\"path\"]\n",
            "[/CONTRACT_TEST_HINT]"
        );
    let surface_req = super::request_without_contract_test_hint(req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: false,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };
    let mut wants_file_delivery = false;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要发送的文件路径或文件名。".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_structured_contract_hint_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        &mut wants_file_delivery,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_contract_hint_repair"));
    assert!(wants_file_delivery);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert!(contract
        .locator_hint
        .contains("tmp/contract_matrix_generated_note.txt"));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
}

#[test]
fn request_without_contract_test_hint_removes_machine_block() {
    let req = "检测包管理器。\n[CONTRACT_TEST_HINT]\nsemantic_kind=package_manager_detection\n[/CONTRACT_TEST_HINT]\n谢谢";
    let stripped = super::request_without_contract_test_hint(req);
    assert!(stripped.contains("检测包管理器"));
    assert!(stripped.contains("谢谢"));
    assert!(!stripped.contains("CONTRACT_TEST_HINT"));
    assert!(!stripped.contains("[/"));
}

#[test]
fn current_turn_contract_repair_does_not_path_bind_package_manager_hint() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let raw_req = concat!(
        "检测这台机器可用的包管理器。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=package_manager_detection\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface_req = super::request_without_contract_test_hint(raw_req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::PackageManagerDetection,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };
    let _ = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        &surface_req,
        &surface,
        workspace_root,
        FirstLayerDecision::PlannerExecute,
        "",
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::Standalone),
    );

    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::PackageManagerDetection
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn recent_scalar_equality_requires_fresh_evidence() {
    assert!(super::output_semantic_kind_requires_fresh_evidence(
        OutputSemanticKind::RecentScalarEqualityCheck
    ));
}

#[test]
fn contract_hint_fallback_recovers_git_route_without_nl_tokens() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "处理这个请求。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=git_repository_state\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    )
    .expect("contract hint fallback");

    assert!(!decision.needs_clarify);
    assert_eq!(
        decision.output_contract.semantic_kind,
        OutputSemanticKind::GitRepositoryState
    );
    assert_eq!(
        decision.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        decision.output_contract.locator_hint,
        workspace_root.display().to_string()
    );
}

#[test]
fn contract_hint_fallback_keeps_package_manager_locatorless() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "处理这个请求。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=package_manager_detection\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    )
    .expect("contract hint fallback");

    assert!(!decision.needs_clarify);
    assert_eq!(
        decision.output_contract.semantic_kind,
        OutputSemanticKind::PackageManagerDetection
    );
    assert_eq!(
        decision.output_contract.locator_kind,
        OutputLocatorKind::None
    );
    assert!(decision.output_contract.locator_hint.is_empty());
}

#[test]
fn contract_hint_fallback_extracts_path_locator() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "处理 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=content_excerpt_summary\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    )
    .expect("contract hint fallback");

    assert!(!decision.needs_clarify);
    assert_eq!(
        decision.output_contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        decision.output_contract.locator_kind,
        OutputLocatorKind::Path
    );
    assert!(decision
        .output_contract
        .locator_hint
        .contains("release_checklist.md"));
}

#[test]
fn contract_hint_workspace_summary_preserves_explicit_directory_locator() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
            "快速看一下 scripts/nl_tests/fixtures/device_local，用非技术用户能听懂的话总结它是什么项目。\n",
            "[CONTRACT_TEST_HINT]\n",
            "semantic_kind=workspace_project_summary\n",
            "[/CONTRACT_TEST_HINT]"
        );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    )
    .expect("contract hint fallback");

    assert!(!decision.needs_clarify);
    assert_eq!(
        decision.output_contract.semantic_kind,
        OutputSemanticKind::WorkspaceProjectSummary
    );
    assert_eq!(
        decision.output_contract.locator_kind,
        OutputLocatorKind::Path
    );
    assert!(decision
        .output_contract
        .locator_hint
        .contains("scripts/nl_tests/fixtures/device_local"));
}

#[test]
fn structured_observation_clarify_repair_routes_two_explicit_targets_to_act() {
    let req = "比较 configs/skills_registry.toml 和 docker/config/skills_registry.toml 哪个文件更大，只回答文件名和大小差";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "您希望我执行文件大小比较操作吗？".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "configs/skills_registry.toml, docker/config/skills_registry.toml"
    );
}

#[test]
fn resolved_directory_observation_clarify_repair_routes_existing_workspace_dir_to_act() {
    let workspace_root = make_temp_workspace_with_child("resolved_dir_clarify", "docs");
    std::fs::write(workspace_root.join("docs").join("a.md"), "alpha").expect("write a");
    std::fs::write(workspace_root.join("docs").join("b.md"), "beta").expect("write b");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    let req =
            "List the two largest files directly under docs and say what kind of docs they appear to be.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Should I use document or docs?".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_resolved_directory_observation_clarify_repair(
        &state,
        &mut contract,
        req,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(
        reason,
        Some("resolved_directory_observation_clarify_repair")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        workspace_root
            .join("docs")
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn resolved_directory_observation_clarify_repair_preserves_non_locator_semantics() {
    let workspace_root = make_temp_workspace_with_child("resolved_dir_non_locator", "target");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    let req = "查看那个 schema 里的 target enum";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要查看的 schema 文件的路径或名称。".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_resolved_directory_observation_clarify_repair(
        &state,
        &mut contract,
        req,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.locator_hint.is_empty());
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn resolved_directory_observation_clarify_repair_preserves_bare_locator_only_reply() {
    let workspace_root = make_temp_workspace_with_child("resolved_dir_bare", "docs");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    let req = "docs";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "What should I do with docs?".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_resolved_directory_observation_clarify_repair(
        &state,
        &mut contract,
        req,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(clarify_question, "What should I do with docs?");
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn unbound_workspace_generic_content_repair_clarifies_short_topic() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = "opaquetopic";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: workspace_root.display().to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_unbound_workspace_generic_content_clarify_repair(
        &mut contract,
        req,
        &surface,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(
        reason,
        Some("unbound_workspace_generic_content_requires_clarify")
    );
    assert!(needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn unbound_workspace_generic_content_repair_preserves_structured_semantic() {
    let req = "opaquetopic";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::QuantityComparison,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/workspace".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_unbound_workspace_generic_content_clarify_repair(
        &mut contract,
        req,
        &surface,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::QuantityComparison
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "/workspace");
}

#[test]
fn unbound_workspace_generic_content_repair_preserves_concrete_locator_surface() {
    let req = "Cargo.toml";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/workspace".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_unbound_workspace_generic_content_clarify_repair(
        &mut contract,
        req,
        &surface,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn structured_observation_clarify_repair_preserves_named_target_without_clean_locator() {
    let req = "读一下 README 然后用恰好三句话总结，不要多也不要少";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供 README 的具体内容或文件路径".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供 README 的具体内容或文件路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_deictic_bare_target_clarify() {
    let req = "读一下那个 README 开头并用一句话总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请确认具体 README 路径".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let patch = serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}});

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        Some(&patch),
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请确认具体 README 路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_unbound_scope_filename_target() {
    let req = "去那个 case_only 目录里找 report.md，只输出路径";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供 case_only 目录的完整路径".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供 case_only 目录的完整路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn structured_observation_clarify_repair_preserves_version_correction_clarify() {
    let req = "Correction: mention Python 3.11, not Python 3.10.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请确认要修正哪段内容".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请确认要修正哪段内容");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_deictic_with_destination_path_clarify() {
    let req = "把那个压缩包解压到 /tmp/unpack_dest 然后告诉我结果";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/unpack_dest".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供压缩包路径".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let patch = serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}});

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        Some(&patch),
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供压缩包路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_deictic_destination_without_patch() {
    let req = "把那个压缩包解压到 /tmp/unpack_dest 然后告诉我结果";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供压缩包路径".to_string();
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供压缩包路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn archive_unpack_missing_archive_locator_forces_clarify_even_with_destination_path() {
    let req = "extract the referenced archive into /tmp/unpack_dest and report the result";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/unpack_dest".to_string(),
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_archive_unpack_missing_archive_locator_clarify(
        &mut contract,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(
        reason,
        Some("archive_unpack_missing_archive_locator_clarify")
    );
    assert!(needs_clarify);
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert!(contract.requires_content_evidence);
}

#[test]
fn archive_unpack_missing_archive_locator_allows_structural_archive_pair() {
    let req = "extract tmp/test_bundle.zip into /tmp/unpack_dest and report the result";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "tmp/test_bundle.zip | /tmp/unpack_dest".to_string(),
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_archive_unpack_missing_archive_locator_clarify(
        &mut contract,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut decision,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(
        contract.locator_hint,
        "tmp/test_bundle.zip | /tmp/unpack_dest"
    );
}

#[test]
fn archive_pack_pair_repairs_generated_file_delivery_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/docs 打包成 tmp/contract_matrix_docs_bundle.zip，并告诉我生成路径。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/docs".to_string(),
            "tmp/contract_matrix_docs_bundle.zip".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_pack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchivePack);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs | tmp/contract_matrix_docs_bundle.zip"
    );
}

#[test]
fn archive_pack_pair_repairs_scalar_path_only_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/docs 打包成 tmp/contract_matrix_docs_bundle.zip，并告诉我生成路径。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_pack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchivePack);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs | tmp/contract_matrix_docs_bundle.zip"
    );
}

#[test]
fn archive_unpack_pair_repairs_generated_file_delivery_contract() {
    let req = "把 tmp/contract_matrix_docs_bundle.zip 解压到 tmp/contract_matrix_unpacked，并告诉我结果。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "tmp/contract_matrix_docs_bundle.zip".to_string(),
            "tmp/contract_matrix_unpacked".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "tmp/contract_matrix_docs_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_unpack_pair_repairs_generic_path_content_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
            "tmp/contract_matrix_unpacked".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_unpack_pair_repairs_content_excerpt_drift_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_unpack_pair_repairs_policy_suffix_contract() {
    let req = concat!(
            "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。",
            "\n[CONTRACT_TEST_HINT]\n",
            "candidate_wrong_action_ref=fs_basic.write_text\n",
            "policy_expectation=runtime_must_reject_or_replace_disallowed_action\n",
            "[/CONTRACT_TEST_HINT]"
        );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
            "tmp/contract_matrix_unpacked".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_read_member_repairs_content_excerpt_drift_contract() {
    let req = concat!(
            "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 里的 notes.txt 内容片段，并简短总结。",
            "\n[CONTRACT_TEST_HINT]\n",
            "contract_id=archive_read\n",
            "semantic_kind=archive_read\n",
            "preferred_action_ref=archive_basic.read\n",
            "[/CONTRACT_TEST_HINT]"
        );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert!(surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate == "test_bundle.zip"));
    assert!(surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate == "notes.txt"));
    assert!(!surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate.contains("archive_basic.read")));

    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt"
    );
}

#[test]
fn archive_read_member_pair_is_not_treated_as_unpack_destination() {
    let req = "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 中 notes.txt 的内容片段并简短总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt"
    );
}

#[test]
fn archive_read_member_repairs_archive_unpack_drift_contract() {
    let req = "从压缩包 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 中提取 notes.txt 文件内容并简短总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt"
    );
}

#[test]
fn archive_read_nested_member_path_is_not_unpack_destination() {
    let req = "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 中 nested/config.ini 的内容片段并简短总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | nested/config.ini"
    );
}

#[test]
fn archive_read_member_repair_requires_member_candidate() {
    let req =
        "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 内容片段，并简短总结。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
    );
}

#[test]
fn archive_pair_does_not_repair_plain_observation_contract() {
    let req =
        "比较 tmp/contract_matrix_docs_bundle.zip 和 tmp/contract_matrix_unpacked 的大小差异。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert!(surface.locator_target_pair.is_some());
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
}

#[test]
fn current_turn_locator_sanitizer_drops_contextual_path_prefix() {
    let req = "读一下 README.md 然后用恰好三句话总结，不要多也不要少";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let cleaned = super::sanitize_resolved_intent_for_current_turn_locator(
        "读取 docs/README.md 文件内容并用恰好三句话进行总结",
        req,
        &surface,
    );

    assert_eq!(cleaned.as_deref(), Some(req));
}

#[test]
fn current_turn_locator_sanitizer_ignores_bare_stem_without_extension() {
    let req = "读一下 README 然后用恰好三句话总结，不要多也不要少";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let cleaned = super::sanitize_resolved_intent_for_current_turn_locator(
        "读取 document 目录下的 README.md 文件内容并用恰好三句话进行总结",
        req,
        &surface,
    );

    assert_eq!(cleaned, None);
}

fn make_temp_workspace_with_child(test_name: &str, child_name: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "rustclaw_intent_router_{test_name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos()
    ));
    std::fs::create_dir_all(root.join(child_name)).expect("create child directory");
    root
}

#[test]
fn normalizer_schema_normalization_does_not_invent_contract_from_surface() {
    let raw = r#"{
          "resolved_user_intent": "检查当前目录是否有隐藏文件，如有则列出3个例子",
          "needs_clarify": false,
          "reason": "local hidden entries check",
          "confidence": 0.98,
          "decision":"planner_execute"
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
        Some("none")
    );
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("free")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_extracts_fenced_json() {
    let raw = r#"```json
{
  "resolved_user_intent": "检查当前目录有没有隐藏文件，只回答有或没有，并补3个例子",
  "needs_clarify": false,
  "reason": "local hidden entries check",
  "confidence": 0.95,
  "decision":"planner_execute",
  "output_contract": {
    "response_shape": "scalar",
    "requires_content_evidence": true,
    "semantic_kind": "hidden_files_example",
    "locator_kind": "current_workspace"
  }
}
```"#;
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
        Some("scalar")
    );
    assert_eq!(
        contract.get("locator_kind").and_then(|v| v.as_str()),
        Some("current_workspace")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_preserves_act_when_shape_is_descriptive() {
    let raw = r#"{
          "resolved_user_intent": "列出 logs 目录下前 10 个文件名，不读取内容",
          "needs_clarify": false,
          "reason": "workspace directory listing",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": {
            "response_shape": "list_of_strings",
            "semantic_kind": "file_names"
          },
          "action": {"tool":"list_directory","path":"logs","limit":10}
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "列出 logs 目录下的前 10 个文件名");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    assert_eq!(
        value.get("decision").and_then(|v| v.as_str()),
        Some("planner_execute")
    );
    assert_eq!(
        value.get("needs_clarify").and_then(|v| v.as_bool()),
        Some(false)
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
        Some("file_names")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_coerces_output_contract_scalar_and_aliases() {
    let raw = r#"{
          "resolved_user_intent": "列出 README.md 和 AGENTS.md，只输出文件名",
          "needs_clarify": false,
          "reason": "names-only inventory",
          "confidence": 0.9,
          "decision":"planner_execute",
          "output_contract": "file_names"
        }"#;
    let normalized =
        super::normalize_intent_normalizer_raw_for_schema(raw, "列出文件，只输出文件名");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("semantic_kind").and_then(|v| v.as_str()),
        Some("file_names")
    );

    let raw = r#"{
          "resolved_user_intent": "严格输出两行",
          "needs_clarify": false,
          "reason": "exact output",
          "confidence": 0.9,
          "decision":"direct_answer",
          "output_contract": {"shape":"exact_format","semantic":"sqlite_table_names"}
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(raw, "严格输出两行");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
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
        Some("sqlite_table_names_only")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn normalizer_schema_normalization_maps_file_type_filename_to_locator_hint() {
    let raw = r#"{
          "resolved_user_intent": "User wants to receive the file from the current workspace.",
          "needs_clarify": false,
          "reason": "file delivery",
          "confidence": 0.95,
          "decision": "planner_execute",
          "wants_file_delivery": true,
          "output_contract": {
            "type": "file",
            "filename": "definitely_missing_named_file_phase0_runtime_20260515.txt"
          }
        }"#;
    let normalized = super::normalize_intent_normalizer_raw_for_schema(
        raw,
        "把 definitely_missing_named_file_phase0_runtime_20260515.txt 发给我",
    );
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");
    let contract = value
        .get("output_contract")
        .and_then(|value| value.as_object())
        .expect("output contract");
    assert_eq!(
        contract.get("response_shape").and_then(|v| v.as_str()),
        Some("file_token")
    );
    assert_eq!(
        contract.get("delivery_required").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        contract.get("locator_kind").and_then(|v| v.as_str()),
        Some("filename")
    );
    assert_eq!(
        contract.get("locator_hint").and_then(|v| v.as_str()),
        Some("definitely_missing_named_file_phase0_runtime_20260515.txt")
    );
    crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        &normalized,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("schema validation");
}

#[test]
fn safe_fallback_tries_llm_except_when_model_unavailable() {
    assert!(!super::safe_fallback_source_should_try_llm(
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    ));
    assert!(super::safe_fallback_source_should_try_llm(
        crate::fallback::ClarifyFallbackSource::IntentUnresolved
    ));
    assert!(super::safe_fallback_source_should_try_llm(
        crate::fallback::ClarifyFallbackSource::SynthesisEmpty
    ));
}

#[test]
fn parse_execution_recipe_hint_missing_profile_falls_back_to_default_spec() {
    // 历史语义：profile 缺失 → None（曾让下游 fallback 到 keyword detect）
    // B1 修复后：normalizer 显式回了 execution_recipe 字段（即使 profile 缺）就视为
    // 已分类，返回 default spec（kind=None, inactive），不再触发本地补判。
    // 这样可以避免 legacy local detector 因 STABLE_FACTS 污染而误升级 read-only 任务。
    let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
        kind: "ops_closed_loop".to_string(),
        profile: String::new(),
        target_scope: "current_repo".to_string(),
    }))
    .expect("normalizer-classified hint should yield Some, even with missing profile");
    assert_eq!(spec.kind, ExecutionRecipeKind::None);
}

#[test]
fn parse_execution_recipe_hint_explicit_none_is_trusted() {
    // 这是修复 B1 的核心回归测试。
    // 场景：normalizer 已经基于完整上下文判定"这不是 ops loop"（kind=none）。
    // 期望：返回 Some(default spec) → initial_execution_recipe_spec 用 default spec
    // → runtime.is_active()=false → plan_repair_reason 不会触发
    // ops_closed_loop_apply_requires_mutation。
    // 历史风险：返回 None 会让下游 fallback 到 keyword 启发式；
    // 长期记忆里残留的 "configs/" "verify" 关键字会把任务误升级为
    // OpsClosedLoop config_change，让 read-only 的 `pwd` 任务跑挂。
    let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
        kind: "none".to_string(),
        profile: "none".to_string(),
        target_scope: "unknown".to_string(),
    }))
    .expect("explicit kind=none should still be Some so local fallback remains bypassed");
    assert_eq!(spec.kind, ExecutionRecipeKind::None);
    assert!(
        !crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(spec).is_active(),
        "default spec must produce an inactive runtime state"
    );
}

#[test]
fn parse_execution_recipe_hint_missing_field_leaves_no_recipe_hint() {
    // 当 normalizer 完全没在响应里给出 execution_recipe 字段时（None），
    // 只表示 LLM 没给出 recipe hint；主链不再用本地关键词检测补判。
    assert!(parse_execution_recipe_hint(None).is_none());
}

#[test]
fn fallback_normalizer_output_still_enforces_content_evidence_planner_execute() {
    let out = normalizer_output_from_fallback(
        "把当前目录有没有隐藏文件看一下",
        "parse_failed_fallback_router",
        RouteDecision {
            resolved_user_intent: "看一下当前目录有没有隐藏文件".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "current workspace executable request".to_string(),
            confidence: Some(0.72),
            schedule_kind: super::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::CurrentWorkspace,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        },
        None,
    );
    assert_eq!(out.first_layer_decision, FirstLayerDecision::PlannerExecute);
    assert!(!out.needs_clarify);
    assert_eq!(
        out.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(out.fallback_source, None);
}

#[test]
fn parse_failed_git_capability_fallback_builds_repository_state_contract() {
    let req = "只告诉我当前 git 分支名。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let fallback = super::parse_failed_explicit_capability_fallback_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
    )
    .expect("explicit git capability fallback");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::GitRepositoryState
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert!(fallback.output_contract.requires_content_evidence);
}

#[test]
fn parse_failed_git_remote_fallback_builds_repository_state_contract() {
    let req = "列出当前仓库 remote 名称和 URL";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let fallback = super::parse_failed_explicit_capability_fallback_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
    )
    .expect("explicit git remote fallback");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::GitRepositoryState
    );
    assert_eq!(
        fallback.output_contract.response_shape,
        OutputResponseShape::Strict
    );
}

#[test]
fn parse_failed_git_capability_fallback_does_not_steal_path_targets() {
    let req = "git show HEAD:README.md";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);

    assert!(super::parse_failed_explicit_capability_fallback_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
    )
    .is_none());
}

#[test]
fn inline_json_transform_fallback_builds_planner_execute_contract() {
    let req = r#"{"action":"transform_data","data":[{"team":"A","score":5},{"team":"A","score":7},{"team":"B","score":3}],"ops":[{"op":"group","by":["team"],"aggregations":[{"op":"sum","field":"score","name":"total"}]}]}"#;
    let fallback = super::inline_json_transform_fallback_decision(req)
        .expect("structured inline transform fallback");
    let out = normalizer_output_from_fallback(
        req,
        "llm_failed_inline_json_transform_fallback",
        fallback,
        None,
    );

    assert_eq!(out.first_layer_decision, FirstLayerDecision::PlannerExecute);
    assert!(!out.needs_clarify);
    assert_eq!(
        out.output_contract.response_shape,
        OutputResponseShape::Strict
    );
    assert!(out.output_contract.requires_content_evidence);
    assert_eq!(out.output_contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(out.output_contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(out.fallback_source, None);
}

#[test]
fn inline_json_transform_fallback_ignores_non_structured_text() {
    let req = "please transform the score data someday";

    assert!(super::inline_json_transform_fallback_decision(req).is_none());
}

#[test]
fn parsed_inline_json_transform_repair_builds_planner_execute_contract() {
    let req = r#"把这个 JSON 对象里的 old_name 改成 new_name，只输出 JSON：{"old_name":"alpha","count":2}"#;
    let fallback = super::parsed_inline_json_transform_repair_decision(
        req,
        true,
        FirstLayerDecision::Clarify,
        false,
        ScheduleKind::None,
        None,
    )
    .expect("parsed inline transform repair");

    assert_eq!(
        fallback.reason,
        "parsed_inline_json_transform_contract_repair"
    );
    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.response_shape,
        OutputResponseShape::Strict
    );
    assert!(fallback.output_contract.requires_content_evidence);
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::None
    );
}

#[test]
fn parsed_inline_json_transform_repair_preserves_file_delivery_route() {
    let req = r#"sort this JSON array by score descending: [{"name":"alpha","score":7}]"#;

    assert!(super::parsed_inline_json_transform_repair_decision(
        req,
        true,
        FirstLayerDecision::Clarify,
        true,
        ScheduleKind::None,
        None
    )
    .is_none());
}

#[test]
fn parsed_inline_json_transform_repair_preserves_clean_planner_route() {
    let req = r#"计算这个 JSON 数组里 value 的总和，只输出数字：[{"value":4},{"value":6}]"#;

    assert!(super::parsed_inline_json_transform_repair_decision(
        req,
        false,
        FirstLayerDecision::PlannerExecute,
        false,
        ScheduleKind::None,
        None
    )
    .is_none());
}

#[test]
fn parsed_inline_json_transform_repair_preserves_direct_non_clarify_route() {
    let req = r#"计算这个 JSON 数组里 value 的总和，只输出数字：[{"value":4},{"value":6}]"#;

    assert!(super::parsed_inline_json_transform_repair_decision(
        req,
        false,
        FirstLayerDecision::DirectAnswer,
        false,
        ScheduleKind::None,
        None
    )
    .is_none());
}

#[test]
fn directory_pair_fallback_builds_planner_execute_locator_contract() {
    let root = make_temp_workspace_with_child("directory_pair_fallback", "seed");
    std::fs::create_dir_all(root.join("fixtures/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(root.join("fixtures/tmp/dynamic_guard_unpack_case")).expect("right");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.clone();
    state.skill_rt.default_locator_search_dir = root.clone();
    let req = "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.";
    let fallback = super::directory_pair_fallback_decision(&state, req)
        .expect("resolved directory pair fallback");
    let out =
        normalizer_output_from_fallback(req, "llm_failed_directory_pair_fallback", fallback, None);

    assert_eq!(out.first_layer_decision, FirstLayerDecision::PlannerExecute);
    assert!(!out.needs_clarify);
    assert_eq!(
        out.output_contract.response_shape,
        OutputResponseShape::Strict
    );
    assert!(out.output_contract.requires_content_evidence);
    assert_eq!(out.output_contract.locator_kind, OutputLocatorKind::Path);
    assert!(out.output_contract.locator_hint.contains("bundle_src"));
    assert!(out
        .output_contract
        .locator_hint
        .contains("dynamic_guard_unpack_case"));
    assert_eq!(out.output_contract.semantic_kind, OutputSemanticKind::None);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn directory_pair_fallback_ignores_unresolved_text() {
    let state = crate::AppState::test_default_with_fixture_provider();

    assert!(super::directory_pair_fallback_decision(
        &state,
        "compare source_alpha and source_beta"
    )
    .is_none());
}

#[test]
fn explicit_surface_path_facts_fallback_builds_existence_contract() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let fallback = super::explicit_surface_path_facts_fallback_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
    )
    .expect("explicit multi-path facts fallback");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert!(fallback.output_contract.requires_content_evidence);
}

#[test]
fn explicit_surface_path_facts_clarify_repair_overrides_missing_path_clarify() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let fallback = super::explicit_surface_path_facts_clarify_repair_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
        true,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        false,
        ScheduleKind::None,
        None,
    )
    .expect("explicit multi-path clarify repair");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(
        fallback.reason,
        "normalizer_clarify_explicit_multi_path_facts"
    );
}

#[test]
fn explicit_surface_path_metadata_clarify_repair_preserves_quantity_comparison() {
    let req = "Inspect metadata for scripts/nl_tests/fixtures/device_local/package.json and scripts/nl_tests/fixtures/device_local/configs/app_config.toml.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::QuantityComparison,
        ..Default::default()
    };
    let fallback = super::explicit_surface_path_metadata_clarify_repair_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
        true,
        FirstLayerDecision::Clarify,
        &contract,
        false,
        ScheduleKind::None,
        None,
    )
    .expect("explicit metadata clarify repair");

    assert!(!fallback.needs_clarify);
    assert_eq!(
        fallback.output_contract.semantic_kind,
        OutputSemanticKind::QuantityComparison
    );
    assert_eq!(
        fallback.output_contract.locator_kind,
        OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        fallback.reason,
        "normalizer_clarify_explicit_multi_path_metadata"
    );
}

#[test]
fn explicit_surface_path_facts_clarify_repair_preserves_structured_contract() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..Default::default()
    };

    assert!(super::explicit_surface_path_facts_clarify_repair_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
        true,
        FirstLayerDecision::Clarify,
        &contract,
        false,
        ScheduleKind::None,
        None,
    )
    .is_none());
}

#[test]
fn explicit_surface_path_facts_fallback_ignores_single_path() {
    let req = "scripts/nl_tests/fixtures/device_local/package.json";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);

    assert!(super::explicit_surface_path_facts_fallback_decision(
        req,
        &surface,
        std::path::Path::new("/workspace"),
    )
    .is_none());
}

#[test]
fn workspace_scope_patch_sets_locator_hint_from_structured_scope() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        locator_hint: "/home/guagua/rustclaw".to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let applied = super::apply_workspace_scope_patch_to_contract(
        &mut contract,
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        Some(&serde_json::json!({"scope": "UI_only"})),
    );

    assert_eq!(applied.as_deref(), Some("UI"));
    assert_eq!(contract.locator_hint, "UI");
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn workspace_scope_patch_keeps_specific_locator_hint() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        locator_hint: "UI".to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let applied = super::apply_workspace_scope_patch_to_contract(
        &mut contract,
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        Some(&serde_json::json!({"scope": "pi_app_only"})),
    );

    assert_eq!(applied, None);
    assert_eq!(contract.locator_hint, "UI");
}

#[test]
fn fallback_normalizer_keeps_llm_failure_on_safe_clarify() {
    let out = normalizer_output_from_fallback(
        "read scripts/nl_tests/fixtures/device_local/package.json and output only the name field",
        "llm_failed_safe_clarify",
        RouteDecision {
            resolved_user_intent: String::new(),
            needs_clarify: true,
            clarify_question: String::new(),
            reason: "fallback_router_llm_failed".to_string(),
            confidence: None,
            schedule_kind: super::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract::default(),
        },
        Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable),
    );
    assert_eq!(out.first_layer_decision, FirstLayerDecision::Clarify);
    assert!(out.needs_clarify);
    assert!(matches!(
        out.output_contract.response_shape,
        OutputResponseShape::Free
    ));
    assert!(!out.output_contract.requires_content_evidence);
    assert!(!out.output_contract.delivery_required);
    assert!(matches!(
        out.output_contract.locator_kind,
        OutputLocatorKind::None
    ));
    assert!(matches!(
        out.output_contract.delivery_intent,
        OutputDeliveryIntent::None
    ));
    assert!(out
        .reason
        .contains("llm_failed_safe_clarify; fallback_router_llm_failed"));
    assert_eq!(
        out.fallback_source,
        Some(crate::fallback::ClarifyFallbackSource::LlmUnavailable)
    );
}

#[test]
fn clarify_question_policy_defaults_to_allow_model() {
    assert_eq!(
        ClarifyQuestionPolicy::default(),
        ClarifyQuestionPolicy::AllowModel
    );
}

#[test]
fn scope_update_clarify_is_resolved_when_active_task_exists() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(
        super::should_resolve_task_scope_update_clarify_with_active_task(
            "先只看登录模块",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        )
    );
}

#[test]
fn scope_update_clarify_reuses_active_task_without_keyword_detector() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Help me create a rollout plan".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(
        super::should_resolve_task_scope_update_clarify_with_active_task(
            "Keep it limited to the onboarding flow",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &IntentOutputContract::default(),
            None,
        )
    );
}

#[test]
fn task_replace_clarify_is_resolved_when_active_task_exists() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a long article about RustClaw".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_resolve_task_replace_clarify_with_active_task(
        "Actually, replace it with an X thread",
        Some(&snapshot),
        Some(TurnType::TaskReplace),
        Some(TargetTaskPolicy::ReplaceActive),
        false,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn task_replace_clarify_reuses_active_task_without_keyword_detector() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a launch memo about RustClaw".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_resolve_task_replace_clarify_with_active_task(
        "Make it a shorter internal memo instead",
        Some(&snapshot),
        Some(TurnType::TaskReplace),
        Some(TargetTaskPolicy::ReplaceActive),
        false,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn active_task_scope_update_is_routed_back_to_direct_answer() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_route_active_task_mutation_to_direct_answer(
        "先只看登录模块",
        Some(&snapshot),
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::PlannerExecute,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn structured_replacement_patch_repairs_active_task_correction_metadata() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Write a short setup checklist for RustClaw".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({
        "replacements": [
            {"old": "Python 3.10", "new": "Python 3.11"}
        ]
    });
    let mut turn_type = None;
    let mut target_task_policy = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "Use Python 3.11 instead of Python 3.10.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, Some("active_task_structured_patch_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn structured_patch_repair_does_not_override_explicit_filename_target() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a release note".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({
        "replacements": [
            {"old": "Python 3.10", "new": "Python 3.11"}
        ]
    });
    let mut turn_type = None;
    let mut target_task_policy = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "In README.md, replace Python 3.10 with Python 3.11.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, None);
    assert_eq!(target_task_policy, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
}

#[test]
fn scalar_patch_with_locator_hint_requires_active_binding_for_repair() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "RustClaw Release Notes - Your Quick Checklist".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let patch = serde_json::json!({"release_notes_python_version": "Python 3.11"});
    let mut turn_type = None;
    let mut target_task_policy = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "release notes".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_structured_patch_repair(
        "Use Python 3.11 instead of Python 3.10.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, None);
    assert_eq!(target_task_policy, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_hint, "release notes");

    let mut turn_type = Some(TurnType::TaskCorrect);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "release notes".to_string(),
        self_extension: Default::default(),
    };
    let reason = super::apply_active_task_structured_patch_repair(
        "Use Python 3.11 instead of Python 3.10.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        Some(&patch),
    );

    assert_eq!(reason, Some("active_task_structured_patch_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert!(!contract.requires_content_evidence);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn standalone_execution_target_misroute_is_repaired_to_active_scope_update() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "current workspace".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "先只看登录模块",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
    );

    assert_eq!(reason, Some("active_task_scope_refinement_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskScopeUpdate));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn scope_refinement_repair_detaches_from_structured_active_target() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("GET http://127.0.0.1:8787/v1/health".to_string()),
            last_primary_task_output: Some("Service status: reachable (HTTP 200).".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "GET http://127.0.0.1:8787/v1/health".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("http://127.0.0.1:8787/v1/health".to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "current workspace".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "A concept label without a concrete target.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
    );

    assert_eq!(
        reason,
        Some("active_task_scope_refinement_detached_from_structured_anchor")
    );
    assert_eq!(turn_type, None);
    assert_eq!(target_task_policy, None);
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn active_ordered_scalar_path_without_ref_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "find matching files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/rustclaw/fuzzy_top3".to_string()),
            ordered_entries: vec![
                "abcd_report.md".to_string(),
                "my_abcd.txt".to_string(),
                "x_abcd_log.txt".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_ordered_scalar_path_chat_repair(
        Some(&snapshot),
        None,
        "",
        false,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(
        reason,
        Some("active_ordered_scalar_path_chat_repair_without_structured_ref")
    );
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn active_observed_output_summary_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_output: Some(r#"{"phase":"loop_done","tool_calls":1}"#.to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read recent log tail".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/rustclaw/logs/act_plan.log".to_string()),
            source_task_id: "task-read".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExcerptKindJudgment,
        locator_hint: "/tmp/rustclaw/logs/act_plan.log".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_observed_output_chat_repair(
        "one sentence status judgment",
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        None,
        false,
        false,
        ScheduleKind::None,
        None,
        false,
        "",
        false,
        &mut decision,
        &mut finalize_style,
        &mut contract,
    );

    assert_eq!(reason, Some("active_observed_output_chat_repair"));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn scope_refinement_repair_does_not_override_explicit_locator() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我做一个测试计划".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "UI/src".to_string(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "先只看 UI/src",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "UI/src");
}

#[test]
fn scope_refinement_repair_preserves_standalone_observation_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("生成一个 JSON 文件".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "检查当前运行环境并只返回关键值",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskRequest));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn active_task_scope_update_en_remains_direct_answer_from_chat_wrapped_execution() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Help me create a test plan".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_route_active_task_mutation_to_direct_answer(
        "Only focus on the login module first",
        Some(&snapshot),
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::PlannerExecute,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn active_task_output_table_refinement_is_routed_back_to_direct_answer() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize the release checklist".to_string()),
            last_primary_task_output: Some(
                "1. Build\n2. Run tests\n3. Publish release notes".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_route_active_task_mutation_to_direct_answer(
        "把结果改成 markdown table 输出",
        Some(&snapshot),
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::PlannerExecute,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn active_task_correct_is_routed_back_to_direct_answer() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Write one deployment note that mentions Python 3.10".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_route_active_task_mutation_to_direct_answer(
        "Correction: not Python 3.10, use Python 3.11",
        Some(&snapshot),
        Some(TurnType::TaskCorrect),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::PlannerExecute,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn active_text_followup_clears_stale_scalar_answer_candidate() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "RustClaw v0.1.7 ships with clearer configuration controls.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let binding = super::AnswerCandidateBindingReport {
        candidate: "RC-CONT-EN-0428-B".to_string(),
        in_current_request: false,
        in_recent_assistant_replies: true,
        in_recent_turns_full: true,
        in_last_turn_full: true,
        in_recent_execution_context: false,
        in_memory_context: true,
    };
    let mut turn_type = Some(TurnType::PreferenceOrMemory);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = false;
    let mut wants_file_delivery = false;
    let mut answer_candidate = binding.candidate.clone();
    let mut contract = IntentOutputContract::default();

    let reason = super::apply_active_text_followup_route_repair(
        "Make it for non-technical users.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        false,
        true,
        &mut answer_candidate,
    );

    assert_eq!(reason, Some("active_text_followup_route_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskScopeUpdate));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(answer_candidate.is_empty());
    assert!(!contract.requires_content_evidence);
}

#[test]
fn active_task_invalid_turn_binding_context_uses_schema_tokens_not_user_phrases() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "RustClaw v0.1.7 ships with clearer configuration controls.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "Make it easier for non-technical readers.",
    );
    let raw = serde_json::json!({
        "turn_type": "response",
        "target_task_policy": "release_note_rewrite_non_technical"
    })
    .to_string();

    let context =
        super::active_task_invalid_turn_binding_context(&raw, Some(&snapshot), &surface, false)
            .unwrap();

    assert!(context.contains("active_task_invalid_turn_binding"));
    assert!(context.contains("turn_type_invalid: true"));
    assert!(context.contains("target_task_policy_invalid: true"));
}

#[test]
fn active_text_correction_clears_stale_workspace_evidence_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "RustClaw v0.1.7 supports Python 3.10 setup notes.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskCorrect);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "release notes".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_active_text_followup_route_repair(
        "Correction: mention Python 3.11, not Python 3.10.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        false,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, Some("active_text_followup_route_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn observed_context_summary_followup_clears_synthesis_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Read the first five lines of README.md".to_string()),
            last_primary_task_output: Some(
                "# Device Local Fixture\n\nStable local files for regression tests.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskAppend);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = true;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_active_text_followup_route_repair(
        "Summarize it in one sentence.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        false,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, Some("active_text_followup_route_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskAppend));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn observed_context_summary_followup_with_stale_evidence_contract_uses_existing_output() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Read the first five lines of README.md".to_string()),
            last_primary_task_output: Some(
                "# Device Local Fixture\n\nStable local files for regression tests.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "Read the first five lines of README.md".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/README.md".to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = None;
    let mut target_task_policy = None;
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = true;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_active_text_followup_route_repair(
        "Summarize it in one sentence.",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        false,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, Some("active_text_followup_route_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

#[test]
fn active_text_repair_preserves_current_request_runtime_locator_anchor() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a document outline".to_string()),
            last_primary_task_output: Some("Outline draft".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskCorrect);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_active_text_followup_route_repair(
        "List entries under the observed workspace directory target",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        true,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
}

#[test]
fn active_text_repair_preserves_current_request_directory_pair_anchor() {
    let workspace_root = make_temp_workspace_with_child("directory_pair_anchor", "seed");
    for idx in 0..2500 {
        std::fs::create_dir_all(workspace_root.join(format!("aaa_filler_{idx:04}")))
            .expect("create filler");
    }
    std::fs::create_dir_all(workspace_root.join("fixtures/tmp/bundle_src")).expect("left");
    std::fs::create_dir_all(workspace_root.join("fixtures/tmp/dynamic_guard_unpack_case"))
        .expect("right");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    state.skill_rt.locator_scan_max_files = 10;
    let request =
        "bundle_src 와 dynamic_guard_unpack_case 를 재귀 비교하고 차이가 있는지 짧게 답해.";
    assert!(super::resolved_directory_pair_from_current_request(&state, request).is_some());

    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a document outline".to_string()),
            last_primary_task_output: Some("Outline draft".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_active_text_followup_route_repair(
        request,
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        true,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);

    let _ = std::fs::remove_dir_all(workspace_root);
}

#[test]
fn active_ordered_observation_followup_keeps_executable_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("List the logs directory entries".to_string()),
            last_primary_task_output: Some("1. act_plan.log\n2. clawd.log".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "List the logs directory entries".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("logs".to_string()),
            ordered_entries: vec!["act_plan.log".to_string(), "clawd.log".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskScopeUpdate);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_active_text_followup_route_repair(
        "Show metadata for item 2",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        false,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn active_task_mutation_with_content_evidence_stays_executable() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize this repository".to_string()),
            last_primary_task_output: Some("It has a web UI and backend services.".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        ..IntentOutputContract::default()
    };
    assert!(!super::should_route_active_task_mutation_to_direct_answer(
        "Focus only on the UI part",
        Some(&snapshot),
        Some(TurnType::TaskScopeUpdate),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::PlannerExecute,
        &contract,
        None,
    ));
}

#[test]
fn active_text_followup_repair_preserves_real_workspace_summary_contract() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize this repository".to_string()),
            last_primary_task_output: Some("It has a web UI and backend services.".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskScopeUpdate);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_active_text_followup_route_repair(
        "Focus only on the UI part",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        false,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::WorkspaceProjectSummary
    );
}

#[test]
fn unresolved_deictic_observation_clarify_is_not_downgraded_to_direct_answer() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
            last_primary_task_output: Some("需要一个具体文件目标。".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };
    let state_patch = serde_json::json!({
        "deictic_reference": {"target": "unresolved_prior_object"}
    });
    assert!(
        !super::should_resolve_task_scope_update_clarify_with_active_task(
            "看看那个文件最后 5 行",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            FirstLayerDecision::Clarify,
            &contract,
            Some(&state_patch),
        )
    );
}

#[test]
fn structured_deictic_unresolved_target_blocks_non_chinese_pronoun_fallback_gap() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface("それの最後の2行を見せて");
    let contract = IntentOutputContract {
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let patch = serde_json::json!({
        "deictic_reference": {"target": "unresolved_prior_object"}
    });

    assert!(super::unresolved_deictic_observable_target_should_clarify(
        &surface,
        &contract,
        Some(&patch),
    ));
    assert!(!super::active_task_turn_can_reuse_semantic_patch(
        &surface,
        Some(&patch),
    ));
}

#[test]
fn structured_deictic_resolved_target_overrides_local_pronoun_fallback() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface("看看那个最后 5 行");
    let contract = IntentOutputContract {
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let patch = serde_json::json!({
        "deictic_reference": {"target": "current_action_result"}
    });

    assert!(!super::unresolved_deictic_observable_target_should_clarify(
        &surface,
        &contract,
        Some(&patch),
    ));
}

#[test]
fn scope_refinement_repair_keeps_unresolved_deictic_observation_clarify() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我整理一个方案".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "看看那个文件最后 5 行",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskRequest));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(needs_clarify);
    assert!(contract.requires_content_evidence);
}

#[test]
fn active_task_output_refinement_clarify_is_resolved() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize this repository".to_string()),
            last_primary_task_output: Some(
                "The UI is a web-based frontend for RustClaw.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_resolve_task_append_clarify_with_active_task(
        "Output a two-row markdown table",
        Some(&snapshot),
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn active_reuse_task_request_clarify_is_repaired_to_current_output_refinement() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize this repository".to_string()),
            last_primary_task_output: Some(
                "RustClaw has a browser UI for non-technical users.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::ReuseActive);
    let mut decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut wants_file_delivery = false;
    let mut answer_candidate = String::new();
    let mut contract = IntentOutputContract::default();

    let reason = super::apply_active_text_followup_route_repair(
        "Output a two-row markdown table",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut decision,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut wants_file_delivery,
        &mut contract,
        None,
        false,
        false,
        &mut answer_candidate,
    );

    assert_eq!(reason, Some("active_text_followup_route_repair"));
    assert_eq!(turn_type, Some(TurnType::TaskCorrect));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::ReuseActive));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert!(!needs_clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn active_task_append_clarify_without_output_is_resolved() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我写个方案".to_string()),
            last_primary_task_output: None,
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(super::should_resolve_task_append_clarify_with_active_task(
        "控制在 80 字内，只输出正文",
        Some(&snapshot),
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn active_task_append_clarify_keeps_file_locator_guard() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
            last_primary_task_output: None,
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!super::should_resolve_task_append_clarify_with_active_task(
        "README.md",
        Some(&snapshot),
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
        false,
        FirstLayerDecision::Clarify,
        &IntentOutputContract::default(),
        None,
    ));
}

#[test]
fn bare_path_correction_can_fill_active_observable_task() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "读一下 configs/config.toml 里的名字字段，只输出值".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskCorrect),
        Some(TargetTaskPolicy::ReuseActive),
        FirstLayerDecision::PlannerExecute,
        &contract,
    ));
}

#[test]
fn bare_path_clarify_with_observable_scalar_contract_can_fill_active_task() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Extract the name field from the package file and output only the value"
                    .to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        None,
        None,
        FirstLayerDecision::Clarify,
        &contract,
    ));
}

#[test]
fn bare_path_active_clarify_state_can_fill_standalone_task_request() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Provide the file path".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(OutputSemanticKind::StructuredKeys.as_str().to_string()),
            source_request: "Find the name field in the package file and output only the value"
                .to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::Standalone),
        FirstLayerDecision::Clarify,
        &contract,
    ));
}

#[test]
fn bare_filename_task_request_can_replace_active_existence_check() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("看看那个重启脚本在不在".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "restart_clawd_latest.sh".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::ReplaceActive),
        FirstLayerDecision::PlannerExecute,
        &contract,
    ));
}

#[test]
fn bare_path_with_executable_contract_can_fill_active_log_tail() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我看一下那个日志最近 20 行".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "logs/clawd.log".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        None,
        None,
        FirstLayerDecision::PlannerExecute,
        &contract,
    ));
}

#[test]
fn bare_filename_can_replace_active_delivery_target() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("send the selected file".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "send the selected file".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("/tmp/old.md".to_string()),
            output_shape: Some(OutputResponseShape::FileToken.as_str().to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "README.md".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        None,
        FirstLayerDecision::PlannerExecute,
        &contract,
    ));
}

#[test]
fn bare_path_without_observable_contract_still_needs_action_clarify() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            FirstLayerDecision::PlannerExecute,
            &IntentOutputContract::default(),
        )
    );
}

#[test]
fn workspace_scope_listing_shape_is_not_treated_as_fileish_cue() {
    let surface =
        crate::intent::surface_signals::analyze_prompt_surface("看看当前目录有哪些顶层文件夹");
    assert!(!super::prompt_has_concrete_fileish_cue(&surface));
}

#[test]
fn simple_command_shape_is_not_treated_as_fileish_cue() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface("执行 pwd");
    assert!(!super::prompt_has_concrete_fileish_cue(&surface));
}

#[test]
fn locator_target_pair_still_counts_as_fileish_cue() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "比较 README.md 和 AGENTS.md 哪个更大",
    );
    assert!(super::prompt_has_concrete_fileish_cue(&surface));
}

/// §3.5c-小切口：intent_normalizer schema 与 Rust parser 漂移检查。
///
/// 校验内容：
/// 1. `prompts/schemas/intent_normalizer.schema.json` 是合法 JSON 且为 object schema；
/// 2. `IntentNormalizerOut` 里所有 `#[serde(default)]` 字段都在 schema `properties` 里；
/// 3. 每个 enum-bearing 字段的 schema 枚举值，喂给对应 `parse_*` 函数都能落到非默认 variant
///    （空字符串和 `"none"`/`"unknown"` 这种"显式无"语义值排除）。
///
/// 任何一项不满足都说明 prompt / schema / parser 三者已漂移，应在本测试里同步更新。
#[test]
fn intent_normalizer_schema_drift() {
    const SCHEMA_RAW: &str = include_str!("../../../prompts/schemas/intent_normalizer.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("intent_normalizer.schema.json must be valid JSON");
    assert_eq!(
        schema.get("type").and_then(|v| v.as_str()),
        Some("object"),
        "schema root must be object"
    );

    // §3.5c-小切口 步骤 2：每个 IntentNormalizerOut 字段必须在 properties 里登记。
    const STRUCT_FIELDS: &[&str] = &[
        "resolved_user_intent",
        "answer_candidate",
        "resume_behavior",
        "schedule_kind",
        "wants_file_delivery",
        "should_refresh_long_term_memory",
        "agent_display_name_hint",
        "needs_clarify",
        "clarify_question",
        "reason",
        "confidence",
        "decision",
        "schedule_intent",
        "output_contract",
        "execution_recipe",
        "turn_type",
        "target_task_policy",
        "should_interrupt_active_run",
        "state_patch",
        "attachment_processing_required",
    ];
    let properties = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .expect("schema must have `properties` object");
    for field in STRUCT_FIELDS {
        assert!(
                properties.contains_key(*field),
                "schema missing parser field `{}` under properties — sync prompts/schemas/intent_normalizer.schema.json with IntentNormalizerOut",
                field
            );
    }

    // §3.5c-小切口 步骤 3：枚举值 → parse_* 函数必须落到非默认 variant
    // （除非是显式的「无 / 未知」语义占位）。
    fn enum_strings<'a>(schema: &'a serde_json::Value, path: &[&str]) -> Vec<String> {
        let mut node = schema;
        for p in path {
            node = node
                .get(*p)
                .unwrap_or_else(|| panic!("schema path `{}` not found", path.join(".")));
        }
        node.get("enum")
            .and_then(|v| v.as_array())
            .unwrap_or_else(|| panic!("schema path `{}.enum` not found", path.join(".")))
            .iter()
            .filter_map(|v| v.as_str().map(str::to_string))
            .collect()
    }

    // resume_behavior：none / "" 是「无」语义，跳过。
    for token in enum_strings(&schema, &["properties", "resume_behavior"]) {
        if token.is_empty() || token == "none" {
            continue;
        }
        let parsed = super::parse_resume_behavior(&token);
        assert_ne!(
            parsed,
            super::ResumeBehavior::None,
            "resume_behavior token `{}` not recognized by parse_resume_behavior",
            token
        );
    }

    for token in enum_strings(&schema, &["properties", "schedule_kind"]) {
        if token.is_empty() || token == "none" {
            continue;
        }
        let parsed = super::parse_schedule_kind(&token);
        assert_ne!(
            parsed,
            super::ScheduleKind::None,
            "schedule_kind token `{}` not recognized by parse_schedule_kind",
            token
        );
    }

    for token in enum_strings(&schema, &["properties", "turn_type"]) {
        if token.is_empty() {
            continue;
        }
        let parsed = super::parse_turn_type(&token);
        assert!(
            parsed.is_some(),
            "turn_type token `{}` not recognized by parse_turn_type",
            token
        );
    }

    for token in enum_strings(&schema, &["properties", "target_task_policy"]) {
        if token.is_empty() {
            continue;
        }
        let parsed = super::parse_target_task_policy(&token);
        assert!(
            parsed.is_some(),
            "target_task_policy token `{}` not recognized by parse_target_task_policy",
            token
        );
    }

    // decision: the only first-layer semantic gate.
    for token in enum_strings(&schema, &["properties", "decision"]) {
        if token.is_empty() {
            continue;
        }
        assert!(
            super::parse_first_layer_decision_text(&token).is_some(),
            "decision token `{}` not recognized by parse_first_layer_decision_text",
            token
        );
    }

    for token in enum_strings(
        &schema,
        &[
            "properties",
            "output_contract",
            "properties",
            "response_shape",
        ],
    ) {
        if token.is_empty() || token == "free" {
            continue;
        }
        assert_ne!(
            super::parse_output_response_shape(&token),
            OutputResponseShape::Free,
            "response_shape `{}` not recognized",
            token
        );
    }
    for token in enum_strings(
        &schema,
        &[
            "properties",
            "output_contract",
            "properties",
            "locator_kind",
        ],
    ) {
        if token.is_empty() || token == "none" {
            continue;
        }
        assert_ne!(
            super::parse_output_locator_kind(&token),
            OutputLocatorKind::None,
            "locator_kind `{}` not recognized",
            token
        );
    }
    for token in enum_strings(
        &schema,
        &[
            "properties",
            "output_contract",
            "properties",
            "delivery_intent",
        ],
    ) {
        if token.is_empty() || token == "none" {
            continue;
        }
        assert_ne!(
            super::parse_output_delivery_intent(&token),
            OutputDeliveryIntent::None,
            "delivery_intent `{}` not recognized",
            token
        );
    }
    for token in enum_strings(
        &schema,
        &[
            "properties",
            "output_contract",
            "properties",
            "semantic_kind",
        ],
    ) {
        if token.is_empty() || token == "none" {
            continue;
        }
        if token == "scalar" {
            assert_eq!(
                super::parse_output_semantic_kind(&token),
                OutputSemanticKind::None,
                "semantic_kind `scalar` is a legacy LLM alias and should normalize to none"
            );
            continue;
        }
        assert_ne!(
            super::parse_output_semantic_kind(&token),
            OutputSemanticKind::None,
            "semantic_kind `{}` not recognized",
            token
        );
    }
    let schema_semantic_kinds = enum_strings(
        &schema,
        &[
            "properties",
            "output_contract",
            "properties",
            "semantic_kind",
        ],
    )
    .into_iter()
    .collect::<std::collections::BTreeSet<_>>();
    for kind in OutputSemanticKind::ALL {
        assert!(
            schema_semantic_kinds.contains(kind.as_str()),
            "intent_normalizer schema missing canonical semantic_kind `{}`",
            kind.as_str()
        );
    }
    for token in enum_strings(
        &schema,
        &[
            "properties",
            "output_contract",
            "properties",
            "self_extension",
            "properties",
            "mode",
        ],
    ) {
        if token.is_empty() || token == "none" {
            continue;
        }
        assert_ne!(
            super::parse_self_extension_mode(&token),
            crate::SelfExtensionMode::None,
            "self_extension.mode `{}` not recognized",
            token
        );
    }
    for token in enum_strings(
        &schema,
        &[
            "properties",
            "output_contract",
            "properties",
            "self_extension",
            "properties",
            "trigger",
        ],
    ) {
        if token.is_empty() || token == "none" {
            continue;
        }
        assert_ne!(
            super::parse_self_extension_trigger(&token),
            crate::SelfExtensionTrigger::None,
            "self_extension.trigger `{}` not recognized",
            token
        );
    }

    for token in enum_strings(
        &schema,
        &["properties", "execution_recipe", "properties", "kind"],
    ) {
        if token.is_empty() || token == "none" {
            continue;
        }
        assert_ne!(
            crate::execution_recipe::parse_execution_recipe_kind_text(&token),
            ExecutionRecipeKind::None,
            "execution_recipe.kind `{}` not recognized",
            token
        );
    }
    for token in enum_strings(
        &schema,
        &["properties", "execution_recipe", "properties", "profile"],
    ) {
        if token.is_empty() || token == "none" {
            continue;
        }
        assert_ne!(
            crate::execution_recipe::parse_execution_recipe_profile_text(&token),
            ExecutionRecipeProfile::None,
            "execution_recipe.profile `{}` not recognized",
            token
        );
    }
    for token in enum_strings(
        &schema,
        &[
            "properties",
            "execution_recipe",
            "properties",
            "target_scope",
        ],
    ) {
        if token.is_empty() || token == "none" || token == "unknown" {
            continue;
        }
        assert_ne!(
            crate::execution_recipe::parse_execution_recipe_target_scope_text(&token),
            ExecutionRecipeTargetScope::Unknown,
            "execution_recipe.target_scope `{}` not recognized",
            token
        );
    }
}

#[test]
fn parse_output_semantic_kind_prefers_last_recognized_token_in_multi_value_output() {
    assert_eq!(
        super::parse_output_semantic_kind("sqlite_table_listing|sqlite_database_kind_judgment"),
        OutputSemanticKind::SqliteDatabaseKindJudgment
    );
}
