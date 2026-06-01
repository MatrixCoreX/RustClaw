use super::{
    active_ordered_entries_count_direct_answer_candidate, apply_direct_answer_gate_outcome,
    ask_reply_with_chat_process, chat_prompt_context_with_route_resolution,
    chat_request_for_prompt, chat_user_request, contract_test_hint_should_enter_planner_loop,
    current_request_mentions_resolvable_gate_locator, direct_answer_chat_user_request,
    direct_answer_gate_can_skip_for_active_observed_output_chat_repair,
    direct_answer_gate_can_skip_for_active_task_text_mutation,
    direct_answer_gate_can_skip_for_pure_chat_draft,
    direct_answer_gate_can_skip_for_recent_count_context,
    direct_answer_gate_can_skip_for_self_contained_payload,
    direct_answer_gate_candidate_needs_unbound_context_clarify,
    direct_answer_gate_planner_needs_unbound_locator_clarify,
    direct_answer_gate_promotion_depends_only_on_background_context,
    direct_answer_gate_promotion_needs_unbound_deictic_clarify,
    direct_answer_gate_recent_execution_context, direct_answer_gate_route_context,
    direct_chat_answer_needs_repair, direct_chat_answer_repair_prompt,
    ensure_active_task_required_visible_literals, forbidden_visible_literals_from_state_patch,
    locator_hint_mentions_current_request, normalizer_chat_direct_answer_candidate,
    normalizer_runtime_fact_direct_answer_candidate, output_contract_from_direct_answer_gate,
    preferred_route_clarify_question, promote_inline_json_transform_context_to_planner,
    recent_count_comparison_direct_answer, replacement_pairs_from_state_patch,
    required_visible_literals_from_state_patch,
    resolved_intent_declares_structured_scalar_extraction,
    route_contract_requests_filename_only_output, route_structured_clarify_context,
    runtime_approval_wait_status_direct_answer_candidate,
    runtime_scalar_path_direct_answer_candidate, session_alias_target_direct_answer_candidate,
    state_patch_alias_bindings_ack, structural_alias_binding_ack, task_payload_text,
    token_looks_like_pathlike_locator, DirectAnswerGateContractOut, DirectAnswerGateOut,
    DirectAnswerGateReferenceResolutionOut, DirectAnswerGateSelfExtensionOut,
    DirectAnswerPreflight,
};

fn schema_enum_strings(schema: &serde_json::Value, path: &[&str]) -> Vec<String> {
    let mut node = schema;
    for part in path {
        node = node
            .get(*part)
            .unwrap_or_else(|| panic!("schema path `{}` not found", path.join(".")));
    }
    node.get("enum")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| panic!("schema path `{}.enum` not found", path.join(".")))
        .iter()
        .filter_map(|value| value.as_str().map(str::to_string))
        .collect()
}

#[test]
fn direct_answer_gate_schema_drift() {
    const SCHEMA_RAW: &str =
        include_str!("../../../prompts/schemas/direct_answer_gate.schema.json");
    let schema: serde_json::Value =
        serde_json::from_str(SCHEMA_RAW).expect("direct_answer_gate schema must be valid JSON");
    assert_eq!(
        schema.get("type").and_then(serde_json::Value::as_str),
        Some("object")
    );
    assert_eq!(
        schema.get("additionalProperties"),
        Some(&serde_json::json!(false)),
        "direct_answer_gate root must reject unknown fields after canonicalization"
    );

    let properties = schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        .expect("direct_answer_gate schema must have object properties");
    for field in [
        "decision",
        "reason",
        "confidence",
        "clarify_question",
        "resolved_user_intent",
        "reference_resolution",
        "output_contract",
    ] {
        assert!(
            properties.contains_key(field),
            "schema missing DirectAnswerGateOut field `{field}`"
        );
    }

    let contract_properties = schema
        .pointer("/properties/output_contract/properties")
        .and_then(serde_json::Value::as_object)
        .expect("output_contract must have object properties");
    for field in [
        "response_shape",
        "exact_sentence_count",
        "requires_content_evidence",
        "delivery_required",
        "locator_kind",
        "delivery_intent",
        "semantic_kind",
        "locator_hint",
        "self_extension",
    ] {
        assert!(
            contract_properties.contains_key(field),
            "schema missing DirectAnswerGateContractOut field `{field}`"
        );
    }

    let semantic_schema = schema_enum_strings(
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
    let semantic_rust = crate::OutputSemanticKind::ALL
        .iter()
        .map(|kind| kind.as_str().to_string())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        semantic_schema, semantic_rust,
        "direct_answer_gate semantic_kind enum must stay aligned with OutputSemanticKind::ALL"
    );

    let raw = serde_json::json!({
        "decision": "planner_execute",
        "reason": "needs fresh evidence",
        "confidence": 0.9,
        "clarify_question": "",
        "resolved_user_intent": "List files",
        "reference_resolution": {"target": "none"},
        "output_contract": {
            "response_shape": "strict",
            "exact_sentence_count": null,
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "path",
            "delivery_intent": "none",
            "semantic_kind": "file_names",
            "locator_hint": "docs",
            "self_extension": {
                "mode": "none",
                "trigger": "none",
                "execute_now": false
            }
        }
    })
    .to_string();
    crate::prompt_utils::validate_against_schema::<DirectAnswerGateOut>(
        &raw,
        crate::prompt_utils::PromptSchemaId::DirectAnswerGate,
    )
    .expect("schema-conformant direct_answer_gate payload must deserialize");
}

#[test]
fn direct_chat_answer_rejects_unclosed_code_fence() {
    assert!(direct_chat_answer_needs_repair("```bash"));
    assert!(direct_chat_answer_needs_repair("```text\nunfinished"));
    assert!(!direct_chat_answer_needs_repair(
        "我会只查看压缩包目录，不会解压。"
    ));
    assert!(!direct_chat_answer_needs_repair("```text\nok\n```"));
}

#[test]
fn direct_chat_answer_repair_prompt_preserves_request_context() {
    let prompt = direct_chat_answer_repair_prompt("REQ: say hi", "```bash");
    assert!(prompt.contains("REQ: say hi"));
    assert!(prompt.contains("Previous Draft Rejected"));
    assert!(prompt.contains("complete final answer"));
}

#[test]
fn contract_test_hint_docker_logs_forces_planner_before_direct_chat() {
    let mut route = chat_route_for_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DockerLogs;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let request = concat!(
        "查看最近一个 Docker 容器日志片段，如果没有容器就说明无法获取日志的原因。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=docker_logs\n",
        "required_evidence_json=[\"candidates\"]\n",
        "allowed_actions_json=[\"docker_basic\",\"run_cmd\"]\n",
        "none_passthrough=false\n",
        "[/CONTRACT_TEST_HINT]"
    );

    assert!(contract_test_hint_should_enter_planner_loop(
        request,
        Some(&ctx)
    ));
}

#[test]
fn contract_test_hint_none_passthrough_does_not_force_planner() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        ..Default::default()
    };
    let request = concat!(
        "不用执行任何操作，直接回答。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=none\n",
        "required_evidence_json=[]\n",
        "allowed_actions_json=[]\n",
        "none_passthrough=true\n",
        "[/CONTRACT_TEST_HINT]"
    );

    assert!(!contract_test_hint_should_enter_planner_loop(
        request,
        Some(&ctx)
    ));
}

#[test]
fn direct_answer_gate_clarify_cannot_override_contract_hint_planner_execution() {
    let mut route = chat_route_for_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ArchiveRead;
    route.output_contract.requires_content_evidence = true;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "Which archive should I read?".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = concat!(
        "Read notes.txt from scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip.\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=archive_read\n",
        "required_evidence_json=[\"field_value\"]\n",
        "allowed_actions_json=[\"archive_basic.read\"]\n",
        "none_passthrough=false\n",
        "[/CONTRACT_TEST_HINT]"
    );

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_contract_hint_forced_planner"));
}

fn chat_route_for_gate() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "帮我写一篇关于 RustClaw 的长文".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.86),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    }
}

fn insert_count_inventory_task(
    state: &crate::AppState,
    task_id: &str,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    path: &str,
    total: i64,
    updated_at: &str,
) {
    let output_excerpt = serde_json::json!({
        "action": "count_inventory",
        "counts": {"total": total},
        "path": path,
        "resolved_path": format!("/tmp/repo/{path}")
    })
    .to_string();
    let result_json = serde_json::json!({
        "messages": [total.to_string()],
        "task_journal": {
            "trace": {
                "step_results": [
                    {"output_excerpt": output_excerpt}
                ]
            }
        }
    })
    .to_string();
    let db = state.core.db.get().expect("db");
    db.execute(
        "INSERT INTO tasks (
                task_id, user_id, chat_id, user_key, channel, kind, payload_json,
                status, result_json, error_text, created_at, updated_at
             ) VALUES (?1, ?2, ?3, ?4, 'ui', 'ask', '{}', 'succeeded', ?5, NULL, ?6, ?6)",
        rusqlite::params![task_id, user_id, chat_id, user_key, result_json, updated_at],
    )
    .expect("insert count task");
}

