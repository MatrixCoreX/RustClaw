#[test]
fn observed_outputs_include_structured_run_cmd_error() {
    let err = format!(
        "__RC_SKILL_ERROR__:{}",
        serde_json::json!({
            "skill": "run_cmd",
            "error_kind": "nonzero_exit",
            "error_text": "Command failed with exit code 128",
            "platform": "linux",
            "extra": {
                "command": "git -C /tmp status",
                "exit_code": 128,
                "exit_category": "terminated_by_signal_or_shell_status",
                "stderr": "fatal: not a git repository",
                "output_truncated": false
            }
        })
    );
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(error_step("step_1", "run_cmd", &err));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(has_observed_answer_candidates(&loop_state));
    assert!(joined.contains("skill(run_cmd)"), "entries: {joined}");
    assert!(
        joined.contains("execution_status: error"),
        "entries: {joined}"
    );
    assert!(
        joined.contains("fatal: not a git repository"),
        "entries: {joined}"
    );
}

#[test]
fn observed_outputs_exclude_synthesis_steps() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"line 1"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "synthesize_answer",
        "stale synthesized answer",
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_3", "respond", "stale delivered answer"));
    loop_state.executed_step_results.push(ok_step(
        "step_4",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"line 2"}"#,
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(joined.contains("line 1"), "entries: {joined}");
    assert!(joined.contains("line 2"), "entries: {joined}");
    assert!(
        !joined.contains("stale synthesized answer"),
        "entries: {joined}"
    );
    assert!(
        !joined.contains("stale delivered answer"),
        "entries: {joined}"
    );
}

#[test]
fn market_quote_scalar_direct_answer_uses_registry_semantic_tag() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "market_probe"
        enabled = true
        kind = "runner"
        semantic_tags = ["market_quote_scalar"]
        "#,
        &["market_probe"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.resolved_intent = "capability_ref=crypto.quote symbol=BTC".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "market_probe",
        r#"{"quote":{"symbol":"BTC","price_usd":123.45}}"#,
    ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("BTC $123.45")
    );
}

#[test]
fn market_quote_scalar_direct_answer_does_not_use_skill_name_branch() {
    let state = test_state_with_registry(
        r#"
        [[skills]]
        name = "crypto"
        enabled = true
        kind = "runner"
        semantic_tags = []
        "#,
        &["crypto"],
    );
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::MarketQuote;
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "crypto",
        r#"{"quote":{"symbol":"BTC","price_usd":123.45}}"#,
    ));

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &state,
            Some(&agent_run_context)
        ),
        None
    );
}

#[test]
fn multi_count_quantity_comparison_guard_lists_all_count_rows() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"count_inventory","path":"crates","resolved_path":"/repo/crates","recursive":false,"counts":{"total":13,"files":0,"dirs":13,"hidden":0}}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"count_inventory","path":"crates/skills","resolved_path":"/repo/crates/skills","recursive":false,"counts":{"total":35,"files":0,"dirs":35,"hidden":0}}"#,
    ));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route.output_contract.semantic_kind = OutputSemanticKind::QuantityComparison;

    let guard = multi_count_quantity_comparison_guard_entry(&loop_state, Some(&route))
        .expect("multi-count guard");

    assert!(
        guard.contains("delivery_constraint=cover_all_observed_count_rows"),
        "guard: {guard}"
    );
    assert!(guard.contains("observed_count_rows=2"), "guard: {guard}");
    assert!(
        guard.contains("observed_count.1.path=/repo/crates"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.1.count_total=13"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.2.path=/repo/crates/skills"),
        "guard: {guard}"
    );
    assert!(
        guard.contains("observed_count.2.count_total=35"),
        "guard: {guard}"
    );
}

#[test]
fn compound_listing_content_delivery_guard_lists_observed_names() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names":["archive","release_checklist.md","service_notes.md"]}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","excerpt":"1|# Release Checklist\n3|1. Verify configuration loads correctly."}"#,
    ));
    let route = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);

    let guard = compound_listing_content_delivery_guard_entry(&loop_state, Some(&route))
        .expect("compound guard");

    assert!(guard.contains("current_task_observed_listing_names"));
    assert!(guard.contains("archive, release_checklist.md, service_notes.md"));
    assert!(guard.contains("current_task_observed_content_excerpt: present"));
}

