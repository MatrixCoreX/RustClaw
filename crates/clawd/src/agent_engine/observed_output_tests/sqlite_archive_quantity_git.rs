#[test]
fn sqlite_database_kind_judgment_is_not_hard_classified_by_observed_output() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[]}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent:
            "看看 data/db-basic-contract.sqlite 里有哪些表，再一句话说这更像业务库还是测试库"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:planner_execute_with_chat_finalizer".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
            locator_hint: "data/db-basic-contract.sqlite".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn sqlite_empty_table_listing_returns_machine_fields() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[]}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "List tables in data/empty.sqlite".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:sqlite_table_listing".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::SqliteTableListing,
            locator_hint: "data/empty.sqlite".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("expected deterministic empty sqlite table listing answer");
    assert!(answer.contains("message_key=clawd.msg.sqlite.tables.observed"), "{answer}");
    assert!(answer.contains("reason_code=sqlite_tables_observed"), "{answer}");
    assert!(answer.contains("table_count=0"), "{answer}");
    assert!(answer.contains("has_tables=false"), "{answer}");
    assert!(answer.contains("db_kind=empty"), "{answer}");
    assert!(answer.contains("db_path=data/empty.sqlite"), "{answer}");
    assert!(!answer.contains("没有任何表"), "{answer}");
    assert!(!answer.contains("no tables"), "{answer}");
}

#[test]
fn sqlite_database_kind_judgment_uses_contract_selector_and_cites_tables() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::act_with_chat_finalizer(),
            resolved_intent:
                "判断 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 更像业务库还是测试库，并给出依据"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:sqlite_database_kind_judgment".to_string(),
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
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "判断这个 SQLite 更像业务库还是测试库。\n[CONTRACT_TEST_HINT]\nselector_database_kind=test\n[/CONTRACT_TEST_HINT]"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("expected deterministic sqlite database kind answer");
    assert!(answer.contains("message_key=clawd.msg.sqlite.database_kind.observed"), "{answer}");
    assert!(answer.contains("reason_code=sqlite_database_kind_observed"), "{answer}");
    assert!(answer.contains("db_kind=test"), "{answer}");
    assert!(answer.contains("classification_source=contract_selector"), "{answer}");
    assert!(answer.contains("table.1=orders"), "{answer}");
    assert!(answer.contains("table.2=service_logs"), "{answer}");
    assert!(answer.contains("table.3=users"), "{answer}");
    assert!(!answer.contains("第 1 步"), "{answer}");
}

#[test]
fn sqlite_database_kind_judgment_uses_run_cmd_table_names_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "orders\nservice_logs\nusers\n",
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::act_with_chat_finalizer(),
            resolved_intent:
                "判断 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 更像业务库还是测试库，并给出依据"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:sqlite_database_kind_judgment".to_string(),
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
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "判断这个 SQLite 更像业务库还是测试库。\n[CONTRACT_TEST_HINT]\nselector_database_kind=test\n[/CONTRACT_TEST_HINT]"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("expected deterministic run_cmd sqlite database kind answer");
    assert!(answer.contains("message_key=clawd.msg.sqlite.database_kind.observed"), "{answer}");
    assert!(answer.contains("db_kind=test"), "{answer}");
    assert!(answer.contains("classification_source=contract_selector"), "{answer}");
    assert!(answer.contains("table.1=orders"), "{answer}");
    assert!(answer.contains("table.2=service_logs"), "{answer}");
    assert!(answer.contains("table.3=users"), "{answer}");
}

#[test]
fn sqlite_schema_version_uses_run_cmd_value_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "run_cmd", "schema_version=7\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent:
            "读取 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 的 schema 版本"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:sqlite_schema_version".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::SqliteSchemaVersion,
            locator_hint: "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
                .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("schema_version=7")
    );
}

#[test]
fn sqlite_table_listing_uses_run_cmd_table_names_without_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "run_cmd",
        "orders\nservice_logs\nusers\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent:
            "列出 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里的表"
                .to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "llm_contract:sqlite_table_listing".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::SqliteTableListing,
            locator_hint: "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
                .to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("| name |\n| --- |\n| orders |\n| service_logs |\n| users |")
    );
}