fn gate_contract(
    requires_content_evidence: bool,
    locator_kind: &str,
    semantic_kind: &str,
) -> DirectAnswerGateContractOut {
    DirectAnswerGateContractOut {
        response_shape: "free".to_string(),
        exact_sentence_count: None,
        requires_content_evidence,
        delivery_required: false,
        locator_kind: locator_kind.to_string(),
        delivery_intent: "none".to_string(),
        semantic_kind: semantic_kind.to_string(),
        locator_hint: String::new(),
        self_extension: DirectAnswerGateSelfExtensionOut::default(),
    }
}

fn gate_out(decision: &str, contract: DirectAnswerGateContractOut) -> DirectAnswerGateOut {
    DirectAnswerGateOut {
        decision: decision.to_string(),
        reason: "test".to_string(),
        confidence: 0.9,
        clarify_question: String::new(),
        resolved_user_intent: "Write a grounded RustClaw article using workspace evidence."
            .to_string(),
        reference_resolution: DirectAnswerGateReferenceResolutionOut::default(),
        output_contract: contract,
    }
}

struct TempDirGuard {
    path: std::path::PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "rustclaw_ask_flow_{label}_{}_{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
fn direct_answer_gate_promotes_chat_to_planner_execute() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "none"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "summarize workspace", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(
        route.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::None
    );
    let contract_snapshot =
        crate::contract_matrix::trace_snapshot_for_route(&route).expect("contract snapshot");
    assert_eq!(
        contract_snapshot
            .get("contract_match")
            .and_then(serde_json::Value::as_str),
        Some("generic_path_content")
    );
    assert_eq!(
        contract_snapshot
            .get("locator_kind")
            .and_then(serde_json::Value::as_str),
        Some("current_workspace")
    );
    assert!(route.route_reason.contains("direct_answer_gate_execute"));
}

#[test]
fn direct_answer_gate_promotion_uses_matrix_finalize_style() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "current_workspace", "file_names");
    contract.response_shape = "free".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "list workspace files", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::planner_execute_plain());
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
    );
    assert_eq!(
        crate::contract_matrix::final_answer_shape_for_output_contract(&route.output_contract)
            .map(|shape| shape.class().as_str()),
        Some("strict_list")
    );
}

#[test]
fn direct_answer_gate_ignores_chat_promotion_without_structured_target() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("planner_execute", gate_contract(true, "path", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "Explain the category label without reading files.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_chat_promotion_without_structured_target_ignored"));
}

#[test]
fn direct_answer_gate_keeps_structural_memory_update_direct() {
    let mut route = chat_route_for_gate();
    route.should_refresh_long_term_memory = true;
    route.resolved_intent = "Update a stored alias binding and acknowledge it.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("planner_execute", gate_contract(true, "path", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "update this alias binding and acknowledge it",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::direct_answer());
    assert!(route.is_chat_gate());
    assert!(!route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_memory_update_ignored"));
}

#[test]
fn direct_answer_gate_keeps_alias_state_patch_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Update a stored alias binding and acknowledge it.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: None,
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "甲文件": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };
    let gate = gate_out("planner_execute", gate_contract(true, "path", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "update this alias binding and acknowledge it",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::direct_answer());
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_memory_update_ignored"));
}

#[test]
fn runtime_approval_wait_status_uses_structured_status_query() {
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::StatusQuery),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "runtime_status_query": {
                    "kind": "approval_wait",
                    "scope": "current_task"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        runtime_approval_wait_status_direct_answer_candidate(Some(&ctx), "en").as_deref(),
        Some("No, I am not waiting for your approval.")
    );
}

#[test]
fn runtime_approval_wait_status_ignores_unstructured_chat() {
    let route = chat_route_for_gate();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(runtime_approval_wait_status_direct_answer_candidate(Some(&ctx), "en").is_none());
}

#[test]
fn direct_answer_gate_blocks_executionless_promotion_without_target() {
    let mut route = chat_route_for_gate();
    route.route_reason =
        "User requested a text correction.; executionless_route_downgraded_to_direct_answer"
            .to_string();
    route.resolved_intent = "Correct the version reference in the relevant prior text.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "content_presence_check"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "Correction: mention Python 3.11, not Python 3.10.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::direct_answer());
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_executionless_promotion_blocked"));
}

#[test]
fn direct_answer_gate_allows_executionless_promotion_with_explicit_target() {
    let mut route = chat_route_for_gate();
    route.route_reason =
        "User requested a text correction.; executionless_route_downgraded_to_direct_answer"
            .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "content_presence_check"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "Correction: mention Python 3.11, not Python 3.10 in README.md.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
}

#[test]
fn direct_answer_gate_promotes_resolved_workspace_child_context() {
    let root = TempDirGuard::new("gate_workspace_child_context");
    std::fs::create_dir_all(root.path.join("document")).expect("document dir");
    std::fs::write(
        root.path.join("document").join("sample.png"),
        "not a real png",
    )
    .expect("sample image placeholder");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Explain how to classify images within ./document without moving files\n",
        "answer_candidate: use metadata labels"
    )
    .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "Preview how images under ./document could be categorized. Do not move files.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.ends_with("document"));
    assert!(route
        .route_reason
        .contains("direct_answer_gate_workspace_child_context_execute"));
}

#[test]
fn direct_answer_gate_does_not_promote_product_name_that_matches_workspace_child() {
    let root = TempDirGuard::new("gate_product_name_child_context");
    std::fs::write(root.path.join("rustclaw"), "#!/usr/bin/env bash\n").expect("script");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Explain RustClaw natural language contract boundaries in two sentences.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "用两句话解释 RustClaw 的自然语言契约边界，不要读取文件。",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_workspace_child_context_execute"));
}

#[test]
fn direct_answer_gate_does_not_promote_category_label_that_matches_workspace_dir() {
    let root = TempDirGuard::new("gate_category_label_child_context");
    std::fs::create_dir_all(root.path.join("logs")).expect("logs dir");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Clarify that a category label alone is not an executable file target.\n",
        "answer_candidate: Please provide a concrete target if you want file inspection."
    )
    .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "logs is only a category label here. Do not read files.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_workspace_child_context_execute"));
}

#[test]
fn direct_answer_gate_keeps_pure_chat_direct_despite_unbound_reference_label() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Explain why memory should not become hidden instructions.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
            &state,
            &mut ctx,
            "Explain in two sentences why memory should not become hidden instructions. Do not read files.",
            gate,
        );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route.needs_clarify);
}

#[test]
fn direct_answer_gate_allows_executionless_promotion_with_session_alias_target() {
    let mut route = chat_route_for_gate();
    route.route_reason =
        "User asked to inspect a session alias.; executionless_route_downgraded_to_direct_answer"
            .to_string();
    route.resolved_intent = "read_file_extract_title".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        session_alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
            alias: "note file".to_string(),
            target: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
            updated_at_ts: 1,
        }],
        ..Default::default()
    };
    let gate = gate_out("planner_execute", gate_contract(true, "path", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "read the title of the note file", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert_eq!(
        route.output_contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    );
}

#[test]
fn filename_locator_contract_is_not_filename_only_output() {
    let mut route = chat_route_for_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;

    assert!(!route_contract_requests_filename_only_output(Some(&route)));

    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    assert!(route_contract_requests_filename_only_output(Some(&route)));
}

#[test]
fn session_alias_target_direct_answer_rejects_route_only_filename_with_content_evidence() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "alias-fast-path-current".to_string(),
        user_id: 31,
        chat_id: 37,
        user_key: Some("alias-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"read note file"}).to_string(),
    };
    let conversation = crate::conversation_state::ConversationState {
        alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
            alias: "note file".to_string(),
            target: "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string(),
            updated_at_ts: 1,
        }],
        last_task_id: "alias-fast-path-prior".to_string(),
        updated_at_ts: 1,
        ..Default::default()
    };
    let state_json = serde_json::to_string(&conversation).expect("conversation json");
    state
        .core
        .db
        .get()
        .expect("db")
        .execute(
            "INSERT INTO conversation_states (
                    user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                task.user_id,
                task.chat_id,
                task.user_key.as_deref().unwrap_or_default(),
                state_json,
                conversation.last_task_id,
                conversation.updated_at_ts as i64
            ],
        )
        .expect("insert conversation state");

    let mut route = chat_route_for_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        session_alias_target_direct_answer_candidate(&state, &task, "read note file", Some(&ctx),),
        None
    );
}