#[test]
fn names_only_inventory_direct_answer_does_not_need_llm_synthesis() {
    let state = AppState::test_default_with_fixture_provider();
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                steps: vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_capability".to_string(),
                        skill: "filesystem.list_names".to_string(),
                        args: serde_json::json!({
                            "path": "document",
                            "names_only": true,
                            "max_entries": 5,
                            "sort_by": "name",
                        }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "synthesize_answer".to_string(),
                        skill: String::new(),
                        args: serde_json::json!({}),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ],
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names_only":true,"names":["full_suite_trace_note.txt","gen-1778122040.png","gen-1778122536.png","hello.sh","hello_from_manual_test.sh"],"names_by_kind":{"files":["full_suite_trace_note.txt","gen-1778122040.png","gen-1778122536.png","hello.sh","hello_from_manual_test.sh"],"dirs":[],"other":[]},"path":"document","resolved_path":"/workspace/document"}"#,
    ));

    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(&loop_state, &state, Some(&agent_run_context))
            .as_deref(),
        Some(
            "full_suite_trace_note.txt\ngen-1778122040.png\ngen-1778122536.png\nhello.sh\nhello_from_manual_test.sh"
        )
    );
}

#[test]
fn names_only_inventory_free_shape_defers_to_llm_synthesis() {
    let state = AppState::test_default_with_fixture_provider();
    let route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state
        .round_traces
        .push(crate::task_journal::TaskJournalRoundTrace {
            round_no: 1,
            goal: String::new(),
            execution_recipe_summary: None,
            plan_result: Some(crate::PlanResult {
                goal: String::new(),
                missing_slots: Vec::new(),
                needs_confirmation: false,
                steps: vec![
                    crate::PlanStep {
                        step_id: "step_1".to_string(),
                        action_type: "call_capability".to_string(),
                        skill: "filesystem.list_names".to_string(),
                        args: serde_json::json!({
                            "path": "logs",
                            "names_only": true,
                            "max_entries": 2,
                        }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    crate::PlanStep {
                        step_id: "step_2".to_string(),
                        action_type: "synthesize_answer".to_string(),
                        skill: String::new(),
                        args: serde_json::json!({}),
                        depends_on: vec!["step_1".to_string()],
                        why: String::new(),
                    },
                ],
                planner_notes: String::new(),
                plan_kind: crate::PlanKind::Single,
                raw_plan_text: String::new(),
            }),
            verify_result: None,
        });
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"inventory_dir","names_only":true,"names":["clawd.run.log","model_io.log"],"path":"logs","resolved_path":"/workspace/logs"}"#,
    ));

    assert!(
        extract_direct_answer_from_generic_output_i18n(&loop_state, &state, Some(&agent_run_context))
            .is_none()
    );
}

#[test]
fn observed_outputs_keep_latest_content_read_for_same_path() {
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"action":"read_range","resolved_path":"/tmp/model_io.log","excerpt":"old head evidence"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "fs_basic",
        r#"{"action":"read_range","resolved_path":"/tmp/model_io.log","excerpt":"new tail evidence"}"#,
    ));

    let entries = observed_output_entries(&loop_state);
    let joined = entries.join("\n");

    assert!(!joined.contains("old head evidence"), "entries: {joined}");
    assert!(joined.contains("new tail evidence"), "entries: {joined}");
}