#[test]
fn sqlite_database_kind_judgment_prefers_table_inventory_over_later_name_columns() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "db_basic",
            r#"{"columns":["id","name","email"],"rows":[{"email":"alice@example.com","id":1,"name":"Alice"},{"email":"bob@example.com","id":2,"name":"Bob"}]}"#,
        ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::act_with_chat_finalizer(),
            resolved_intent:
                "判断 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 更像业务库还是测试库，并给出依据"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "llm_contract:sqlite_database_kind_judgment".to_string(),
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
                response_shape: OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::SqliteDatabaseKindJudgment,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "判断这个 SQLite 更像业务库还是测试库。\n[CONTRACT_TEST_HINT]\nselector_database_kind=test\n[/CONTRACT_TEST_HINT]"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };
    let answer = extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context))
        .expect("expected deterministic sqlite database kind answer");
    assert!(answer.contains("message_key=clawd.msg.sqlite.database_kind.observed"), "{answer}");
    assert!(answer.contains("db_kind=test"), "{answer}");
    assert!(answer.contains("table.1=orders"), "{answer}");
    assert!(answer.contains("table.2=service_logs"), "{answer}");
    assert!(answer.contains("table.3=users"), "{answer}");
    assert!(!answer.contains("Alice"), "{answer}");
    assert!(!answer.contains("Bob"), "{answer}");
}

#[test]
fn direct_answer_lists_sqlite_table_names_without_llm_when_names_only_is_requested() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::act_with_chat_finalizer(),
            resolved_intent:
                "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:planner_execute_with_chat_finalizer".to_string(),
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
                semantic_kind: crate::OutputSemanticKind::SqliteTableNamesOnly,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("orders\nusers")
    );
}

#[test]
fn direct_scalar_lists_sqlite_table_names_when_names_only_contract_is_scalar() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
            ask_mode: crate::AskMode::act_plain(),
            resolved_intent:
                "看一下 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 里有哪些表，只输出表名"
                    .to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: "normalizer:act".to_string(),
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
                semantic_kind: crate::OutputSemanticKind::SqliteTableNamesOnly,
                locator_hint:
                    "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("orders\nusers")
    );
}

#[test]
fn direct_scalar_lists_sqlite_table_names_from_route_marker_without_semantic_enum() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.route_reason = "contract:sqlite_table_names_only".to_string();
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    assert_eq!(
        route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("orders\nusers")
    );
}

#[test]
fn direct_scalar_does_not_take_first_db_row_from_multi_row_query() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "Read a scalar value from the SQLite database".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:act".to_string(),
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
            locator_hint: "data/app.sqlite".to_string(),
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
fn direct_scalar_counts_db_rows_for_scalar_count_contract() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "db_basic",
            r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"service_logs"},{"name":"users"}]}"#,
        ));
    let mut route_result = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route_result.resolved_intent =
            "统计 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 的表数量，只输出数字"
                .to_string();
    route_result.output_contract.requires_content_evidence = true;
    route_result.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    route_result.output_contract.locator_kind = OutputLocatorKind::Path;
    route_result.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string();
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("3")
    );
}

#[test]
fn structured_observed_body_preserves_db_table_inventory_instead_of_first_scalar_only() {
    let body = r#"{"columns":["name"],"rows":[{"name":"users"},{"name":"orders"},{"name":"service_logs"}]}"#;
    assert_eq!(
        structured_observed_body("db_basic", body).as_deref(),
        Some("db_tables=users, orders, service_logs")
    );
}

#[test]
fn archive_list_summary_parses_raw_zip_table_for_synthesis() {
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    let summary = archive_list_summary_from_body(body).expect("zip listing should parse");
    assert_eq!(summary.archive.as_deref(), Some("/tmp/test_bundle.zip"));
    assert_eq!(summary.entries.len(), 2);
    assert_eq!(summary.entries[0].name, "notes.txt");
    assert_eq!(summary.entries[0].size_bytes, Some(22));
    assert_eq!(
            structured_observed_body("archive_basic", body).as_deref(),
            Some(
                "archive_basic action=list archive=/tmp/test_bundle.zip total_entries=2\nentry name=notes.txt size_bytes=22\nentry name=nested/config.ini size_bytes=20"
            )
        );
}

#[test]
fn archive_list_observed_fact_survives_artifact_filter() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));

    assert!(
        has_observed_answer_candidates(&loop_state),
        "normalized archive list facts should remain available for synthesis"
    );
}