#[test]
fn session_alias_target_direct_answer_allows_current_schema_filename_request() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "alias-fast-path-current-schema".to_string(),
        user_id: 41,
        chat_id: 43,
        user_key: Some("alias-user-schema".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({
            "text":"What file does the note file refer to now? Output only the basename."
        })
        .to_string(),
    };
    let conversation = crate::conversation_state::ConversationState {
        alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
            alias: "note file".to_string(),
            target: "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md".to_string(),
            updated_at_ts: 1,
        }],
        last_task_id: "alias-fast-path-current-schema-prior".to_string(),
        updated_at_ts: 1,
        ..Default::default()
    };
    let state_json = serde_json::to_string(&conversation).expect("conversation json");
    state
        .core
        .db
        .get()
        .expect("db")
        .execute(
            "INSERT INTO conversation_states (
                    user_id, chat_id, user_key, state_json, last_task_id, updated_at_ts
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                task.user_id,
                task.chat_id,
                task.user_key.as_deref().unwrap_or_default(),
                state_json,
                conversation.last_task_id,
                conversation.updated_at_ts as i64
            ],
        )
        .expect("insert conversation state");

    let mut route = chat_route_for_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        session_alias_target_direct_answer_candidate(
            &state,
            &task,
            "What file does the note file refer to now? Output only the basename.",
            Some(&ctx),
        )
        .as_deref(),
        Some("release_checklist.md")
    );
}

#[test]
fn structured_scalar_extraction_ignores_embedded_answer_candidate() {
    assert!(resolved_intent_declares_structured_scalar_extraction(
        "confirm_read_note_title\nanswer_candidate: Confirmed"
    ));
    assert!(!resolved_intent_declares_structured_scalar_extraction(
        "Read the note file title and output only the title."
    ));
}

#[test]
fn direct_answer_gate_keeps_direct_chat_when_decision_is_direct() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "hello", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::direct_answer());
    assert!(route.is_chat_gate());
    assert!(!route.output_contract.requires_content_evidence);
}

#[test]
fn direct_answer_gate_clarifies_unbound_candidate_even_when_decision_is_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Extract the name field from Cargo.toml and output only that value\n",
        "answer_candidate: rustclaw"
    )
    .to_string();
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"{"state_patch":{"deictic_reference":{"target":"unresolved_prior_object"}},"request":{"operation":"read_field","target_kind":"referenced_file","field_path":"name"}}"#;
    assert!(direct_answer_gate_candidate_needs_unbound_context_clarify(
        &state, request, &route, &gate, None, false, false, false,
    ));
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert!(route.clarify_question.is_empty());
}

#[test]
fn direct_answer_gate_keeps_contextual_summary_reference_direct_without_answer_candidate() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "总结RustClaw真实客户端连续会话测试的主要验证目标".to_string();
    route.route_reason = "这是一个对测试目的的总结请求，不是召回请求。根据规则，当请求总结/解释/判断测试验证什么时，不应将之前记住的编号作为答案。测试背景已在上下文中确认，主要验证多渠道 agent 控制台的非技术用户在真实客户端连续交互场景下的会话状态保持和系统稳定性。".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.exact_sentence_count = Some(1);
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(false, "none", "none");
    contract.response_shape = "strict".to_string();
    contract.exact_sentence_count = Some(1);
    let mut gate = gate_out("direct_answer", contract);
    gate.resolved_user_intent =
        "用一句话总结RustClaw真实客户端连续会话测试的主要验证目标".to_string();
    gate.reason = "这是对已建立的测试背景进行概念性总结的请求，测试目的已在当前会话中由用户明确描述，不需要读取本地文件或执行命令".to_string();
    gate.reference_resolution.target = "none".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "请用一句话总结这个连续会话测试主要验证什么。",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_keeps_contextual_summary_reference_direct_with_chat_candidate() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
            "用户希望用一句话总结 RustClaw 连续会话测试的主要验证目标\n",
            "answer_candidate: 该连续会话测试主要验证 RustClaw 在多渠道环境下保持客户端会话上下文连贯性的能力。"
        )
        .to_string();
    route.route_reason =
        "用户请求对测试目的进行一句话概括，属于讨论/总结类请求，无需外部证据，可直接回答。"
            .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.exact_sentence_count = Some(1);
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(false, "none", "none");
    contract.response_shape = "strict".to_string();
    contract.exact_sentence_count = Some(1);
    let mut gate = gate_out("direct_answer", contract);
    gate.resolved_user_intent = "用一句话概括 RustClaw 连续会话测试的核心验证目标".to_string();
    gate.reason = "用户要求对已明确记住的上下文进行一句话概括，属于纯讨论/总结类请求，无需外部证据"
        .to_string();
    gate.reference_resolution.target = "none".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "请用一句话总结这个连续会话测试主要验证什么。",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_keeps_preference_memory_summary_direct_without_execution_surface() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Summarize the remembered continuous-test context in one sentence.".to_string();
    route.route_reason =
        "The request depends on durable memory context, not fresh workspace evidence.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        ..Default::default()
    };
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "none"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "Summarize what this continuous test has covered so far in one short English sentence.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_preference_memory_context_ignored"));
}

#[test]
fn direct_answer_gate_still_clarifies_unbound_pathlike_context_without_candidate() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Summarize Cargo.toml package configuration.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    gate.resolved_user_intent = "Summarize Cargo.toml package configuration.".to_string();
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "summarize that package file", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn slash_joined_prose_is_not_pathlike_locator() {
    assert!(!token_looks_like_pathlike_locator(
        "总结/解释/判断测试验证什么时"
    ));
    assert!(token_looks_like_pathlike_locator("docs/reports"));
    assert!(token_looks_like_pathlike_locator("configs/config.toml"));
    assert!(token_looks_like_pathlike_locator("/var/log/system.log"));
    assert!(token_looks_like_pathlike_locator("https://example.test/a"));
}

#[test]
fn direct_answer_gate_keeps_self_contained_scalar_candidate_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "只用一句话回答：2+2 等于几\nanswer_candidate: 2+2 等于 4".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    assert!(!direct_answer_gate_candidate_needs_unbound_context_clarify(
        &state,
        "只用一句话回答：2+2 等于几，不要引用任何历史记忆",
        &route,
        &gate,
        None,
        false,
        false,
        false,
    ));
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "只用一句话回答：2+2 等于几，不要引用任何历史记忆",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_can_skip_pure_chat_draft_without_locator() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "撰写关于团队协作的长文\nanswer_candidate: ## 团队协作\n一段通用写作草稿。".to_string();

    assert!(direct_answer_gate_can_skip_for_pure_chat_draft(
        &state,
        "帮我写一篇关于团队协作的长文",
        Some(&route)
    ));
    assert_eq!(
        direct_answer_chat_user_request(
            &route.resolved_intent,
            "帮我写一篇关于团队协作的长文",
            false,
        ),
        "撰写关于团队协作的长文"
    );
}

#[test]
fn direct_answer_gate_does_not_skip_scalar_answer_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "输出当前用户名\nanswer_candidate: admin".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(!direct_answer_gate_can_skip_for_pure_chat_draft(
        &state,
        "只输出当前用户名，不要解释",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_does_not_skip_current_workspace_identity_draft() {
    let root = TempDirGuard::new("gate_workspace_identity_draft");
    let workspace = root.path.join("rustclaw");
    std::fs::create_dir_all(&workspace).expect("workspace dir");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.clone();
    state.skill_rt.default_locator_search_dir = workspace;
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "撰写关于 RustClaw 的长文\nanswer_candidate: ## RustClaw\n一段未验证的写作草稿。"
            .to_string();

    assert!(!direct_answer_gate_can_skip_for_pure_chat_draft(
        &state,
        "帮我写一篇关于 RustClaw 的长文",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_keeps_locator_draft_under_gate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "撰写关于配置文件的长文\nanswer_candidate: ## 配置文件\n一段未验证的写作草稿。".to_string();

    assert!(!direct_answer_gate_can_skip_for_pure_chat_draft(
        &state,
        "帮我写一篇关于 configs/config.toml 的长文",
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_accepts_distinctive_candidate_bound_in_memory_context() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "recall_scalar\nanswer_candidate: RC-CONT-CN-0428-A".to_string();
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        memory_context_for_execution: Some(
            "### MEMORY_CONTEXT (NOT CURRENT REQUEST)\n\
#### RELEVANT_FACTS\n\
- 当前连续测试的编号为 RC-CONT-CN-0428-A，助手应记住并在后续任务中引用。"
                .to_string(),
        ),
        ..Default::default()
    };

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "刚才让你记住的连续测试编号是什么？只回答编号。",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_keeps_observed_result_interpretation_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Judge whether the previously observed log entries contain abnormal patterns".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_ANCHOR\n\
- latest_request=tail that log 10 lines\n\
- latest_result=2026-04-01 WARN cache miss ratio above baseline | 2026-04-01 ERROR provider timeout while fetching external metadata\n\
### RECENT_EXECUTION_EVENTS\n\
- kind=ask request=tail that log 10 lines result=2026-04-01 ERROR provider timeout while fetching external metadata"
                .to_string(),
        ),
        ..Default::default()
    };
    let mut gate = gate_out(
        "planner_execute",
        gate_contract(true, "none", "content_excerpt_summary"),
    );
    gate.reference_resolution.target = "current_action_result".to_string();
    gate.resolved_user_intent =
        "Judge whether the already observed log entries contain anything abnormal.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "in one sentence tell me if anything looks abnormal",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_existing_observed_result_ignored"));
}

#[test]
fn direct_answer_gate_keeps_observed_failure_explanation_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "如果文件不存在，则简短说明原因\nanswer_candidate: 文件不存在，路径可能错误或文件已删除。"
            .to_string();
    route.output_contract.requires_content_evidence = true;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_ANCHOR\n\
- latest_request=把那个文件发给我\n\
- latest_result=未找到文件：/tmp/not_exists.md，所以无法发送。请确认完整路径或上传该文件\n\
### RECENT_EXECUTION_EVENTS\n\
- kind=ask request=把那个文件发给我 result=未找到文件：/tmp/not_exists.md，所以无法发送。"
                .to_string(),
        ),
        ..Default::default()
    };
    let mut gate = gate_out("direct_answer", gate_contract(true, "none", "none"));
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "如果不存在就简短说明原因", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_existing_observed_result_ignored"));
}