fn chat_wrapped_unclassified_route(response_shape: OutputResponseShape) -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "Run an observation, then produce the requested final wording."
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::None,
            locator_hint: "/workspace/project".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn execution_failed_step_guard_prefers_failed_machine_fields_over_success_stdout() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let ctx = AgentRunContext {
        route_result: Some(route.clone()),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(3);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "RC_RENDER_KO_OK\n"));
    loop_state.executed_step_results.push(error_step(
        "step_2",
        "run_cmd",
        &crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "nonzero_exit",
            "Command failed with exit code 127",
            Some("linux"),
            Some(serde_json::json!({
                "command": "definitely_missing_command_rustclaw_render_ko_0605",
                "exit_category": "command_not_found",
                "exit_classification_source": "exit_code",
                "exit_code": 127,
                "stderr": "bash: line 1: definitely_missing_command_rustclaw_render_ko_0605: command not found\n",
                "stdout": serde_json::Value::Null,
            })),
        ),
    ));
    loop_state.executed_step_results.push(error_step(
        "step_4",
        "run_cmd",
        &crate::skills::structured_skill_error_from_parts(
            "run_cmd",
            "nonzero_exit",
            "Command failed with exit code 127",
            Some("linux"),
            Some(serde_json::json!({
                "command": "definitely_missing_command_rustclaw_render_ko_0605",
                "exit_category": "command_not_found",
                "exit_classification_source": "exit_code",
                "exit_code": 127,
                "stderr": "bash: line 1: definitely_missing_command_rustclaw_render_ko_0605: command not found\n",
                "stdout": serde_json::Value::Null,
            })),
        ),
    ));

    let guard = execution_failed_step_guard_entry(&loop_state, ctx.route_result.as_ref()).unwrap();

    assert!(route_disallows_direct_observation_passthrough(&route));
    assert!(guard.contains("final_answer_shape=failed_step_with_evidence"));
    assert!(guard.contains("successful_step_outputs_are_not_final_answer=true"));
    assert!(guard.contains("success_step.1.output_is_not_answer=RC_RENDER_KO_OK"));
    assert!(guard.contains("failed_step.1.step_id=step_2"));
    assert!(guard.contains("failed_step.1.skill=run_cmd"));
    assert!(
        guard.contains("failed_step.1.command=definitely_missing_command_rustclaw_render_ko_0605"),
        "guard: {guard}"
    );
    assert!(guard.contains("failed_step.1.exit_category=command_not_found"));
    assert!(guard.contains("failed_step.1.exit_code=127"));
    assert!(!guard.contains("step_4"), "guard: {guard}");
}

#[test]
fn execution_failed_step_guard_skips_contract_policy_gap_errors() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route.output_contract.semantic_kind = OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    let ctx = AgentRunContext {
        route_result: Some(route),
        ..AgentRunContext::default()
    };
    let mut loop_state = LoopState::new(3);
    loop_state.executed_step_results.push(error_step(
        "step_1",
        "make_dir",
        r#"__RC_SKILL_ERROR__:{"error_kind":"contract_action_rejected","error_text":"planned tool step was not allowed for this request","extra":{"failure_attribution":"contract_gap"},"skill":"make_dir"}"#,
    ));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "run_cmd", "note.txt alpha beta removed\n"));

    let guard = execution_failed_step_guard_entry(&loop_state, ctx.route_result.as_ref());

    assert!(
        guard.is_none(),
        "contract policy gaps are loop recovery signals, not final failed-step evidence: {guard:?}"
    );
}

#[test]
fn scalar_path_observed_route_rejects_content_evidence_contract() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    route.output_contract.requires_content_evidence = true;

    assert!(route_requests_scalar_path_only(&route));
    assert!(!route_allows_path_batch_scalar_path_observed_answer(&route));

    route.output_contract.requires_content_evidence = false;
    assert!(route_allows_path_batch_scalar_path_observed_answer(&route));
}

#[test]
fn observed_output_route_policy_accepts_contract_markers_without_semantic_enum() {
    let mut scalar_path_route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    scalar_path_route.route_reason = "contract:scalar_path_only".to_string();
    scalar_path_route.output_contract.requires_content_evidence = false;
    assert_eq!(
        scalar_path_route.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    assert!(route_requests_scalar_path_only(&scalar_path_route));
    assert!(route_allows_path_batch_scalar_path_observed_answer(
        &scalar_path_route
    ));

    scalar_path_route.route_reason =
        "contract:scalar_path_only; execution_required_read_file_extract_scalar".to_string();
    assert!(!route_allows_path_batch_scalar_path_observed_answer(
        &scalar_path_route
    ));

    let mut file_names_route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    file_names_route.route_reason = "contract:file_names".to_string();
    assert!(route_prefers_plain_fs_search_paths(&file_names_route));
    assert!(route_allows_raw_listing_direct_answer(Some(
        &file_names_route
    )));

    let mut failed_step_route = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    failed_step_route.route_reason = "contract:execution_failed_step".to_string();
    failed_step_route.output_contract.locator_kind = OutputLocatorKind::None;
    failed_step_route.output_contract.locator_hint.clear();
    assert!(route_disallows_direct_observation_passthrough(
        &failed_step_route
    ));

    let mut quantity_route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    quantity_route.route_reason = "contract:quantity_comparison".to_string();
    quantity_route.output_contract.requires_content_evidence = true;
    assert!(route_quantity_comparison_requires_model_language_synthesis(
        &quantity_route
    ));
}

#[test]
fn scalar_count_answer_detects_non_numeric_diagnostic_line() {
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "configs/config_copy".to_string();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "0\n\nfind: /workspace/configs/config_copy: No such file or directory\n",
    ));

    let diagnostic = scalar_count_diagnostic_line_for_answer("0", Some(&route), &loop_state);

    assert_eq!(
        diagnostic.as_deref(),
        Some("find: /workspace/configs/config_copy: No such file or directory")
    );
    let answer = scalar_count_diagnostic_machine_answer(diagnostic.as_deref().unwrap());
    assert!(answer.contains("message_key=clawd.msg.scalar_count.unreliable"));
    assert!(answer.contains("reason_code=count_unreliable_diagnostic"));
    assert!(answer.contains("final_answer_shape=scalar_count_unavailable"));
    assert!(answer.contains(
        "diagnostic=find: /workspace/configs/config_copy: No such file or directory"
    ));
}

