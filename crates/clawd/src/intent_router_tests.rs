use super::{
    apply_current_turn_structural_contract_repair, parse_execution_recipe_hint,
    parse_runtime_async_job_start_plan_hint, structural_alias_binding_fallback_decision,
    structured_execution_signal_for_effective_route, IntentExecutionRecipeOut,
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, RouteTraceDecision, ScheduleKind, TargetTaskPolicy, TurnType,
};
use crate::execution_recipe::{
    ExecutionRecipeKind, ExecutionRecipeProfile, ExecutionRecipeTargetScope,
};
use serde_json::Value;

#[test]
fn parse_execution_recipe_hint_accepts_explicit_ops_service_contract() {
    let spec = parse_execution_recipe_hint(Some(IntentExecutionRecipeOut {
        kind: "ops_closed_loop".to_string(),
        profile: "ops_service".to_string(),
        target_scope: "system".to_string(),
        ..IntentExecutionRecipeOut::default()
    }))
    .expect("execution recipe spec");
    assert_eq!(spec.profile, ExecutionRecipeProfile::OpsService);
    assert_eq!(spec.target_scope, ExecutionRecipeTargetScope::System);
    assert!(spec.inspect_first);
    assert!(spec.validation_required);
}

#[test]
fn runtime_async_job_start_state_patch_survives_schema_and_becomes_plan_hint() {
    let raw = r#"{
      "resolved_user_intent":"start async local command",
      "resume_behavior":"none",
      "schedule_kind":"none",
      "schedule_intent":null,
      "wants_file_delivery":false,
      "should_refresh_long_term_memory":false,
      "agent_display_name_hint":"",
      "needs_clarify":false,
      "clarify_question":"",
      "reason":"runtime_async_job_start_plan_hint",
      "confidence":0.9,
      "decision":"planner_execute",
      "output_contract":{
        "response_shape":"strict",
        "exact_sentence_count":null,
        "requires_content_evidence":true,
        "delivery_required":false,
        "locator_kind":"none",
        "delivery_intent":"none",
        "locator_hint":"",
        "scalar_count_filter":null,
        "list_selector":null,
        "self_extension":{"mode":"none","trigger":"none","execute_now":false}
      },
      "execution_recipe":{"kind":"none","profile":"none","target_scope":"none"},
      "turn_type":"task_request",
      "target_task_policy":"standalone",
      "should_interrupt_active_run":false,
      "state_patch":{
        "runtime_async_job_start":{
          "command":"sleep 2 && echo RUSTCLAW_ASYNC_LIFECYCLE",
          "execution_mode":"async_start",
          "async_adapter_kind":"local_process_poll"
        }
      },
      "attachment_processing_required":false
    }"#;

    let out = crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("runtime async job state patch should validate");
    let hint = parse_runtime_async_job_start_plan_hint(out.value.state_patch.as_ref())
        .expect("runtime async job state patch should become plan hint");

    assert_eq!(
        hint.command.as_deref(),
        Some("sleep 2 && echo RUSTCLAW_ASYNC_LIFECYCLE")
    );
    assert_eq!(hint.execution_mode.as_deref(), Some("async_start"));
    assert_eq!(
        hint.async_adapter_kind.as_deref(),
        Some("local_process_poll")
    );
}

#[test]
fn state_patch_slice_tokens_are_promoted_to_resolved_intent_machine_tokens() {
    let resolved = super::append_state_patch_slice_tokens_to_resolved_intent(
        "Summarize the bounded excerpt.".to_string(),
        Some(&serde_json::json!({
            "slice_mode": "tail",
            "slice_n": 5,
            "slice_start": 12,
            "slice_end": "16"
        })),
    );

    assert!(resolved.contains("slice_mode=tail"));
    assert!(resolved.contains("slice_n=5"));
    assert!(resolved.contains("slice_start=12"));
    assert!(resolved.contains("slice_end=16"));
}

#[test]
fn structural_alias_binding_fallback_uses_state_patch_without_llm() {
    let (decision, turn_analysis) = structural_alias_binding_fallback_decision(
        "先记一下，后面我说“那个文件”就是 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md",
    )
    .expect("quoted alias plus single locator should be structural");

    assert!(!decision.needs_clarify);
    assert_eq!(turn_analysis.turn_type, Some(TurnType::PreferenceOrMemory));
    assert_eq!(
        turn_analysis.target_task_policy,
        Some(TargetTaskPolicy::Standalone)
    );
    let patch = turn_analysis.state_patch.expect("state patch");
    assert_eq!(
        patch
            .pointer("/alias_bindings/0/alias")
            .and_then(Value::as_str),
        Some("那个文件")
    );
    assert_eq!(
        patch
            .pointer("/alias_bindings/0/target")
            .and_then(Value::as_str),
        Some("/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md")
    );
}