#[test]
fn direct_answer_gate_still_promotes_locatorless_runtime_observation() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_ANCHOR\n- latest_request=list logs\n- latest_result=app.log"
                .to_string(),
        ),
        ..Default::default()
    };
    let mut gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    gate.reference_resolution.target = "none".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "what is the hostname?", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
}

#[test]
fn active_ordered_entries_count_returns_scalar_count() {
    let mut route = chat_route_for_gate();
    route.needs_clarify = true;
    route.set_first_layer_decision(crate::FirstLayerDecision::Clarify);
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "### ACTIVE_EXECUTION_ANCHOR\n\
followup_bound_target: /tmp/docs\n\
followup_ordered_entries: 1:archive | 2:release_checklist.md | 3:service_notes.md\n\
observed_ordered_entries: 1:archive | 2:release_checklist.md | 3:service_notes.md"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        active_ordered_entries_count_direct_answer_candidate(
            "然后告诉我一共有多少个直接子项，只输出数字",
            Some(&ctx),
        ),
        Some("3".to_string())
    );
}

#[test]
fn direct_answer_gate_clarifies_locatorless_target_specific_planner_request() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    gate.resolved_user_intent =
        "Find the SQLite database in the current project and query the schema version value."
            .to_string();
    gate.reference_resolution.target = "missing_locator".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &crate::IntentOutputContract::default(),
    );

    assert!(direct_answer_gate_planner_needs_unbound_locator_clarify(
        &state,
        "check the schema version of that sqlite database",
        &contract,
        &gate.reference_resolution,
        None,
        false,
    ));

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "check the schema version of that sqlite database",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_allows_locatorless_targetless_planner_request() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &crate::IntentOutputContract::default(),
    );

    assert!(!direct_answer_gate_planner_needs_unbound_locator_clarify(
        &state,
        "detect the current runtime package manager",
        &contract,
        &gate.reference_resolution,
        None,
        false,
    ));

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "detect the current runtime package manager",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_clarifies_unbound_path_candidate_for_delivery_and_preserves_contract() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Deliver the requested local config file without pasting its body\n",
        "answer_candidate: /tmp/untrusted/config.toml"
    )
    .to_string();
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "file_token".to_string();
    contract.delivery_required = true;
    contract.delivery_intent = "file_single".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "send me the local config file without pasting the body",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
}

#[test]
fn direct_answer_gate_allows_locatorless_scalar_runtime_execution() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "runtime_scalar\nanswer_candidate: not-current-runtime-user-000".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "scalar".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.resolved_user_intent = "Report the current runtime account name.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "current runtime account", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(!route.needs_clarify);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar
    );
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_keeps_verified_runtime_identity_scalar_direct() {
    let Some(runtime_user) = ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    else {
        return;
    };
    let mut route = chat_route_for_gate();
    route.resolved_intent = format!("runtime_scalar\nanswer_candidate: {runtime_user}");
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "scalar".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.resolved_user_intent = "Report the current runtime account name.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "current runtime account", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route.is_execute_gate());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_bound_candidate_evidence"));
}

#[test]
fn direct_answer_gate_clarifies_unbound_existing_file_delivery_without_locator() {
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
        "Deliver the local configuration file without pasting content.".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "current_workspace", "none");
    contract.delivery_required = true;
    contract.response_shape = "file_token".to_string();
    contract.delivery_intent = "file_single".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "把那份本地配置直接甩给我，别贴正文",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route.clarify_question.is_empty());
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_allows_generated_file_delivery_without_locator() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "current_workspace", "generated_file_delivery");
    contract.delivery_required = true;
    contract.response_shape = "file_token".to_string();
    contract.delivery_intent = "file_single".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "写一份部署清单，保存成 md 文件发给我",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::GeneratedFileDelivery
    );
}

#[test]
fn direct_answer_gate_allows_locatorless_workspace_project_summary_semantic() {
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "none", "workspace_project_summary"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();
    let contract = output_contract_from_direct_answer_gate(
        gate.output_contract.clone(),
        &crate::IntentOutputContract::default(),
    );

    assert!(!direct_answer_gate_planner_needs_unbound_locator_clarify(
        &state,
        "summarize this project",
        &contract,
        &gate.reference_resolution,
        None,
        false,
    ));
}

#[test]
fn direct_answer_gate_promotes_artifact_listing_candidate_to_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
            "List the first five entries under the selected workspace directory\n",
            "answer_candidate: act_plan.log, clawd.log, clawd.run.log, clawd.test.log, clawd_manual.log"
        )
        .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "list the selected logs", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::planner_execute_plain());
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
    );
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_artifact_listing_execute"));
}

#[test]
fn direct_answer_gate_does_not_promote_non_artifact_example_list() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Give simple examples\nanswer_candidate: apple, banana, cherry".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "give examples", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route.output_contract.requires_content_evidence);
}

#[test]
fn direct_answer_gate_promotes_inline_json_transform_to_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Apply the provided structured transform payload\nanswer_candidate: beta, alpha"
            .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"sort","by":"score","order":"desc"},{"op":"project","fields":["name"]}]}"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_inline_transform_execute"));
}

#[test]
fn direct_answer_gate_promotes_inline_json_table_candidate_to_transform_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Sort provided JSON array by score descending and output as markdown table.\nanswer_candidate: | name | score |\n|------|-------|\n| beta | 12 |\n| gamma | 9 |\n| alpha | 7 |".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "strict".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"把这个 JSON 数组按 score 从高到低排序，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_inline_transform_execute"));
}

#[test]
fn direct_answer_gate_promotes_inline_json_planner_without_candidate_to_transform_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Sort provided JSON array by score descending and output as markdown table.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(false, "none", "none");
    contract.response_shape = "strict".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"把这个 JSON 数组按 score 从高到低排序，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12},{"name":"gamma","score":9}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_inline_transform_execute"));
}

#[test]
fn direct_answer_gate_marks_contextual_inline_payload_execution() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "none", "none");
    contract.response_shape = "strict".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("inline_structured_payload_context_execute"));
}

#[test]
fn inline_json_transform_context_promotion_uses_strict_execution_contract() {
    let mut route = chat_route_for_gate();
    route.route_reason = "executionless_route_downgraded_to_direct_answer".to_string();
    route.resolved_intent =
        "Transform inline JSON.\nanswer_candidate: [{\"city\":\"Tokyo\"},{\"city\":\"Osaka\"}]"
            .to_string();
    let request = r#"{"action":"transform_data","data":[{"city":"Tokyo","temp":22},{"city":"Osaka","temp":24}],"ops":[{"op":"project","fields":["city"]}]}"#;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(promote_inline_json_transform_context_to_planner(
        &mut ctx, request
    ));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Strict
    );
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("inline_json_transform_structured_execute"));
    assert_eq!(
        route.resolved_intent,
        format!("{request}\nanswer_candidate: [{{\"city\":\"Tokyo\"}},{{\"city\":\"Osaka\"}}]")
    );
}

#[test]
fn direct_answer_gate_promotes_explicit_readme_summary_to_planner() {
    let root = TempDirGuard::new("gate_bare_readme_summary");
    std::fs::write(root.path.join("README.md"), "# Demo\n\nLocal readme body")
        .expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let mut route = chat_route_for_gate();
    route.resolved_intent =
            "Read the README and summarize it in exactly three sentences\nanswer_candidate: synthetic summary"
                .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "current_workspace", "none");
    contract.locator_hint = "README or README.md".to_string();
    contract.exact_sentence_count = Some(3);
    let gate = gate_out("planner_execute", contract);

    let current_request = "读一下 README.md 然后用恰好三句话总结，不要多也不要少";
    assert!(locator_hint_mentions_current_request(
        "README or README.md",
        current_request
    ));
    assert!(current_request_mentions_resolvable_gate_locator(
        &state,
        current_request,
        &crate::IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: "README or README.md".to_string(),
            ..crate::IntentOutputContract::default()
        },
    ));

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, current_request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.path.join("README.md").display().to_string()
    );
    assert!(route.route_reason.contains("direct_answer_gate_"));
    assert!(route.route_reason.contains("_execute"));
}