fn reuse_active_context(user_request: &str) -> AgentRunContext {
    AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskAppend),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        user_request: Some(user_request.to_string()),
        ..Default::default()
    }
}

#[test]
fn recent_generated_output_extracts_internal_merge_block() {
    let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope\n\nNew user instruction:\ncount only";

    assert_eq!(
        recent_generated_output_from_user_request(merged).as_deref(),
        Some("archive\nrelease_checklist.md\nservice_notes.md")
    );
}

#[test]
fn cross_turn_observed_entries_require_reuse_active_context() {
    let merged = "Current task:\nlook at that docs dir\n\nMost recent generated output:\narchive\nrelease_checklist.md\nservice_notes.md\n\nContinuity rules:\n- keep scope";
    let loop_state = LoopState::new(1);
    let allowed = reuse_active_context(merged);

    let entries = cross_turn_observed_output_entries(&loop_state, Some(&allowed));
    assert_eq!(entries.len(), 1);
    assert!(entries[0].contains("prior_turn_observed_output"));
    assert!(entries[0].contains("archive"));
    assert!(!entries[0].contains("Continuity rules"));

    let standalone = AgentRunContext {
        turn_analysis: Some(crate::intent_router::TurnAnalysis {
            turn_type: Some(crate::intent_router::TurnType::TaskRequest),
            target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: false,
        }),
        user_request: Some(merged.to_string()),
        ..Default::default()
    };
    assert!(cross_turn_observed_output_entries(&loop_state, Some(&standalone)).is_empty());
}

#[test]
fn direct_scalar_ignores_exit_zero_prefix() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "git_basic", "exit=0\nmain\n"));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("main")
    );
}

#[test]
fn direct_scalar_extracts_system_basic_runtime_status_value() {
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route_result.output_contract.locator_kind = OutputLocatorKind::None;
    route_result.output_contract.locator_hint.clear();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..Default::default()
    };
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "system_basic",
        r#"{"action":"runtime_status","kind":"current_user","value":"guagua","field_value":"guagua","command_output":"guagua"}"#,
    ));

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("guagua")
    );
}

#[test]
fn direct_scalar_defers_git_oneline_log_record_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n09342a6a fix: expose nl execution and locator flows\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "查看当前工作区最近一次 git 提交的标题，并简短告诉我。".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "Self-contained workspace inspection request for git commit title."
            .to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
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
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: ".".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn observed_entries_include_structured_extract_field_outputs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"react-example","value":"react-example","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"package.name","value_text":"clawd","value":"clawd","value_type":"string"}"#,
        ));

    let entries = observed_output_entries(&loop_state);
    assert_eq!(entries.len(), 2);
    assert!(entries[0].contains("name: react-example"));
    assert!(entries[1].contains("package.name: clawd"));
}

#[test]
fn direct_scalar_ignores_shell_locale_warning_noise() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "run_cmd",
            "/tmp/rustclaw-workspace\n\nbash: warning: setlocale: LC_ALL: cannot change locale (C.UTF-8): No such file or directory\n",
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("/tmp/rustclaw-workspace")
    );
}