#[test]
fn structural_alias_binding_fallback_ignores_same_turn_work_after_locator() {
    let fallback = structural_alias_binding_fallback_decision(
        "先记一下，“那个配置文件”就是 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/configs/app_config.toml；然后把那个配置文件发给我；最后再用一句话说它主要是干嘛的",
    );

    assert!(fallback.is_none());
}

#[test]
fn structural_alias_binding_fallback_allows_terminal_punctuation() {
    let (decision, turn_analysis) = structural_alias_binding_fallback_decision(
        "先记一下，后面我说“那个文件”就是 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md。",
    )
    .expect("terminal punctuation keeps the prompt alias-only");

    assert!(!decision.needs_clarify);
    assert_eq!(turn_analysis.turn_type, Some(TurnType::PreferenceOrMemory));
}

#[test]
fn structural_alias_binding_fallback_supports_multiple_aliases_without_llm() {
    let (_decision, turn_analysis) = structural_alias_binding_fallback_decision(
        "先记一下，“那个目录”是 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs，“那个日志”是 /home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/app.log",
    )
    .expect("multiple quoted aliases plus explicit locators should be structural");

    assert_eq!(turn_analysis.turn_type, Some(TurnType::PreferenceOrMemory));
    let patch = turn_analysis.state_patch.expect("state patch");
    let bindings = patch
        .get("alias_bindings")
        .and_then(Value::as_array)
        .expect("alias bindings array");
    assert_eq!(bindings.len(), 2);
    assert_eq!(
        bindings[0].get("alias").and_then(Value::as_str),
        Some("那个目录")
    );
    assert_eq!(
        bindings[0].get("target").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs")
    );
    assert_eq!(
        bindings[1].get("alias").and_then(Value::as_str),
        Some("那个日志")
    );
    assert_eq!(
        bindings[1].get("target").and_then(Value::as_str),
        Some("/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/logs/app.log")
    );
}

#[test]
fn route_result_drops_backend_model_display_name_hint() {
    use std::sync::Arc;

    fn identity_decision(
        agent_display_name_hint: &str,
        should_refresh_long_term_memory: bool,
    ) -> super::RouteDecision {
        super::RouteDecision {
            resolved_user_intent: "User asks for assistant identity.".to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            reason: "identity direct response".to_string(),
            confidence: Some(0.95),
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory,
            agent_display_name_hint: agent_display_name_hint.to_string(),
            output_contract: IntentOutputContract::default(),
        }
    }

    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.llm_providers = vec![Arc::new(crate::LlmProviderRuntime {
        config: claw_core::config::LlmProviderConfig {
            name: "vendor-mimo".to_string(),
            provider_type: "openai_compat".to_string(),
            base_url: "http://fixture.invalid".to_string(),
            api_key: "fixture".to_string(),
            model: "mimo-v2.5-pro".to_string(),
            context_window_tokens: None,
            priority: 1,
            timeout_seconds: 5,
            max_concurrency: 1,
            params: claw_core::config::LlmProviderParams::default(),
        },
        client: reqwest::Client::new(),
        semaphore: Arc::new(tokio::sync::Semaphore::new(1)),
        breaker: Arc::new(crate::providers::CircuitBreaker::new()),
    })];
    let task = crate::ClaimedTask {
        task_id: "task-display-name-backend-sanitize".to_string(),
        user_id: 91,
        chat_id: 202,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"who are you"}).to_string(),
    };

    let out = super::normalizer_output_from_fallback(
        "who are you",
        "test_fallback",
        identity_decision("MiMo-v2.5-pro", false),
        None,
    );
    let route = super::route_result_from_normalizer(&state, &task, &out);

    assert_eq!(route.agent_display_name_hint, "");
    assert!(route
        .route_reason
        .contains("agent_display_name_hint_backend_metadata_removed"));

    let out = super::normalizer_output_from_fallback(
        "remember this display name",
        "test_fallback",
        identity_decision("MiMo-v2.5-pro", true),
        None,
    );
    let route = super::route_result_from_normalizer(&state, &task, &out);
    assert_eq!(route.agent_display_name_hint, "MiMo-v2.5-pro");

    let out = super::normalizer_output_from_fallback(
        "who are you",
        "test_fallback",
        identity_decision("Mimosa", false),
        None,
    );
    let route = super::route_result_from_normalizer(&state, &task, &out);
    assert_eq!(route.agent_display_name_hint, "Mimosa");
}