#[test]
fn direct_answer_gate_promotes_package_manager_detect_to_planner() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "package manager detection\nanswer_candidate: not observed".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::PackageManagerDetection;
    route.output_contract.requires_content_evidence = true;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = "consulta el gestor de paquetes detectado";

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::PackageManagerDetection
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_package_manager_detect_execute"));
}

#[test]
fn direct_answer_gate_promotes_package_install_preview_without_locator() {
    let mut route = chat_route_for_gate();
    route.route_reason =
            "llm_semantic_contract_repair:dry_run_command_discovery_requires_local_observation; executionless_route_downgraded_to_direct_answer"
                .to_string();
    route.resolved_intent =
        "package preview\nanswer_candidate: command: sudo -n apt-get install -y ripgrep"
            .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("planner_execute", gate_contract(true, "none", "none"));
    gate.resolved_user_intent =
        "Show the package install dry-run preview without installing.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "ripgrep 설치는 하지 말고 dry-run 으로 어떤 명령이 될지만 알려줘.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert!(route.resolved_intent.contains("answer_candidate:"));
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_executionless_promotion_blocked"));
}

#[test]
fn direct_answer_gate_can_skip_self_contained_inline_json_explanation() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Explain inline JSON records\nanswer_candidate: two score records".to_string();
    let request =
        r#"解释这个 JSON 代表什么：[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);

    assert!(
        direct_answer_gate_can_skip_for_self_contained_payload(request, Some(&route),),
        "surface={surface:?}"
    );
}

#[test]
fn direct_answer_gate_keeps_self_contained_inline_json_array_explanation_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Explain the meaning and structure of the provided JSON array: ",
        r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]. "#,
        "Preserve the original order as specified."
    )
    .to_string();
    route.route_reason = "The request is for explanation/interpretation of embedded structured data. The user explicitly specifies no sorting. This is a pure discussion task requiring no external retrieval, execution, or workspace inspection.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("direct_answer", gate_contract(false, "none", "none"));
    gate.resolved_user_intent =
        "Explain the meaning and structure of the provided JSON array.".to_string();
    gate.reason =
        "Self-contained embedded structured data; no external retrieval is needed.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();
    let request = r#"Explain what this JSON represents without sorting it: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
}

#[test]
fn direct_answer_gate_does_not_skip_inline_json_transform_payload() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "Apply the provided structured transform payload\nanswer_candidate: beta, alpha"
            .to_string();
    let request = r#"{"action":"transform_data","data":[{"name":"alpha","score":7},{"name":"beta","score":12}],"ops":[{"op":"sort","by":"score","order":"desc"},{"op":"project","fields":["name"]}]}"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);

    assert!(
        !direct_answer_gate_can_skip_for_self_contained_payload(request, Some(&route),),
        "surface={surface:?}"
    );
}

#[test]
fn direct_answer_gate_skip_rejects_locator_payload() {
    let mut route = chat_route_for_gate();
    route.output_contract.locator_hint = "README.md".to_string();

    assert!(!direct_answer_gate_can_skip_for_self_contained_payload(
        r#"读取 README.md 并按 [{"field":"score"}] 排序"#,
        Some(&route),
    ));
}

#[test]
fn direct_answer_gate_skips_active_text_mutation_without_locator() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({"format": "three-step checklist"})),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_active_task_text_mutation(
        "Actually switch it to a three-step checklist.",
        Some(&ctx)
    ));
}

#[test]
fn direct_answer_gate_skips_active_text_mutation_with_interrupt_flag() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: true,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["80 characters", "body only"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_active_task_text_mutation(
        "Make it less technical, under 80 characters, body only.",
        Some(&ctx)
    ));
}

#[test]
fn direct_answer_gate_skips_active_observed_output_chat_repair() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    route.route_reason = "active_observed_output_chat_repair".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_active_observed_output_chat_repair(Some(&ctx)));
}

#[test]
fn direct_answer_gate_outcome_preserves_active_text_mutation_from_clarify() {
    let mut route = chat_route_for_gate();
    route.route_confidence = Some(0.72);
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: true,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["80 characters", "body only"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "Need a topic before rewriting.".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "Make it less technical, under 80 characters, body only.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route
        .route_reason
        .contains("direct_answer_gate_active_task_text_mutation_ignored"));
    assert!(!route.needs_clarify);
}

#[test]
fn chat_route_context_keeps_active_text_mutation_draft_as_semantic_anchor() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "修正当前方案文档的目标用户描述，将受众从老板改为开发者".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        semantic_answer_candidate_draft: Some(
            "目标用户：开发者。正文应围绕开发者的使用场景展开。".to_string(),
        ),
        ..Default::default()
    };

    let context = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(context.contains("active_task_semantic_draft:"));
    assert!(context.contains("开发者"));
    assert!(context.contains("Non-evidence writing draft"));
}

#[test]
fn chat_route_context_exposes_structured_required_visible_literals() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Update the active draft for the corrected audience.".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["开发者"],
                "forbidden_visible_literals": ["老板"],
                "replacement_pairs": [{"from": "老板", "to": "开发者"}]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let context = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(context.contains("active_task_required_visible_literals: 开发者"));
    assert!(context.contains("active_task_replacement_pairs: 老板 -> 开发者"));
    assert!(context.contains("active_task_forbidden_visible_literals: 老板"));
    assert!(context.contains("must visibly contain"));
}

#[test]
fn required_visible_literals_accepts_protocol_aliases() {
    let state_patch = serde_json::json!({
        "required_visible_literals": ["开发者", " developer "],
        "visible_constraints": {
            "literals": [{"literal": "`SDK v2`"}]
        }
    });

    assert_eq!(
        required_visible_literals_from_state_patch(&state_patch),
        vec!["开发者", "developer", "SDK v2"]
    );
}

#[test]
fn replacement_pairs_and_forbidden_literals_accept_structured_protocol() {
    let state_patch = serde_json::json!({
        "replacement_pairs": [
            {"from": "老板", "to": "开发者"},
            {"old": "v1", "new": "v2"}
        ],
        "visible_constraints": {
            "forbidden_visible_literals": ["internal only"]
        }
    });

    assert_eq!(
        replacement_pairs_from_state_patch(&state_patch),
        vec![
            super::ActiveTaskReplacementPair {
                from: "老板".to_string(),
                to: "开发者".to_string()
            },
            super::ActiveTaskReplacementPair {
                from: "v1".to_string(),
                to: "v2".to_string()
            }
        ]
    );
    assert_eq!(
        forbidden_visible_literals_from_state_patch(&state_patch),
        vec!["internal only", "老板", "v1"]
    );
}

#[test]
fn active_task_required_visible_literal_guard_prefixes_missing_literal() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "replacement_pairs": [{"from": "老板", "to": "开发者"}]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "系统瓶颈影响交付，目标提升吞吐量。".to_string(),
        Some(&ctx),
    );

    assert!(answer.starts_with("开发者: "));
}

#[test]
fn active_task_required_visible_literal_guard_ignores_untyped_output_constraints() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_visible_literals": ["under 80 characters", "body only"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "Invest in this focused plan to reduce risk and improve delivery speed.".to_string(),
        Some(&ctx),
    );

    assert_eq!(
        answer,
        "Invest in this focused plan to reduce risk and improve delivery speed."
    );
}

#[test]
fn active_task_required_visible_literal_guard_leaves_existing_literal() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "required_content_literals": ["developer"]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let answer = ensure_active_task_required_visible_literals(
        "This version is for Developer onboarding.".to_string(),
        Some(&ctx),
    );

    assert_eq!(answer, "This version is for Developer onboarding.");
}

#[test]
fn direct_answer_gate_does_not_skip_active_text_mutation_with_explicit_file_target() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskCorrect),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({"format": "three-step checklist"})),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert!(!direct_answer_gate_can_skip_for_active_task_text_mutation(
        "In README.md, switch it to a three-step checklist.",
        Some(&ctx)
    ));
}

#[test]
fn direct_answer_gate_ignores_background_only_promotion_for_bound_answer_candidate() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "User wants to output only the final checklist.\nanswer_candidate: final_checklist"
            .to_string();
    let promoted_contract = crate::IntentOutputContract {
        requires_content_evidence: true,
        locator_kind: crate::OutputLocatorKind::Path,
        locator_hint: "README.md".to_string(),
        ..crate::IntentOutputContract::default()
    };

    assert!(
        direct_answer_gate_promotion_depends_only_on_background_context(
            &crate::AppState::test_default_with_fixture_provider(),
            "Output only the final checklist.",
            &route,
            &promoted_contract,
            &DirectAnswerGateReferenceResolutionOut::default(),
            false,
        )
    );
}

#[test]
fn direct_answer_gate_keeps_deictic_file_followup_promotable() {
    let mut route = chat_route_for_gate();
    route.resolved_intent =
        "User wants the selected file.\nanswer_candidate: README.md".to_string();
    let promoted_contract = crate::IntentOutputContract {
        requires_content_evidence: true,
        delivery_required: true,
        locator_kind: crate::OutputLocatorKind::Path,
        delivery_intent: crate::OutputDeliveryIntent::FileSingle,
        locator_hint: "README.md".to_string(),
        ..crate::IntentOutputContract::default()
    };

    assert!(
        !direct_answer_gate_promotion_depends_only_on_background_context(
            &crate::AppState::test_default_with_fixture_provider(),
            "Send that file.",
            &route,
            &promoted_contract,
            &DirectAnswerGateReferenceResolutionOut {
                target: "current_action_result".to_string(),
            },
            false,
        )
    );
}