#[test]
fn direct_scalar_reads_extract_field_value_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"name","value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("rustclaw")
    );
}

#[test]
fn direct_scalar_reads_read_field_value_from_structured_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"package.name","value_text":"react-example","value":"react-example","value_type":"string"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("react-example")
    );
}

#[test]
fn direct_scalar_defers_container_read_field_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\"}","value_type":"object"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None),
        None
    );
}

#[test]
fn direct_scalar_returns_container_read_field_json_for_scalar_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","exists":true,"field_path":"package.version","value":{"workspace":true},"value_text":"{\"workspace\":true}","value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "Cargo.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some(r#"{"workspace":true}"#)
    );
}

#[test]
fn direct_scalar_preserves_resolved_extract_field_label_for_non_exact_match() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"model.vendor","resolved_field_path":"llm.selected_vendor","match_strategy":"missing_parent_leaf_key_suffix","value_text":"minimax","value":"minimax","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("llm.selected_vendor: minimax")
    );
}

#[test]
fn direct_scalar_reads_array_identity_field_value_without_label() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"archive_basic.group","resolved_field_path":"skills[name=archive_basic].group","match_strategy":"array_item_key_path","value_text":"system","value":"system","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system")
    );
}

#[test]
fn direct_answer_reads_array_identity_extract_field_value_without_label() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"skills.[name=archive_basic].group","resolved_field_path":"skills.[name=archive_basic].group","match_strategy":"exact_path","value_text":"system","value":"system","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("system")
    );
}

#[test]
fn direct_answer_reads_config_basic_extract_field_value() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"run_cmd.planner_kind","value_text":"tool","value":"tool","value_type":"string"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/skills_registry.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("run_cmd.planner_kind: tool")
    );
    assert!(has_observed_answer_candidates(&loop_state));
}

#[test]
fn direct_answer_reads_config_basic_read_fields_values() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_fields","path":"package.json","resolved_path":"/tmp/package.json","count":2,"results":[{"field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"},{"field_path":"version","exists":true,"value_type":"string","value_text":"1.0.0","value":"1.0.0"}]}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "package.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("name: react-example\nversion: 1.0.0")
    );
}

#[test]
fn direct_scalar_reads_structured_keys_value_list() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","exists":true,"container_type":"object","count":3,"keys":["app","features","paths"],"field_path":""}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("app\nfeatures\npaths")
    );
}

#[test]
fn direct_answer_does_not_treat_root_level_as_missing_key() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"structured_keys","exists":true,"container_type":"object","count":3,"keys":["app","features","paths"],"field_path":""}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.resolved_intent = "List root-level keys in app_config.toml only".to_string();
    route_result.output_contract.semantic_kind = OutputSemanticKind::StructuredKeys;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "app_config.toml".to_string();
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "List root-level keys in scripts/nl_tests/fixtures/device_local/configs/app_config.toml only."
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("app\nfeatures\npaths")
    );
}

#[test]
fn direct_answer_defers_container_extract_field_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"scripts","value":{"build":"echo build","dev":"echo dev","lint":"echo lint"},"value_text":"{\"build\":\"echo build\",\"dev\":\"echo dev\",\"lint\":\"echo lint\"}","value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "package.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_schema_enum_extract_field_with_resolved_path() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","exists":true,"field_path":"target","resolved_field_path":"properties.reference_resolution.properties.target","match_strategy":"unique_bare_key","value":{"type":"string","enum":["none","current_action_result","current_turn_locator"]},"value_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Strict);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "prompts/schemas/agent_loop_decision_envelope.schema.json".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("schema enum should be formatted without synthesis");

    assert!(answer.contains("properties.reference_resolution.properties.target"));
    assert!(answer.contains("`none`"));
    assert!(answer.contains("`current_turn_locator`"));
}

#[test]
fn direct_answer_formats_config_basic_validate_result_as_pass_fail() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","path":"configs/config.toml","resolved_path":"/tmp/configs/config.toml","format":"toml","valid":true,"root_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::OneSentence);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some(
            "Validate configs/config.toml and answer pass or fail.".to_string(),
        ),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("message_key=clawd.msg.validate_structured_pass\nreason_code=validate_structured_pass\nfinal_answer_shape=structured_validation\nvalid=true\nformat=toml")
    );
}