#[test]
fn route_result_uses_machine_execution_signal_not_legacy_normalizer_hint() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-machine-route-execute".to_string(),
        user_id: 91,
        chat_id: 202,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"inspect workspace"}).to_string(),
    };
    let decision = super::RouteDecision {
        resolved_user_intent: "inspect current workspace".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "structured evidence required".to_string(),
        confidence: Some(0.95),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            ..IntentOutputContract::default()
        },
    };
    let mut out = super::normalizer_output_from_fallback(
        "inspect workspace",
        "test_fallback",
        decision,
        None,
    );
    out.route_trace_record.route_trace_decision = RouteTraceDecision::Respond;

    let route = super::route_result_from_normalizer(&state, &task, &out);

    assert!(route.ask_mode.is_execute_gate());
    assert!(route.route_reason.contains("test_fallback"));
    assert_eq!(
        route.output_contract.semantic_kind,
        OutputSemanticKind::None,
        "raw normalizer semantic_kind should not remain route authority"
    );
    assert_eq!(
        route.effective_output_contract_semantic_kind(),
        OutputSemanticKind::WorkspaceProjectSummary
    );
    assert!(route
        .route_reason
        .contains("contract:workspace_project_summary"));
}

#[test]
fn route_result_does_not_bind_task_control_from_async_contract_field_names() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-async-contract-fields".to_string(),
        user_id: 91,
        chat_id: 202,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"dry-run async contract"}).to_string(),
    };
    let decision = super::RouteDecision {
        resolved_user_intent:
            "runtime_async_start_contract adapter_kind=local_process_poll checkpoint_id poll_ref cancel_ref"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason:
            "required boundary fields adapter_kind checkpoint_id poll_ref cancel_ref".to_string(),
        confidence: Some(0.95),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::None,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            locator_hint: String::new(),
            ..IntentOutputContract::default()
        },
    };
    let out = super::normalizer_output_from_fallback(
        "dry-run async contract",
        "test_fallback",
        decision,
        None,
    );

    let route = super::route_result_from_normalizer(&state, &task, &out);

    assert!(!route
        .route_reason
        .contains("task_lifecycle_machine_fields_bound_to_task_control"));
    assert!(!route
        .route_reason
        .contains("capability_ref=task_control.list"));
    assert_ne!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("task_lifecycle.*")
    );
}

#[test]
fn route_result_binds_task_control_from_machine_capability_ref() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-control-capability-ref".to_string(),
        user_id: 91,
        chat_id: 202,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"inspect task lifecycle"}).to_string(),
    };
    let decision = super::RouteDecision {
        resolved_user_intent: "capability_ref=task_control.list".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "capability_ref=task_control.list".to_string(),
        confidence: Some(0.95),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    };
    let out = super::normalizer_output_from_fallback(
        "inspect task lifecycle",
        "test_fallback",
        decision,
        None,
    );

    let route = super::route_result_from_normalizer(&state, &task, &out);

    assert!(route
        .route_reason
        .contains("task_lifecycle_machine_fields_bound_to_task_control"));
    assert_eq!(
        route
            .output_contract
            .self_extension
            .structured_field_selector
            .as_deref(),
        Some("task_lifecycle.*")
    );
}

#[test]
fn route_result_ignores_legacy_planner_hint_without_machine_execution_signal() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-machine-route-chat".to_string(),
        user_id: 91,
        chat_id: 202,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"plain discussion"}).to_string(),
    };
    let decision = super::RouteDecision {
        resolved_user_intent: "plain discussion".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: "no execution signal".to_string(),
        confidence: Some(0.95),
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract::default(),
    };
    let mut out =
        super::normalizer_output_from_fallback("plain discussion", "test_fallback", decision, None);
    out.route_trace_record.route_trace_decision = RouteTraceDecision::Act;

    let route = super::route_result_from_normalizer(&state, &task, &out);

    assert!(route.ask_mode.is_execute_gate());
    assert!(!route.ask_mode.is_chat_gate());
}