#[test]
fn archive_read_direct_answer_returns_member_content() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"action":"read","archive":"/tmp/test_bundle.zip","member":"notes.txt","content":"fixture archive notes\n"}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveRead;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip | notes.txt".to_string();

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some("/tmp/test_bundle.zip | notes.txt".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("fixture archive notes")
    );
}

#[test]
fn archive_read_direct_answer_does_not_require_semantic_label() {
    let mut loop_state = LoopState::new(2);
    let body = r#"{"action":"read","archive":"/tmp/test_bundle.zip","member":"notes.txt","content":"fixture archive notes\n"}"#;
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Free);
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip".to_string();

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        auto_locator_path: Some("/tmp/test_bundle.zip".to_string()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("fixture archive notes")
    );
}

#[test]
fn archive_raw_passthrough_replacement_uses_structured_summary() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let state = AppState::test_default_with_fixture_provider();
    let replacement = archive_list_raw_passthrough_replacement(body, &state, &loop_state, "zh-CN")
        .expect("raw archive output should be replaced");
    assert!(replacement.contains("message_key=clawd.msg.archive_list.observed"));
    assert!(replacement.contains("reason_code=archive_list_observed"));
    assert!(replacement.contains("entry_count=2"));
    assert!(replacement.contains("shown_count=2"));
    assert!(replacement.contains("omitted_count=0"));
    assert!(replacement.contains("notes.txt"));
    assert!(replacement.contains("nested/config.ini"));
    assert!(!replacement.contains("Archive:"));
}

#[test]
fn archive_list_scalar_count_reads_entry_count_directly() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;

    let agent_run_context = AgentRunContext {
        route_result: Some(route.clone()),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)).as_deref(),
        Some("2")
    );
    assert!(scalar_count_diagnostic_line_for_answer("2", Some(&route), &loop_state).is_none());
}

#[test]
fn archive_entry_existence_reads_archive_list_directly() {
    let mut loop_state = LoopState::new(2);
    let body = "exit=0\nArchive:  /tmp/test_bundle.zip\n  Length      Date    Time    Name\n---------  ---------- -----   ----\n       22  2026-04-03 01:14   notes.txt\n       20  2026-04-03 01:14   nested/config.ini\n---------                     -------\n       42                     2 files";
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "archive_basic", body));
    let mut route = chat_wrapped_unclassified_route(OutputResponseShape::Scalar);
    route.resolved_intent =
        "Check whether notes.txt exists in /tmp/test_bundle.zip without extraction.".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPath;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/test_bundle.zip".to_string();

    let agent_run_context = AgentRunContext {
        route_result: Some(route),
        original_user_request: Some(
            "Only tell me whether notes.txt exists in /tmp/test_bundle.zip; do not extract it."
                .to_string(),
        ),
        auto_locator_path: Some("/tmp/test_bundle.zip".to_string()),
        ..AgentRunContext::default()
    };

    let answer = extract_direct_scalar_from_generic_output_i18n(
        &loop_state,
        &AppState::test_default_with_fixture_provider(),
        Some(&agent_run_context),
    )
    .expect("archive member existence should be answered from archive entries");
    assert!(answer.contains("notes.txt"), "answer: {answer}");
    assert!(answer.contains("exists"), "answer: {answer}");
    assert!(!answer.contains("nested/config.ini"), "answer: {answer}");
}

#[test]
fn structured_observed_body_includes_path_batch_metadata_for_synthesis() {
    let body = r#"{"action":"path_batch_facts","count":2,"facts":[{"exists":true,"fact":{"kind":"file","modified_ts":1777345844,"path":"Cargo.lock","resolved_path":"/tmp/repo/Cargo.lock","size_bytes":121657},"path":"/tmp/repo/Cargo.lock"},{"exists":true,"fact":{"kind":"file","modified_ts":1777357772,"path":"Cargo.toml","resolved_path":"/tmp/repo/Cargo.toml","size_bytes":2606},"path":"/tmp/repo/Cargo.toml"}],"include_missing":true}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "path_batch_facts\npath_fact name=Cargo.lock path=/tmp/repo/Cargo.lock exists=true kind=file size_bytes=121657 modified_ts=1777345844\npath_fact name=Cargo.toml path=/tmp/repo/Cargo.toml exists=true kind=file size_bytes=2606 modified_ts=1777357772"
            )
        );
}