#[test]
fn recent_file_context_promotion_ignores_sentence_punctuation() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
            "Acknowledge that no concrete target is bound.\n",
            "answer_candidate: Understood. No file read triggered. If you need a specific path, name it."
        )
        .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_EVENTS\n\
                 - ts=2 kind=ask request=read configs/config.toml result=ok\n\
                 - ts=1 kind=ask request=read README.md result=ok"
                .to_string(),
        ),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome = apply_direct_answer_gate_outcome(
        &crate::AppState::test_default_with_fixture_provider(),
        &mut ctx,
        "Acknowledge only; no current target is bound.",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.is_execute_gate());
    assert!(!route
        .route_reason
        .contains("direct_answer_gate_recent_file_context_execute"));
}

#[test]
fn direct_answer_gate_promotes_artifact_candidate_with_recent_file_targets_to_planner() {
    let root = TempDirGuard::new("recent_file_targets");
    let readme = root.path.join("README.md");
    let notes = root.path.join("service_notes.md");
    std::fs::write(&readme, "# Demo\nmentions app_config.toml\n").expect("write readme");
    std::fs::write(&notes, "# Service\nrestart notes\n").expect("write notes");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "Compare the previous file targets in one sentence\n",
        "answer_candidate: app_config.toml is config; service_notes.md is service notes"
    )
    .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(format!(
            "### RECENT_EXECUTION_EVENTS\n\
                 - ts=2 kind=ask request=read {} result=- `app_config.toml`: sample config\n\
                 - ts=1 kind=ask request=read {} result=service restart notes",
            readme.display(),
            notes.display()
        )),
        ..Default::default()
    };
    let gate = gate_out("direct_answer", gate_contract(false, "none", "none"));

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "compare the recent files", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert_eq!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert!(route
        .route_reason
        .contains("direct_answer_gate_recent_file_context_execute"));
}

#[test]
fn direct_answer_gate_context_marks_answer_candidate_as_unobserved() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "get current runtime scalar\nanswer_candidate: stale_value".to_string();
    route.route_reason = "prior normalizer said direct answer".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    let context = direct_answer_gate_route_context(Some(&ctx));

    assert!(context.contains("### PRIOR_ROUTE_CONTEXT"));
    assert!(context.contains("resolved_user_intent: get current runtime scalar"));
    assert!(context.contains("normalizer_answer_candidate_present: true"));
    assert!(context.contains("not runtime evidence"));
    assert!(context.contains("prior_route_reason: prior normalizer said direct answer"));
    assert!(!context.contains("stale_value"));
    assert!(!context.contains("answer_candidate: stale_value"));
}

#[test]
fn direct_answer_gate_recent_execution_context_exposes_targets_not_excerpt_paths() {
    let ctx = crate::agent_engine::AgentRunContext {
            cross_turn_recent_execution_context: Some(
                "### RECENT_EXECUTION_EVENTS\n- request=read /tmp/README.md result=- `/tmp/config.toml`: sample config"
                    .to_string(),
            ),
            ..Default::default()
        };

    let context = direct_answer_gate_recent_execution_context(Some(&ctx));

    assert!(context.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(context.contains("Previous executed targets are authoritative"));
    assert!(context.contains("Paths mentioned inside a prior file excerpt are content"));
    assert!(context.contains("/tmp/README.md"));
    assert!(context.contains("/tmp/config.toml"));
}

#[test]
fn direct_answer_gate_promotes_contract_evidence_even_when_decision_is_direct() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "content_excerpt_summary");
    contract.locator_hint = "/tmp/clawd.log".to_string();
    let gate = gate_out("direct_answer", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "summarize /tmp/clawd.log", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(
        route.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, "/tmp/clawd.log");
    assert!(route
        .route_reason
        .contains("direct_answer_gate_contract_execute"));
}

#[test]
fn direct_answer_gate_binds_resolvable_workspace_child_locator() {
    let root = TempDirGuard::new("gate_workspace_child");
    std::fs::create_dir_all(root.path.join("docs")).expect("create docs");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "content_excerpt_summary");
    contract.locator_hint = "docs".to_string();
    let gate = gate_out("planner_execute", contract);

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "look at the docs folder", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        route.output_contract.locator_hint,
        root.path.join("docs").display().to_string()
    );
}

#[test]
fn direct_answer_gate_binds_deictic_request_when_request_itself_resolves_target() {
    let root = TempDirGuard::new("gate_deictic_workspace_child");
    std::fs::create_dir_all(root.path.join("docs")).expect("create docs");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "content_excerpt_summary");
    contract.locator_hint = "docs".to_string();
    let gate = gate_out("planner_execute", contract);

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "look at the docs folder and summarize it",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(
        route.output_contract.locator_hint,
        root.path.join("docs").display().to_string()
    );
}

#[test]
fn direct_answer_gate_clarifies_unbound_deictic_observation_instead_of_guessing_locator() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "获取指定文件中 name 字段的值\nanswer_candidate: rustclaw".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "structured_keys");
    contract.locator_hint = "Cargo.toml".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "读一下那个文件里的名字字段，只输出值",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::clarify());
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_allows_deictic_observation_with_structured_auto_locator() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some("/tmp/bound/package.json".to_string()),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "structured_keys");
    contract.locator_hint = "/tmp/bound/package.json".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "读一下那个文件里的名字字段，只输出值",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_hint,
        "/tmp/bound/package.json"
    );
}

#[test]
fn direct_answer_gate_clarifies_deictic_observation_with_gate_locator_hint_only() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint = "/tmp/bound/README.md".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "把那个文件开头读 10 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_clarifies_claimed_current_locator_without_current_surface() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "current_turn_locator".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "读一下那个文件前 3 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_clarifies_locator_hint_without_current_surface_or_reference_report() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint =
        "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/README.md".to_string();
    let gate = gate_out("planner_execute", contract);
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "读一下那个文件前 3 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn direct_answer_gate_allows_deictic_observation_with_authoritative_anchor() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        has_authoritative_deictic_anchor: true,
        ..Default::default()
    };
    let mut contract = gate_contract(true, "path", "none");
    contract.locator_hint = "/tmp/bound/README.md".to_string();
    let mut gate = gate_out("planner_execute", contract);
    gate.reference_resolution.target = "unresolved_prior_object".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "把那个文件开头读 10 行", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(route.output_contract.locator_hint, "/tmp/bound/README.md");
    assert!(route.output_contract.requires_content_evidence);
}

#[test]
fn direct_answer_gate_allows_current_workspace_summary_with_deictic_surface() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "workspace_project_summary"),
    );
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "先看当前目录顶层主要文件夹，再用一句话解释这个仓库怎么分区",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(!direct_answer_gate_promotion_needs_unbound_deictic_clarify(
        &state,
        "先看当前目录顶层主要文件夹，再用一句话解释这个仓库怎么分区",
        None,
        false,
        false,
        &crate::IntentOutputContract {
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            semantic_kind: crate::OutputSemanticKind::None,
            ..Default::default()
        },
        &DirectAnswerGateReferenceResolutionOut {
            target: "current_action_result".to_string(),
        },
    ));
}

#[test]
fn direct_answer_gate_clarifies_current_workspace_when_reference_is_unbound() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out(
        "planner_execute",
        gate_contract(true, "current_workspace", "content_presence_check"),
    );
    gate.reference_resolution.target = "missing_locator".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome =
        apply_direct_answer_gate_outcome(&state, &mut ctx, "查看指定 schema 的 enum", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::Clarify(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.needs_clarify);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_unbound_deictic_clarify"));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route.output_contract.locator_hint.is_empty());
}

#[test]
fn recent_count_comparison_uses_completed_count_inventory_tasks() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 7;
    let chat_id = 9;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-current".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent =
            "Compare the two most recent count_inventory observations and report the selected target label."
                .to_string();
    route.route_reason = "structured_quantity_comparison".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "max",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "上一个和上上个哪个更多，只回答目录名",
            Some(&ctx),
        )
        .as_deref(),
        Some("scripts")
    );
}

#[test]
fn recent_count_comparison_overrides_bad_direct_answer_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 17;
    let chat_id = 19;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-direct".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent =
            "Compare the two observed count_inventory totals and return only the selected target label.\nanswer_candidate: 当前范围"
                .to_string();
    route.route_reason = "structured_quantity_comparison".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "max",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "上一个和上上个哪个更多，只回答目录名",
            Some(&ctx),
        )
        .as_deref(),
        Some("scripts")
    );
}