#[test]
fn clarify_inline_payload_wrapper_keeps_locatorless_transform_contract() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"Continue the previous resolved request by applying the same operation to the provided target or content.
Previous user request: sort the JSON array by score and render a markdown table
Provided target or content: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        request,
        &surface,
        &state.skill_rt.workspace_root,
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::Standalone),
    );

    assert_eq!(repair, Some("inline_structured_payload_context_execute"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn current_workspace_generic_summary_contract_repair_uses_workspace_project_summary() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = "Produce the requested workspace-grounded summary.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        request,
        &surface,
        &state.skill_rt.workspace_root,
        None,
        None,
    );

    assert_eq!(
        repair,
        Some("current_workspace_summary_semantic_contract_repair")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::WorkspaceProjectSummary
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(
        contract.locator_hint,
        state.skill_rt.workspace_root.display().to_string()
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
}

#[test]
fn current_workspace_extension_inventory_repair_uses_file_paths() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = "find toml files in this repo and briefly mention a few representative ones";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        request,
        &surface,
        &state.skill_rt.workspace_root,
        None,
        None,
    );

    assert_eq!(
        repair,
        Some("current_workspace_extension_file_paths_contract_repair")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FilePaths);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(
        contract.locator_hint,
        state.skill_rt.workspace_root.display().to_string()
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
}

#[test]
fn current_workspace_extension_inventory_repair_ignores_explicit_file_targets() {
    let request = "summarize Cargo.toml";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    assert!(
        !super::current_turn_extension_inventory_file_paths_repair_applies(
            &contract, request, &surface
        )
    );
}

#[test]
fn semantic_kind_alone_does_not_require_structured_execution_signal() {
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::FileBasename,
        ..Default::default()
    };

    assert!(!structured_execution_signal_for_effective_route(
        &contract,
        false,
        ScheduleKind::None,
        None,
    ));
    contract.semantic_kind = OutputSemanticKind::None;
    contract.requires_content_evidence = true;
    assert!(structured_execution_signal_for_effective_route(
        &contract,
        false,
        ScheduleKind::None,
        None,
    ));
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

    // §3.5c-小切口 步骤 2：每个 live IntentNormalizerOut schema 字段必须在 properties 里登记。
    // Removed legacy fields such as answer_candidate must stay out of both the live schema and parser model.
    const STRUCT_FIELDS: &[&str] = &[
        "resolved_user_intent",
        "resume_behavior",
        "schedule_kind",
        "wants_file_delivery",
        "should_refresh_long_term_memory",
        "agent_display_name_hint",
        "needs_clarify",
        "clarify_question",
        "reason",
        "confidence",
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
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .expect("schema must have `required` array");
    assert!(
        !required.iter().any(|value| value.as_str() == Some("decision")),
        "legacy normalizer `decision` must stay optional; ordinary semantics belong to the agent loop"
    );
    for field in STRUCT_FIELDS {
        assert!(
            properties.contains_key(*field),
            "schema missing parser field `{}` under properties — sync prompts/schemas/intent_normalizer.schema.json with IntentNormalizerOut",
            field
        );
    }
    assert!(
        !properties.contains_key("decision"),
        "intent_normalizer schema must not expose legacy decision"
    );
    assert!(
        !properties.contains_key("answer_candidate"),
        "intent_normalizer schema must not expose legacy answer_candidate"
    );

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
    assert_eq!(
        super::parse_turn_type("runtime_status_query"),
        Some(TurnType::StatusQuery)
    );

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
    let output_contract_properties = schema
        .pointer("/properties/output_contract/properties")
        .and_then(|value| value.as_object())
        .expect("schema must declare output_contract properties");
    assert!(
        output_contract_properties.contains_key("contract_marker"),
        "intent_normalizer schema must expose contract_marker"
    );
    assert!(
        !output_contract_properties.contains_key("semantic_kind"),
        "intent_normalizer schema must not expose legacy semantic_kind"
    );
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
fn intent_normalizer_schema_accepts_missing_legacy_decision() {
    let raw = r#"{
      "resolved_user_intent":"boundary-only request",
      "needs_clarify":false,
      "reason":"boundary_only",
      "confidence":0.9,
      "output_contract":{
        "response_shape":"free",
        "requires_content_evidence":false,
        "delivery_required":false,
        "locator_kind":"none",
        "delivery_intent":"none",
        "contract_marker":"none",
        "locator_hint":""
      }
    }"#;
    let validated = crate::prompt_utils::validate_against_schema::<super::IntentNormalizerOut>(
        raw,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    )
    .expect("normalizer schema should allow omitted legacy decision")
    .value;
    assert_eq!(validated.resolved_user_intent, "boundary-only request");
}

#[test]
fn parse_output_semantic_kind_prefers_last_recognized_token_in_multi_value_output() {
    assert_eq!(
        super::parse_output_semantic_kind("sqlite_table_listing|sqlite_database_kind_judgment"),
        OutputSemanticKind::SqliteDatabaseKindJudgment
    );
}
