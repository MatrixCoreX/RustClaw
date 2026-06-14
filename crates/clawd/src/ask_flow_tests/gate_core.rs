#[test]
fn direct_answer_gate_schema_drift() {
    const SCHEMA_RAW: &str =
        include_str!("../../../../prompts/schemas/direct_answer_gate.schema.json");
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
        "state_patch",
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
fn direct_answer_gate_schema_preserves_quantity_comparison_state_patch() {
    let raw = serde_json::json!({
        "decision": "direct_answer",
        "reason": "observed_recent_count_comparison",
        "confidence": 0.95,
        "clarify_question": "",
        "resolved_user_intent": "Compare two recent count_inventory observations and return the selected target label.",
        "reference_resolution": {"target": "comparison_result"},
        "state_patch": {
            "quantity_comparison": {
                "source": "recent_count_inventory",
                "selection": "max",
                "candidates": [
                    {"label": "scripts", "count": 65},
                    {"label": "document", "count": 37}
                ],
                "winner": "scripts"
            }
        },
        "output_contract": {
            "response_shape": "scalar",
            "exact_sentence_count": null,
            "requires_content_evidence": false,
            "delivery_required": false,
            "locator_kind": "none",
            "delivery_intent": "none",
            "semantic_kind": "quantity_comparison",
            "locator_hint": "",
            "self_extension": {"mode": "none", "trigger": "none", "execute_now": false}
        }
    })
    .to_string();
    let validated = crate::prompt_utils::validate_against_schema::<DirectAnswerGateOut>(
        &raw,
        crate::prompt_utils::PromptSchemaId::DirectAnswerGate,
    )
    .expect("direct_answer_gate state_patch should validate");
    assert_eq!(
        validated
            .value
            .state_patch
            .as_ref()
            .and_then(|patch| patch.pointer("/quantity_comparison/selection"))
            .and_then(serde_json::Value::as_str),
        Some("max")
    );
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
fn active_task_factual_rewrite_review_prompt_requires_json_machine_output() {
    let prompt = active_task_factual_rewrite_review_prompt(
        "Current task:\nDraft from observed README.\n\nMost recent generated output:\nRuns locally from configured services.",
        "Runs locally and provides offline cloud privacy guarantees.",
    );

    assert!(prompt.contains("Active Task Factual Rewrite Review"));
    assert!(prompt.contains("\"unsupported_claims\""));
    assert!(prompt.contains("Return only JSON"));
}

#[test]
fn active_task_factual_rewrite_review_needs_repair_only_for_machine_claims() {
    let passed = ActiveTaskFactualRewriteReview {
        pass: true,
        unsupported_claims: vec!["new operational guarantee".to_string()],
    };
    assert!(!active_task_factual_rewrite_review_needs_repair(&passed));

    let failed_without_claims = ActiveTaskFactualRewriteReview {
        pass: false,
        unsupported_claims: vec![" ".to_string()],
    };
    assert!(!active_task_factual_rewrite_review_needs_repair(
        &failed_without_claims
    ));

    let failed_with_claim = ActiveTaskFactualRewriteReview {
        pass: false,
        unsupported_claims: vec!["new operational guarantee".to_string()],
    };
    assert!(active_task_factual_rewrite_review_needs_repair(
        &failed_with_claim
    ));
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

fn insert_wrapped_count_inventory_task(
    state: &crate::AppState,
    task_id: &str,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    path: &str,
    total: i64,
    updated_at: &str,
) {
    let payload = serde_json::json!({
        "action": "count_inventory",
        "counts": {"total": total},
        "path": path,
        "resolved_path": format!("/tmp/repo/{path}")
    });
    let output_excerpt = serde_json::json!({
        "extra": payload,
        "text": payload.to_string()
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
    .expect("insert wrapped count task");
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
        state_patch: None,
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
fn direct_answer_gate_ignores_current_workspace_semantic_none_promotion() {
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

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_background_only_ignored"));
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
fn direct_answer_gate_keeps_alias_bindings_patch_with_extra_fields_direct() {
    let mut route = chat_route_for_gate();
    route.resolved_intent = "Update a stored alias binding and acknowledge it.".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: None,
            target_task_policy: None,
            should_interrupt_active_run: false,
            state_patch: Some(serde_json::json!({
                "alias_bindings": [{
                    "alias": "甲文件",
                    "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
                }],
                "visible_constraints": {
                    "reply_shape": "ack_only"
                }
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
    assert!(!route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_memory_update_ignored"));
}

#[test]
fn runtime_approval_wait_status_defers_structured_status_to_language_path() {
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

    assert!(runtime_approval_wait_status_direct_answer_candidate(Some(&ctx), "en").is_none());
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

    let request = "Preview how images under ./document could be categorized. Do not move files.";
    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(route.resolved_intent, request);
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
fn direct_answer_gate_keeps_exact_answer_candidate_for_workspace_path_explanation() {
    let root = TempDirGuard::new("gate_exact_answer_workspace_path");
    std::fs::create_dir_all(root.path.join("logs")).expect("logs dir");
    std::fs::write(root.path.join("logs").join("clawd.log"), "startup\n").expect("log file");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "用一句话说明 logs/clawd.log 的用途\n",
        "answer_candidate: logs/clawd.log 记录 clawd 守护进程的运行日志，包括启动、错误和请求等关键信息。"
    )
    .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.exact_sentence_count = Some(1);
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut contract = gate_contract(false, "none", "none");
    contract.response_shape = "one_sentence".to_string();
    contract.exact_sentence_count = Some(1);
    let gate = gate_out("direct_answer", contract);

    let outcome = apply_direct_answer_gate_outcome(
        &state,
        &mut ctx,
        "不要执行命令，用一句话说明 logs/clawd.log 一般是干什么的。",
        gate,
    );

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(route.is_chat_gate());
    assert!(!route.output_contract.requires_content_evidence);
    assert!(route
        .route_reason
        .contains("direct_answer_gate_exact_candidate_ignored_execution"));
}

#[test]
fn direct_answer_gate_planner_execute_preserves_current_workspace_child_request() {
    let root = TempDirGuard::new("gate_workspace_child_context_planner");
    std::fs::create_dir_all(root.path.join("docs")).expect("docs dir");
    std::fs::write(root.path.join("docs").join("release.md"), "# Release\n").expect("release doc");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root.path.clone();
    state.skill_rt.default_locator_search_dir = root.path.clone();

    let mut route = chat_route_for_gate();
    route.resolved_intent = concat!(
        "list docs, then classify release.md\n",
        "answer_candidate: release.md is a checklist"
    )
    .to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let gate = gate_out("planner_execute", gate_contract(false, "none", "none"));

    let request = "List files under ./docs, then read release.md and classify it in one sentence.";
    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, request, gate);

    assert!(matches!(outcome, DirectAnswerPreflight::PlannerExecute(_)));
    let route = ctx.route_result.expect("route");
    assert!(route.is_execute_gate());
    assert_eq!(route.resolved_intent, request);
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert!(route.output_contract.locator_hint.ends_with("docs"));
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
fn direct_answer_gate_skips_standalone_freeform_repair_without_candidate() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.resolved_intent = "draft a plan".to_string();
    route.route_reason = "standalone_freeform_clarify_downgraded_to_direct_answer".to_string();

    assert!(!direct_answer_gate_can_skip_for_pure_chat_draft(
        &state,
        "draft a plan",
        Some(&route)
    ));
    assert!(direct_answer_gate_can_skip_for_standalone_freeform_repair(
        Some(&route)
    ));
}

#[test]
fn direct_answer_gate_clarify_is_ignored_for_standalone_freeform_repair() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let mut route = chat_route_for_gate();
    route.route_reason = "standalone_freeform_clarify_downgraded_to_direct_answer".to_string();
    let mut ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let mut gate = gate_out("clarify", gate_contract(false, "none", "none"));
    gate.clarify_question = "What plan topic should I use?".to_string();

    let outcome = apply_direct_answer_gate_outcome(&state, &mut ctx, "draft a plan", gate);

    assert!(matches!(outcome, DirectAnswerPreflight::DirectAnswer));
    let route = ctx.route_result.expect("route");
    assert!(!route.needs_clarify);
    assert_eq!(route.ask_mode, crate::AskMode::direct_answer());
    assert!(route
        .route_reason
        .contains("direct_answer_gate_standalone_freeform_clarify_ignored"));
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