#[test]
fn direct_scalar_formats_config_validation_result_in_request_language() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"validate_structured","path":"configs/config.toml","resolved_path":"/tmp/configs/config.toml","format":"toml","valid":true,"root_type":"object"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.semantic_kind = OutputSemanticKind::ConfigValidation;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = "configs/config.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some("只检查 configs/config.toml 是否是合法 TOML。".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("message_key=clawd.msg.validate_structured_pass\nreason_code=validate_structured_pass\nfinal_answer_shape=structured_validation\nvalid=true\nformat=toml")
    );
}

#[test]
fn direct_scalar_defers_recent_structured_scalar_comparison_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_fields","path":"UI/package.json","resolved_path":"/tmp/UI/package.json","count":1,"results":[{"field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"}]}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","path":"crates/clawd/Cargo.toml","resolved_path":"/tmp/crates/clawd/Cargo.toml","field_path":"package.name","exists":true,"value_type":"string","value_text":"clawd","value":"clawd"}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::act_plain(),
            resolved_intent:
                "UI/package.json 里的 name 和 crates/clawd/Cargo.toml 里的 package.name 一样吗？只回答一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:compare_targets".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::QuantityComparison,
                locator_hint: "UI/package.json|crates/clawd/Cargo.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_scalar_defers_multiple_structured_scalars_without_semantic_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"read_field","path":"UI/package.json","resolved_path":"/tmp/UI/package.json","field_path":"name","resolved_field_path":"name","exists":true,"value_type":"string","value_text":"react-example","value":"react-example"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "config_basic",
            r#"{"action":"read_field","path":"crates/clawd/Cargo.toml","resolved_path":"/tmp/crates/clawd/Cargo.toml","field_path":"package.name","resolved_field_path":"package.name","exists":true,"value_type":"string","value_text":"clawd","value":"clawd"}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "UI/package.json|crates/clawd/Cargo.toml".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some(
            "Read two structured fields, then provide one final line.".to_string(),
        ),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
    assert!(extract_direct_scalar_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context)
    )
    .is_none());
}

#[test]
fn direct_scalar_formats_recent_structured_scalar_equality() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","field_path":"name","exists":true,"value_text":"RustClaw","value":"RustClaw","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","field_path":"crate_name","exists":true,"value_text":"rustclaw","value":"rustclaw","value_type":"string"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Are those two names the same? Answer same or different".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:same_or_different".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("RustClaw != rustclaw")
    );
}

#[test]
fn direct_scalar_formats_compare_paths_equality_with_explicit_existence_fields() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"compare_paths","comparison":{"same_path":false,"same_size":false,"size_delta_bytes":119},"field_value":{"left_exists":true,"right_exists":true,"same_path":false,"same_size":false,"size_delta_bytes":119},"left":{"exists":true,"path":"service_notes.md"},"right":{"exists":true,"path":"release_checklist.md"}},"text":"{}"}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Compare two paths and return same_path plus existence fields."
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:path_metadata_compare".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint: "service_notes.md | release_checklist.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context))
            .as_deref(),
        Some("same_path=false\nleft_exists=true\nright_exists=true")
    );
}

#[test]
fn direct_scalar_equality_ignores_duplicate_structured_source() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","path":"/tmp/Cargo.toml","resolved_path":"/tmp/Cargo.toml","field_path":"workspace.package.version","resolved_field_path":"workspace.package.version","exists":true,"value_text":"0.1.7","value":"0.1.7","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "config_basic",
            r#"{"action":"extract_field","path":"/tmp/Cargo.toml","resolved_path":"/tmp/Cargo.toml","field_path":"workspace.package.version","resolved_field_path":"workspace.package.version","exists":true,"value_text":"0.1.7","value":"0.1.7","value_type":"string"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Compare the Cargo.toml version with the version mentioned in README.md."
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:compare_targets".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint: "/tmp/Cargo.toml | /tmp/README.md".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_answer_formats_recent_structured_scalar_equality_for_strict_route() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "config_basic",
            r#"{"action":"extract_field","field_path":"name","exists":true,"value_text":"rustclaw-nl-fixture","value":"rustclaw-nl-fixture","value_type":"string"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "config_basic",
            r#"{"action":"extract_field","field_path":"package.name","exists":true,"value_text":"clawd","value":"clawd","value_type":"string"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "Read two names and answer in one line with whether they are the same or different."
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:same_or_different".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint:
                "scripts/nl_tests/fixtures/device_local/package.json and crates/clawd/Cargo.toml"
                    .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some("Read the name from scripts/nl_tests/fixtures/device_local/package.json. Read package.name from crates/clawd/Cargo.toml. Then answer in one line with the two names and whether they are the same or different.".to_string()),
            ..AgentRunContext::default()
        };
    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("rustclaw-nl-fixture != clawd")
    );
}