#[test]
fn structured_observed_body_includes_inventory_dir_entry_metadata_for_synthesis() {
    let body = r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","sort_by":"size_desc"}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "inventory_dir path=/tmp/repo/prompts/schemas sort_by=size_desc total=2 files=2 dirs=0 hidden=0\nentry name=intent_normalizer.schema.json kind=file size_bytes=9402 modified_ts=1777513843\nentry name=plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
            )
        );
}

#[test]
fn structured_observed_body_includes_inventory_dir_size_summary_for_synthesis() {
    let body = r#"{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","size_summary":{"largest_file":{"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},"matched_file_count":2,"smallest_file":{"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187},"total_file_size_bytes":13589},"sort_by":"name"}"#;
    let observed = structured_observed_body("system_basic", body).expect("observed body");
    assert!(observed.contains("size_summary.matched_file_count=2"));
    assert!(observed.contains("size_summary.total_file_size_bytes=13589"));
    assert!(observed.contains(
        "size_summary.largest_file name=intent_normalizer.schema.json path=prompts/schemas/intent_normalizer.schema.json kind=file size_bytes=9402 modified_ts=1777513843"
    ));
    assert!(observed.contains(
        "size_summary.smallest_file name=plan_result.schema.json path=prompts/schemas/plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
    ));
}

#[test]
fn structured_observed_body_unwraps_extra_inventory_dir_for_synthesis() {
    let body = r#"{"extra":{"action":"inventory_dir","counts":{"dirs":0,"files":2,"hidden":0,"total":2},"entries":[{"hidden":false,"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},{"hidden":false,"kind":"file","modified_ts":1777526917,"name":"plan_result.schema.json","path":"prompts/schemas/plan_result.schema.json","size_bytes":4187}],"names":["intent_normalizer.schema.json","plan_result.schema.json"],"path":"prompts/schemas","resolved_path":"/tmp/repo/prompts/schemas","size_summary":{"largest_file":{"kind":"file","modified_ts":1777513843,"name":"intent_normalizer.schema.json","path":"prompts/schemas/intent_normalizer.schema.json","size_bytes":9402},"matched_file_count":2,"total_file_size_bytes":13589},"sort_by":"name"},"text":"raw wrapper fallback text"}"#;
    let observed = structured_observed_body("fs_basic", body).expect("observed body");
    assert!(observed.starts_with(
        "inventory_dir path=/tmp/repo/prompts/schemas sort_by=name total=2 files=2 dirs=0 hidden=0"
    ));
    assert!(observed.contains("size_summary.largest_file name=intent_normalizer.schema.json"));
    assert!(observed.contains(
        "entry name=plan_result.schema.json kind=file size_bytes=4187 modified_ts=1777526917"
    ));
    assert!(!observed.contains("raw wrapper fallback text"));
}

#[test]
fn structured_observed_body_includes_inventory_dir_names_by_kind_for_synthesis() {
    let body = r#"{"extra":{"action":"inventory_dir","counts":{"dirs":4,"files":1,"hidden":0,"total":5},"names_by_kind":{"dirs":["agent_rollout_metrics","base_skill_contracts_20260516_100540","base_skill_contracts_20260516_112927","base_skill_contracts_20260527_042323"],"files":["act_plan.log"],"other":[]},"path":"logs","resolved_path":"/tmp/repo/logs","sort_by":"name"},"text":"{}"}"#;
    let observed = structured_observed_body("fs_basic", body).expect("observed body");
    assert!(observed.starts_with(
        "inventory_dir path=/tmp/repo/logs sort_by=name total=5 files=1 dirs=4 hidden=0"
    ));
    assert!(observed.contains(
        "dir_entries=agent_rollout_metrics,base_skill_contracts_20260516_100540"
    ));
    assert!(observed.contains("file_entries=act_plan.log"));
    assert!(!observed.contains("raw wrapper fallback text"));
}