#[test]
fn recent_count_comparison_uses_min_selection_from_state_patch() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 27;
    let chat_id = 29;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-direct-min".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.resolved_intent =
            "Compare the two observed count_inventory totals and return only the selected target label."
                .to_string();
    route.route_reason = "structured_quantity_comparison".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "quantity_comparison": {
                    "selection": "min",
                    "source": "recent_count_inventory"
                }
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    assert_eq!(
        recent_count_comparison_direct_answer(
            &state,
            &task,
            "上一个和上上个哪个更多，只回答目录名",
            Some(&ctx),
        )
        .as_deref(),
        Some("document")
    );
}

#[test]
fn recent_count_comparison_ignores_missing_structured_selection() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 37;
    let chat_id = 39;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-missing-selection".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"上一个和上上个哪个更多，只回答目录名"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.ask_mode = crate::AskMode::direct_answer();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(recent_count_comparison_direct_answer(
        &state,
        &task,
        "上一个和上上个哪个更多，只回答目录名",
        Some(&ctx),
    )
    .is_none());
}

#[test]
fn direct_answer_gate_skips_recent_count_scalar_context_without_structured_selection() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 47;
    let chat_id = 49;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        64,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "/tmp/repo/document",
        34,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-skip-gate".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the two recent counts, scalar only"})
            .to_string(),
    };
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_recent_count_context(
        &state,
        &task,
        Some(&ctx),
    ));
}

#[test]
fn direct_answer_gate_skips_recent_count_context_even_when_shape_is_free() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let user_id = 57;
    let chat_id = 59;
    let user_key = "user-key";
    insert_count_inventory_task(
        &state,
        "count-scripts",
        user_id,
        chat_id,
        user_key,
        "scripts",
        65,
        "2026-05-18T08:00:00Z",
    );
    insert_count_inventory_task(
        &state,
        "count-document",
        user_id,
        chat_id,
        user_key,
        "document",
        36,
        "2026-05-18T08:01:00Z",
    );
    let task = crate::ClaimedTask {
        task_id: "compare-skip-gate-free".to_string(),
        user_id,
        chat_id,
        user_key: Some(user_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"compare the two recent counts"}).to_string(),
    };
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert!(direct_answer_gate_can_skip_for_recent_count_context(
        &state,
        &task,
        Some(&ctx),
    ));
}

#[test]
fn direct_answer_gate_promotes_chat_to_clarify_when_blocker_is_missing() {
    let route = chat_route_for_gate();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "要创建的文件夹叫什么名字？".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "create a folder", gate);

    assert!(
        matches!(outcome, DirectAnswerPreflight::Clarify(question) if question == "要创建的文件夹叫什么名字？")
    );
    let route = ctx.route_result.expect("route");
    assert_eq!(route.ask_mode, crate::AskMode::clarify());
    assert!(route.is_clarify_gate());
    assert!(route.needs_clarify);
    assert_eq!(route.clarify_question, "要创建的文件夹叫什么名字？");
    assert!(route.route_reason.contains("direct_answer_gate_clarify"));
}

#[test]
fn direct_answer_gate_clarify_preserves_existing_file_delivery_contract() {
    let mut route = chat_route_for_gate();
    route.wants_file_delivery = true;
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    route.output_contract.delivery_intent = crate::OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "Which file should I send?".to_string();
    let state = crate::AppState::test_default_with_fixture_provider();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "send that file", gate);

    assert!(
        matches!(outcome, DirectAnswerPreflight::Clarify(question) if question == "Which file should I send?")
    );
    let route = ctx.route_result.expect("route");
    assert!(route.is_clarify_gate());
    assert!(route.needs_clarify);
    assert!(route.wants_file_delivery);
    assert!(route.output_contract.delivery_required);
    assert_eq!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::FileToken
    );
    assert_eq!(
        route.output_contract.delivery_intent,
        crate::OutputDeliveryIntent::FileSingle
    );
}

#[test]
fn chat_prompt_context_appends_authoritative_route_resolution() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason:
            "'上一个'=assistant[-1](document,17), '上上个'=assistant[-2](scripts,48); scripts 更多"
                .to_string(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_hint: "scripts".to_string(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let rendered = chat_prompt_context_with_route_resolution(
        "### MEMORY_CONTEXT\nRECENT_ASSISTANT_RESULTS\n- old summary",
        Some(&ctx),
    );
    assert!(rendered.contains("### ROUTE_RESOLUTION"));
    assert!(rendered.contains("resolved_user_intent: 上一个和上上个哪个更多，只回答目录名"));
    assert!(rendered.contains("locator_hint: scripts"));
    assert!(rendered.contains("scripts 更多"));
}

#[test]
fn chat_prompt_context_replaces_empty_placeholder_with_route_resolution() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "client-like-continuous-20260428_144029".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));
    assert!(!rendered.contains("<none>"));
    assert!(rendered.contains("### ROUTE_RESOLUTION"));
    assert!(rendered.contains("client-like-continuous-20260428_144029"));
}

#[test]
fn chat_prompt_context_includes_recent_execution_when_contract_requires_evidence() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "Summarize the observed README excerpt in one sentence".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "prior observed content is available".to_string(),
        route_confidence: Some(0.94),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            requires_content_evidence: true,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        cross_turn_recent_execution_context: Some(
            "read_range path=/tmp/README.md\n# RustClaw\nlocal Rust agent runtime".to_string(),
        ),
        ..Default::default()
    };

    let rendered = chat_prompt_context_with_route_resolution("<none>", Some(&ctx));

    assert!(rendered.contains("### ROUTE_RESOLUTION"));
    assert!(rendered.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(rendered.contains("local Rust agent runtime"));
}

#[test]
fn chat_user_request_preserves_inline_structured_prompt_when_resolution_dropped_payload() {
    let prompt = r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let resolved =
        "Sort the provided JSON array by score in descending order and output as a markdown table";
    assert_eq!(chat_user_request(resolved, prompt), prompt);
}

#[test]
fn chat_request_for_prompt_keeps_original_constraints_and_semantic_anchor() {
    let request = chat_request_for_prompt(
        "刚才我让你记住的测试编号是什么？只回答编号。",
        "client-like-continuous-20260428_144029",
    );
    assert!(request.contains("Original user request:"));
    assert!(request.contains("只回答编号"));
    assert!(request.contains("Resolved semantic intent / answer candidate:"));
    assert!(request.contains("client-like-continuous-20260428_144029"));
    assert!(request.contains("output only the resolved value"));
}

#[test]
fn direct_answer_chat_user_request_strips_unapproved_answer_candidate() {
    let unapproved = direct_answer_chat_user_request(
        "get current hostname\nanswer_candidate: stale-user",
        "只输出当前机器 hostname，不要解释",
        false,
    );
    assert_eq!(unapproved, "get current hostname");

    let approved = direct_answer_chat_user_request(
        "recall stored id\nanswer_candidate: client-like-continuous-20260428_144029",
        "刚才我让你记住的测试编号是什么？只回答编号。",
        true,
    );
    assert!(approved.contains("answer_candidate: client-like-continuous-20260428_144029"));
}

#[test]
fn task_payload_text_preserves_raw_current_turn_for_chat_language_hint() {
    let task = crate::ClaimedTask {
        task_id: "task".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"先只看登录模块"}).to_string(),
    };
    assert_eq!(task_payload_text(&task).as_deref(), Some("先只看登录模块"));
}

#[test]
fn chat_reply_does_not_attach_context_process_message() {
    let reply = ask_reply_with_chat_process("RustClaw 是本地 agent 运行时。".to_string(), "zh-CN");

    assert_eq!(reply.text, "RustClaw 是本地 agent 运行时。");
    assert!(reply.messages.is_empty());
}

#[test]
fn english_chat_reply_does_not_attach_execution_process_message() {
    let reply = ask_reply_with_chat_process("RustClaw is a local agent runtime.".to_string(), "en");

    assert_eq!(reply.text, "RustClaw is a local agent runtime.");
    assert!(reply.messages.is_empty());
}

#[test]
fn alias_state_patch_uses_structured_ack_without_chat_llm() {
    let ctx = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [
                    {
                        "alias": "that docs dir",
                        "target": "/tmp/docs"
                    }
                ]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = state_patch_alias_bindings_ack(Some(&ctx), "zh-CN").unwrap();

    assert_eq!(reply.text, "已记住：`that docs dir` -> `/tmp/docs`。");
    assert!(reply.messages.is_empty());
}

#[test]
fn structural_alias_ack_uses_quote_and_single_locator_without_gate_llm() {
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(chat_route_for_gate()),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = structural_alias_binding_ack(
        Some(&ctx),
        "再记一下“乙”指 /tmp/device/docs/service_notes.md",
        "record alias to /tmp/device/docs/service_notes.md",
        "zh-CN",
    )
    .unwrap();

    assert_eq!(
        reply.text,
        "已记住：`乙` -> `/tmp/device/docs/service_notes.md`。"
    );
    assert!(reply.messages.is_empty());
}

