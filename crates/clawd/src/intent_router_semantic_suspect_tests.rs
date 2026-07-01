// Semantic suspect and contract-integrity tests for intent_router.

use serde_json::Value;

#[test]
fn semantic_suspect_ignores_legacy_chat_hint_with_observable_contract() {
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            None,
            "",
            std::path::Path::new("/tmp/rustclaw")
        ),
        None
    );
}

#[test]
fn semantic_suspect_treats_semantic_kind_alone_as_descriptive() {
    let out = super::IntentNormalizerOut {
        resolved_user_intent: "descriptive contract only".to_string(),
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
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "file_names".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            None,
            "",
            std::path::Path::new("/tmp/rustclaw")
        ),
        None
    );
}

#[test]
fn semantic_suspect_reviews_workspace_identity_free_chat_route() {
    let req = "Write a long article about RustClaw";
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
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "free".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
        Some("workspace_identity_chat_route_needs_semantic_review")
    );
}

#[test]
fn semantic_suspect_keeps_workspace_identity_one_sentence_chat_route() {
    let req = "你好，用一句话说明 RustClaw 适合帮我做什么";
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
        decision: "direct_answer".to_string(),
        schedule_intent: None,
        output_contract: Some(super::IntentOutputContractOut {
            response_shape: "one_sentence".to_string(),
            exact_sentence_count: Some(Value::from(1)),
            requires_content_evidence: false,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
        None
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            None,
            "",
            std::path::Path::new("/tmp/rustclaw")
        ),
        Some("file_names_contract_needs_semantic_shape_review")
    );
}

#[test]
fn llm_contract_integrity_repair_skips_regular_shape_review_details() {
    let mut report = super::ContractRepairReport::default();
    report.add(
        "semantic_suspect",
        "file_names_contract_needs_semantic_shape_review",
    );
    report.add(
        "semantic_suspect",
        "single_path_generic_contract_needs_semantic_shape_review",
    );

    assert!(
        !report.needs_llm_contract_integrity_repair(),
        "ordinary executable contract shape refinement belongs to the planner loop, not the pre-planner repair judge"
    );
}

#[test]
fn llm_contract_integrity_repair_keeps_malformed_machine_field_details() {
    let mut report = super::ContractRepairReport::default();
    report.add(
        "semantic_suspect",
        "executable_route_unknown_scalar_output_contract",
    );

    assert!(report.needs_llm_contract_integrity_repair());
}

#[test]
fn semantic_suspect_reviews_locatorless_generic_evidence_contract() {
    let out = super::IntentNormalizerOut {
        resolved_user_intent: "count direct child files in the current workspace".to_string(),
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
            exact_sentence_count: Some(Value::from(1)),
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "none".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: "status_query".to_string(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            None,
            "",
            std::path::Path::new("/tmp/rustclaw")
        ),
        Some("locatorless_generic_evidence_contract_needs_semantic_shape_review")
    );
}

#[test]
fn llm_contract_integrity_repair_runs_for_locatorless_generic_evidence_detail() {
    let mut report = super::ContractRepairReport::default();
    report.add(
        "semantic_suspect",
        "locatorless_generic_evidence_contract_needs_semantic_shape_review",
    );

    assert!(report.needs_llm_contract_integrity_repair());
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
                scalar_count_filter: None,
                list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            None,
            "",
            std::path::Path::new("/tmp/rustclaw")
        ),
        Some("file_paths_contract_needs_semantic_shape_review")
    );
}

#[test]
fn semantic_suspect_reviews_raw_command_output_with_locator_without_command_payload() {
    let req = "Read plan/missing.md; if it is missing, search the plan directory and return matching paths.";
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
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "raw_command_output".to_string(),
            locator_hint: "plan/missing.md".to_string(),
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
        Some("raw_command_output_locator_needs_semantic_review")
    );
}

#[test]
fn semantic_suspect_reviews_explicit_command_summary_for_failure_contract() {
    let req = "please run echo ok and then report the failed command step";
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
            exact_sentence_count: Some(Value::from(1)),
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "command_output_summary".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: vec![],
        execute_prefixes: vec!["please run ".to_string()],
        standalone_commands: vec!["echo".to_string()],
        default_locale: "en-US".to_string(),
        verify_enforce_enabled: true,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output_with_command_runtime(
            &out,
            None,
            req,
            std::path::Path::new("/tmp/rustclaw"),
            Some(&runtime),
        ),
        Some("command_output_summary_needs_failure_contract_review")
    );
}

#[test]
fn semantic_suspect_reviews_command_summary_from_resolved_intent_command() {
    let req = "请依次执行命令 `echo ok`，然后报告失败步骤";
    let out = super::IntentNormalizerOut {
        resolved_user_intent: "Run command `echo ok`, then report the failed command step."
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
            response_shape: "one_sentence".to_string(),
            exact_sentence_count: Some(Value::from(1)),
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "none".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "command_output_summary".to_string(),
            locator_hint: String::new(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut::default()),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let runtime = crate::CommandIntentRuntime {
        all_result_suffixes: vec![],
        execute_prefixes: vec!["Run command ".to_string()],
        standalone_commands: vec![],
        default_locale: "en-US".to_string(),
        verify_enforce_enabled: true,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output_with_command_runtime(
            &out,
            None,
            req,
            std::path::Path::new("/tmp/rustclaw"),
            Some(&runtime),
        ),
        Some("command_output_summary_needs_failure_contract_review")
    );
}

#[test]
fn semantic_suspect_keeps_raw_command_output_with_active_execution_recipe() {
    let req = "Run the requested command and return its output.";
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
            response_shape: "strict".to_string(),
            exact_sentence_count: None,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "raw_command_output".to_string(),
            locator_hint: "plan".to_string(),
            scalar_count_filter: None,
            list_selector: None,
            self_extension: None,
        }),
        execution_recipe: Some(super::IntentExecutionRecipeOut {
            kind: "ops_closed_loop".to_string(),
            profile: "none".to_string(),
            target_scope: "unknown".to_string(),
            ..super::IntentExecutionRecipeOut::default()
        }),
        turn_type: String::new(),
        target_task_policy: String::new(),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert_eq!(
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
        None
    );
}

#[test]
fn semantic_suspect_reviews_planner_directory_entry_groups_contract() {
    let out = super::IntentNormalizerOut {
        resolved_user_intent:
            "List documentation files in document and explain which one is most relevant."
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
            locator_kind: "path".to_string(),
            delivery_intent: "none".to_string(),
            semantic_kind: "directory_entry_groups".to_string(),
            locator_hint: "document/".to_string(),
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            None,
            "",
            std::path::Path::new("/tmp/rustclaw")
        ),
        Some("directory_entry_groups_contract_needs_semantic_shape_review")
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            None,
            "",
            std::path::Path::new("/tmp/rustclaw")
        ),
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
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
            scalar_count_filter: None,
            list_selector: None,
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
        super::semantic_suspect_detail_for_normalizer_output(
            &out,
            Some(&surface),
            req,
            std::path::Path::new("/tmp/rustclaw")
        ),
        Some("single_path_scalar_count_contract_needs_semantic_shape_review")
    );
}