#[test]
fn structured_observed_body_compacts_large_inventory_dir_by_kind() {
    let entries = (0..9)
        .map(|idx| {
            serde_json::json!({
                "hidden": false,
                "kind": "dir",
                "modified_ts": 1777513843,
                "name": format!("dir_{idx}"),
                "path": format!("dir_{idx}"),
                "size_bytes": 0
            })
        })
        .chain((0..9).map(|idx| {
            serde_json::json!({
                "hidden": false,
                "kind": "file",
                "modified_ts": 1777513843,
                "name": format!("file_{idx}.md"),
                "path": format!("file_{idx}.md"),
                "size_bytes": 42
            })
        }))
        .collect::<Vec<_>>();
    let body = serde_json::json!({
        "action": "inventory_dir",
        "counts": {"dirs": 9, "files": 9, "hidden": 0, "total": 18},
        "entries": entries,
        "path": ".",
        "resolved_path": "/tmp/repo",
        "sort_by": "name"
    })
    .to_string();

    let observed = structured_observed_body("system_basic", &body).expect("observed body");
    assert!(observed.contains("dir_entries=dir_0:size_bytes=0,dir_1:size_bytes=0"));
    assert!(observed.contains("file_entries=file_0.md:size_bytes=42,file_1.md:size_bytes=42"));
    assert!(!observed.contains("modified_ts=1777513843"));
    assert!(observed.contains("size_bytes=42"));
}

#[test]
fn structured_observed_body_includes_count_inventory_breakdown_for_synthesis() {
    let body = r#"{"action":"count_inventory","counts":{"dirs":26,"files":40,"hidden":0,"total":66},"kind_filter":"any","path":".","resolved_path":"/tmp/repo"}"#;
    assert_eq!(
            structured_observed_body("system_basic", body).as_deref(),
            Some(
                "action=count_inventory\npath=.\nresolved_path=/tmp/repo\nkind_filter=any\ncount_files=40\ncount_dirs=26\ncount_total=66\ncount_hidden=0"
            )
        );
}

#[test]
fn sqlite_table_listing_summary_defers_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "db_basic",
        r#"{"columns":["name"],"rows":[{"name":"orders"},{"name":"users"}]}"#,
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "列一下 data/app.sqlite 里有哪些表".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: "normalizer:planner_execute_with_chat_finalizer".to_string(),
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
            semantic_kind: crate::OutputSemanticKind::SqliteTableListing,
            locator_hint: "data/app.sqlite".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)).is_none()
    );
}

#[test]
fn direct_scalar_defers_route_locator_hint_quantity_comparison_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "list_dir", "a\nb\n"));
    loop_state
        .executed_step_results
        .push(ok_step("step_2", "list_dir", "a\nb\nc\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "上一个和上上个哪个更多，只回答目录名".to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason:
            "'上一个'=assistant[-1](document,2), '上上个'=assistant[-2](scripts,3); scripts 更多"
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
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::QuantityComparison,
            locator_hint: "scripts".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_scalar_defers_compare_paths_result_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
            "step_1",
            "system_basic",
            r#"{"action":"compare_paths","left":{"path":"Cargo.toml","resolved_path":"/tmp/Cargo.toml","kind":"file","size_bytes":123},"right":{"path":"Cargo.lock","resolved_path":"/tmp/Cargo.lock","kind":"file","size_bytes":456},"comparison":{"same_kind":true,"same_name":false,"same_size":false,"size_delta_bytes":-333,"left_newer":null,"same_content":false}}"#,
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大，顺手用一句通俗话解释原因"
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
            locator_hint: "Cargo.lock|Cargo.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
    assert!(
        has_observed_answer_candidates(&loop_state),
        "compare_paths should remain available as observed facts for synthesis"
    );
}

#[test]
fn quantity_comparison_does_not_force_direct_scalar_observed_answer() {
    let route = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "比较 Cargo.toml 和 Cargo.lock 哪个更大".to_string(),
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
            locator_hint: "Cargo.lock|Cargo.toml".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    assert!(!super::route_prefers_direct_observed_answer_for_scalar(
        &route
    ));
}

#[test]
fn direct_answer_defers_git_status_dirty_worktree_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? new_file.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_repository_state_one_sentence_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_wrapped_git_repository_state_one_sentence_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    let output = serde_json::json!({
        "text": "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
        "extra": {
            "action": "status",
            "subcommand": "status",
            "exit_code": 0,
            "output": "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n"
        }
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "git_basic", &output));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "semantic contract only".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_strict_git_repository_state_when_exact_one_sentence() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "semantic contract only".to_string(),
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
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_repository_state_for_any_language() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "現在のリポジトリに未コミットの変更があるか、一文で答えてください"
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_does_not_override_git_state_language_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n?? tmp/generated.txt\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "synthesize_answer",
        "是的，当前仓库有 8 个文件有未提交的改动。",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "检查当前仓库是否存在未提交的改动，用一句话返回结果".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_branch_and_dirty_state_language_request() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n  dev\n* main\n  release\n",
    ));
    loop_state.executed_step_results.push(ok_step(
        "step_2",
        "git_basic",
        "exit=0\n## main...origin/main\n M Cargo.toml\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent:
            "show the current git branch, then say whether the worktree looks clean or mid-edit"
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
            exact_sentence_count: Some(1),
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
            route_result: Some(route_result),
            original_user_request: Some(
                "show the current git branch, then say in one plain English sentence whether the worktree looks clean or mid-edit"
                    .to_string(),
            ),
            ..AgentRunContext::default()
        };

    assert_eq!(
        extract_direct_answer_from_generic_output_i18n(
            &loop_state,
            &AppState::test_default_with_fixture_provider(),
            Some(&agent_run_context)
        ),
        None
    );
}