#[test]
fn alias_state_patch_ack_accepts_alias_only_task_misclassification() {
    let ctx = crate::agent_engine::AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [
                    {
                        "alias": "that docs dir",
                        "target": "/tmp/docs"
                    }
                ]
            })),
            attachment_processing_required: false,
        }),
        ..Default::default()
    };

    let reply = state_patch_alias_bindings_ack(Some(&ctx), "zh-CN").unwrap();
    assert_eq!(reply.text, "已更新。");
    assert!(reply.messages.is_empty());
}

#[test]
fn response_language_hint_prefers_current_request_language() {
    assert_eq!(
        crate::language_policy::preferred_response_language_hint("写个两句短诗", None),
        "zh-CN"
    );
    assert_eq!(
        crate::language_policy::preferred_response_language_hint(
            "do not run anything, just tell me a very short joke",
            None
        ),
        "en"
    );
    assert_eq!(
        crate::language_policy::preferred_response_language_hint("用 English 解释 README", None),
        "mixed"
    );
    assert_eq!(
        crate::language_policy::preferred_response_language_hint("12345", None),
        "config_default"
    );
}

#[test]
fn normalizer_chat_direct_answer_does_not_bypass_gate_for_unverified_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent: "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜"
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer supplied candidate".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract::default(),
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
            Some(&ctx),
        ),
        None
    );

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "写一首两句的打工人短诗\nanswer_candidate: 早出晚归血汗钱\n苦中作乐笑开颜",
            Some(&ctx),
        ),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_allows_distinctive_candidate_bound_in_memory_context() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "recall_scalar\nanswer_candidate: RC-CONT-CN-0428-A".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        memory_context_for_execution: Some(
            "### MEMORY_CONTEXT (NOT CURRENT REQUEST)\n\
#### STABLE_FACTS\n\
- Current consecutive test ID: RC-CONT-CN-0428-A"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "recall_scalar\nanswer_candidate: RC-CONT-CN-0428-A",
            Some(&ctx),
        )
        .as_deref(),
        Some("RC-CONT-CN-0428-A")
    );
}

#[test]
fn normalizer_chat_direct_answer_allows_bound_anchor_basename_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "known_file_basename\nanswer_candidate: ABCD.txt".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=known_file_basename\n\
answer_candidate: ABCD.txt\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Delivery\n\
followup_bound_target: /tmp/rustclaw/stem_unique/ABCD.txt\n\
followup_ordered_entries: 1:/tmp/rustclaw/stem_unique/ABCD.txt\n\
observed_bound_target: /tmp/rustclaw/stem_unique/ABCD.txt"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "known_file_basename\nanswer_candidate: ABCD.txt",
            Some(&ctx),
        )
        .as_deref(),
        Some("ABCD.txt")
    );
}

#[test]
fn normalizer_chat_direct_answer_reads_route_answer_candidate_when_merged_prompt_lacks_it() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "previous_ordered_entry\nanswer_candidate: orders".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=previous_ordered_entry\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: List\n\
followup_ordered_entries: 1:orders | 2:service_logs | 3:users"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "merged active task without candidate",
            Some(&ctx)
        )
        .as_deref(),
        Some("orders")
    );
}

#[test]
fn normalizer_chat_direct_answer_allows_active_observation_synthesis_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.resolved_intent =
        "classify the current observed excerpt\nanswer_candidate: It is a runtime log.".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=classify active excerpt\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Read\n\
followup_bound_target: /tmp/rustclaw/app.log\n\
observed_bound_target: /tmp/rustclaw/app.log"
                .to_string(),
        ),
        cross_turn_recent_execution_context: Some(
            "### RECENT_EXECUTION_EVENTS\n\
1 request=read target result=2026-05-30T00:00:00Z INFO clawd listening"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "classify the current observed excerpt\nanswer_candidate: It is a runtime log.",
            Some(&ctx),
        )
        .as_deref(),
        Some("It is a runtime log.")
    );
}

#[test]
fn normalizer_chat_direct_answer_does_not_self_ground_answer_candidate_from_context_summary() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "known_file_basename\nanswer_candidate: ABCD.txt".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        context_bundle_summary: Some(
            "route_view=false resolved_prompt=known_file_basename\n\
answer_candidate: ABCD.txt\n\n\
### ACTIVE_EXECUTION_ANCHOR\n\
followup_op_kind: Delivery"
                .to_string(),
        ),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "known_file_basename\nanswer_candidate: ABCD.txt",
            Some(&ctx),
        ),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_does_not_bypass_evidence_contract() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::direct_answer(),
        resolved_intent:
            "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "needs local evidence".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Medium,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            semantic_kind: crate::OutputSemanticKind::HiddenEntriesCheck,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            "检查当前目录是否有隐藏文件\nanswer_candidate: 有，例如 .git、.gitignore、.pids",
            Some(&ctx),
        ),
        None
    );
}

#[test]
fn normalizer_chat_direct_answer_uses_runtime_fact_candidate_without_budget_fallback() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
    let route = crate::RouteResult {
            ask_mode: crate::AskMode::direct_answer(),
            resolved_intent: format!(
                "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
            ),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer supplied runtime fact".to_string(),
            route_confidence: Some(1.0),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract::default(),
        };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
            normalizer_chat_direct_answer_candidate(
                &state,
                &format!(
                    "User request: output absolute path of current working directory\nanswer_candidate: {runtime_path}"
                ),
                Some(&ctx),
            )
            .as_deref(),
            Some(runtime_path.as_str())
        );
}

#[test]
fn normalizer_chat_direct_answer_uses_runtime_identity_candidate() {
    let Some(runtime_user) = ["USER", "LOGNAME", "USERNAME"]
        .into_iter()
        .filter_map(|key| std::env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
    else {
        return;
    };
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = format!("runtime_scalar\nanswer_candidate: {runtime_user}");
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_chat_direct_answer_candidate(
            &state,
            &format!("runtime_scalar\nanswer_candidate: {runtime_user}"),
            Some(&ctx),
        )
        .as_deref(),
        Some(runtime_user.as_str())
    );
}

#[test]
fn normalizer_runtime_fact_direct_answer_allows_scalar_clarify_guard_output() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: format!("runtime_scalar\nanswer_candidate: {runtime_path}"),
        needs_clarify: true,
        clarify_question: "Please provide the target path.".to_string(),
        route_reason: "background_locator_requires_clarify".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        normalizer_runtime_fact_direct_answer_candidate(
            &state,
            &format!("runtime_scalar\nanswer_candidate: {runtime_path}"),
            Some(&ctx),
        )
        .as_deref(),
        Some(runtime_path.as_str())
    );
}

#[test]
fn runtime_scalar_path_direct_answer_uses_verified_contract_locator() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let runtime_path = state.skill_rt.workspace_root.to_string_lossy().to_string();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Output the current workspace path".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "runtime scalar path".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: runtime_path.clone(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)).as_deref(),
        Some(runtime_path.as_str())
    );
}

#[test]
fn runtime_scalar_path_direct_answer_rejects_unverified_locator() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "Output the current workspace path".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "runtime scalar path".to_string(),
        route_confidence: Some(1.0),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            semantic_kind: crate::OutputSemanticKind::ScalarPathOnly,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            locator_hint: "/tmp/not-the-rustclaw-workspace".to_string(),
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };

    assert_eq!(
        runtime_scalar_path_direct_answer_candidate(&state, Some(&ctx)),
        None
    );
}

#[test]
fn preferred_route_clarify_question_respects_explicit_route_question_before_generic_fallback() {
    let mut route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "看看那个目录下面都有什么".to_string(),
        needs_clarify: true,
        clarify_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        route_reason: "fresh_deictic_missing_locator:directory_lookup".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Free,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Path,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route.clone()),
        ..Default::default()
    };
    assert_eq!(
        preferred_route_clarify_question(Some(&ctx)).as_deref(),
        Some("LOCATOR_CLARIFY_PROMPT")
    );

    route.clarify_question.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
    let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
    assert!(context.contains("clarify_case: missing_read_target"));
    assert!(context.contains("locator_kind: path"));
}

#[test]
fn fuzzy_locator_candidates_are_structured_context_not_hard_question() {
    let route = crate::RouteResult {
        ask_mode: crate::AskMode::clarify(),
        resolved_intent: "读取 Cargo.toml 的 package.name，只输出值".to_string(),
        needs_clarify: true,
        clarify_question: String::new(),
        route_reason: "llm_contract:generic_filename_scalar_extract".to_string(),
        route_confidence: Some(0.95),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Scalar,
            requires_content_evidence: true,
            locator_kind: crate::OutputLocatorKind::Filename,
            ..Default::default()
        },
    };
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        fuzzy_locator_suggestions: vec![
            "/tmp/a/Cargo.toml".to_string(),
            "/tmp/b/Cargo.toml".to_string(),
        ],
        ..Default::default()
    };
    assert_eq!(preferred_route_clarify_question(Some(&ctx)), None);
    let context = route_structured_clarify_context(Some(&ctx)).expect("structured context");
    assert!(context.contains("clarify_case: fuzzy_locator_candidates"));
    assert!(context.contains("candidate_1: /tmp/a/Cargo.toml"));
    assert!(context.contains("candidate_2: /tmp/b/Cargo.toml"));
}