#[test]
fn direct_answer_formats_wrapped_config_basic_structured_scalar_equality() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "config_basic",
        &serde_json::json!({
            "extra": {
                "action": "extract_field",
                "path": "/repo/UI/package.json",
                "resolved_path": "/repo/UI/package.json",
                "field_path": "name",
                "resolved_field_path": "name",
                "exists": true,
                "value_text": "react-example",
                "value": "react-example",
                "value_type": "string"
            },
            "text": "{\"action\":\"extract_field\",\"exists\":true,\"field_path\":\"name\",\"value_text\":\"react-example\",\"value\":\"react-example\",\"value_type\":\"string\"}"
        })
        .to_string(),
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "config_basic",
        &serde_json::json!({
            "extra": {
                "action": "extract_field",
                "path": "/repo/crates/clawd/Cargo.toml",
                "resolved_path": "/repo/crates/clawd/Cargo.toml",
                "field_path": "package.name",
                "resolved_field_path": "package.name",
                "exists": true,
                "value_text": "clawd",
                "value": "clawd",
                "value_type": "string"
            },
            "text": "{\"action\":\"extract_field\",\"exists\":true,\"field_path\":\"package.name\",\"value_text\":\"clawd\",\"value\":\"clawd\",\"value_type\":\"string\"}"
        })
        .to_string(),
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Compare two structured field values.".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:same_or_different".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
            locator_hint: "UI/package.json | crates/clawd/Cargo.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        )
        .as_deref(),
        Some("react-example != clawd")
    );
}

#[test]
fn structured_pair_answer_does_not_infer_fields_from_read_file_outputs() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        r#"{"name":"react-example","version":"0.0.0"}"#,
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "read_file",
        r#"[package]
name = "clawd"
version.workspace = true
"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::act_plain(),
            resolved_intent:
                "读取 UI/package.json 里的 name 字段，再读取 crates/clawd/Cargo.toml 里的 package.name 字段，最后用一行输出：前者、后者、一样或不一样"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:same_or_different".to_string(),
            route_confidence: None,
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                exact_sentence_count: None,
                response_shape: OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::RecentScalarEqualityCheck,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let _agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        super::recent_structured_scalar_observation_count(&loop_state),
        0
    );
}

#[test]
fn direct_scalar_reports_missing_extract_field_as_readable_message() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, None).as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=name")
    );
}

#[test]
fn internal_missing_sentinel_uses_structured_extract_field_evidence() {
    let state = AppState::test_default_with_fixture_provider();
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"package.name","value_text":"","value":null,"value_type":"null"}"#,
        ));

    assert_eq!(
        replace_internal_missing_sentinel_with_structured_observation(
            "<missing>",
            &state,
            &loop_state,
            None
        )
        .as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=package.name")
    );
    assert_eq!(
        replace_internal_missing_sentinel_with_structured_observation(
            "package.name: <missing>",
            &state,
            &loop_state,
            None
        )
        .as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=package.name")
    );
}

#[test]
fn direct_scalar_missing_field_language_uses_original_request_before_resolved_prompt() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"extract_field","exists":false,"field_path":"name","value_text":"","value":null,"value_type":"null"}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Read the name field from package.json and output only its value."
            .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:field_extract".to_string(),
        route_confidence: None,
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: "package.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        original_user_request: Some("读取 package.json 里的 name 字段，只输出值".to_string()),
        user_request: Some(
            "Read the name field from package.json and output only its value.".to_string(),
        ),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context),
        )
        .as_deref(),
        Some("message_key=clawd.msg.extract_field_missing\nreason_code=extract_field_missing\nfinal_answer_shape=missing_structured_field\nexists=false\nfield_path=name")
    );
}