#[test]
fn direct_answer_defers_git_log_release_note_to_synthesis() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "read_file",
        "RustClaw is a local Rust agent runtime centered on clawd.",
    ));
    loop_state.executed_step_results.push(ok_step(
            "step_2",
            "system_basic",
            r#"{"action":"extract_field","field_path":"workspace.package.version","value_text":"0.1.7"}"#,
        ));
    loop_state.executed_step_results.push(ok_step(
            "step_3",
            "git_basic",
            "exit=0\n09342a6a fix: expose nl execution and locator flows\n336e8d92 docs: update planner-first architecture diagrams\n",
        ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "Write a short release note for RustClaw.".to_string(),
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
            response_shape: OutputResponseShape::Free,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            locator_hint: "RustClaw".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_scalar_extracts_git_commit_subject_from_oneline_log() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        "exit=0\n09342a6a fix: expose nl execution and locator flows\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "return the latest git commit subject only".to_string(),
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
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::GitCommitSubject,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        Some("fix: expose nl execution and locator flows".to_string())
    );
}

#[test]
fn direct_scalar_extracts_git_current_branch_from_structured_field() {
    let mut loop_state = LoopState::new(2);
    let output = serde_json::json!({
        "text": "exit=0\nmain\n",
        "extra": {
            "schema_version": 1,
            "action": "current_branch",
            "raw_action": "current_branch",
            "subcommand": "rev-parse",
            "exit_code": 0,
            "output": "exit=0\nmain\n",
            "branch": "main",
            "current_branch": "main",
            "field_value": {
                "action": "current_branch",
                "exit_code": 0,
                "branch": "main",
                "current_branch": "main"
            }
        },
        "error_text": null,
        "status": "ok"
    })
    .to_string();
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "git_basic", &output));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "semantic contract only".to_string(),
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
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::GitRepositoryState,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };

    assert_eq!(
        extract_direct_scalar_from_generic_output(&loop_state, Some(&agent_run_context)),
        Some("main".to_string())
    );
}

#[test]
fn direct_answer_defers_git_status_clean_when_exit_only_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state
        .executed_step_results
        .push(ok_step("step_1", "git_basic", "exit=0\n"));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "看看这个仓库现在有没有未提交改动，用一句话告诉我".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}

#[test]
fn direct_answer_defers_git_status_dirty_without_branch_header_to_llm() {
    let mut loop_state = LoopState::new(2);
    loop_state.executed_step_results.push(ok_step(
        "step_1",
        "git_basic",
        " M Cargo.toml\n?? new_file.txt\n",
    ));
    let route_result = RouteResult {
        ask_mode: crate::AskMode::act_plain(),
        resolved_intent: "看看这个仓库现在有没有未提交改动，用一句话告诉我".to_string(),
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
            response_shape: OutputResponseShape::OneSentence,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    };
    let agent_run_context = AgentRunContext {
        route_result: Some(route_result),
        ..AgentRunContext::default()
    };
    assert_eq!(
        extract_direct_answer_from_generic_output(&loop_state, Some(&agent_run_context)),
        None
    );
}
