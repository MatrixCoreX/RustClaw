use super::{
    active_task_context_only_memory_context, apply_execution_context_to_prompts,
    build_active_task_context, build_request_surface_hints, build_session_alias_context,
    chat_memory_context_hint, classify_route_context_budget, current_request_only_memory_context,
    needs_text_anchor_probe_for_route, observed_facts_provide_immediate_anchor,
    planner_memory_context_hint, request_can_fill_active_clarify_target,
    request_qualifies_for_anchor_only_route_context, route_needs_recent_execution_history,
    session_snapshot_has_primary_task_context, session_snapshot_provides_execution_state_anchor,
    should_prefer_light_execution_memory_from_session, should_suppress_execution_anchor_context,
    uses_light_execution_context_budget, ExecutionContextView, PlannerContextView,
    RouteContextBudgetTier, TaskContextBundle, TaskContextRawSources,
};

#[test]
fn active_task_context_is_empty_without_primary_task_state() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(build_active_task_context(&snapshot), "<none>");
}

#[test]
fn active_task_context_includes_primary_prompt_and_output() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Write one deployment note that mentions Python 3.10".to_string(),
            ),
            last_primary_task_output: Some(
                "Ensure the target environment has Python 3.10 installed.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let context = build_active_task_context(&snapshot);
    assert!(context.contains("### ACTIVE_TASK_CONTEXT"));
    assert!(context.contains("last_primary_task_prompt:"));
    assert!(context.contains("Write one deployment note"));
    assert!(context.contains("last_primary_task_output:"));
    assert!(context.contains("Python 3.10 installed"));
    assert!(context.contains("not a filesystem locator"));
}

#[test]
fn primary_task_output_counts_as_active_task_context_for_memory_hint() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_output: Some("Draft checklist".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!session_snapshot_provides_execution_state_anchor(&snapshot));
    assert!(session_snapshot_has_primary_task_context(&snapshot));
}

#[test]
fn active_task_context_truncates_long_primary_output() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a compact test plan".to_string()),
            last_primary_task_output: Some("x".repeat(1100)),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let context = build_active_task_context(&snapshot);
    assert!(context.contains("...(truncated)"));
    assert!(context.len() < 1500);
}

#[test]
fn session_alias_context_exports_temporary_bindings() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "那个日志".to_string(),
                target: "/tmp/app.log".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let context = build_session_alias_context(&snapshot);
    assert!(context.contains("### SESSION_ALIAS_BINDINGS"));
    assert!(context.contains("那个日志"));
    assert!(context.contains("/tmp/app.log"));
    assert!(context.contains("not execution evidence"));
}

#[test]
fn request_surface_hints_do_not_export_semantic_phrase_shapes() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "看看 data/db-basic-contract.sqlite 里有哪些表，并简短告诉我结果",
    );
    let rendered = build_request_surface_hints(&surface);
    assert_eq!(rendered, "<none>");

    let surface = crate::intent::surface_signals::analyze_prompt_surface("看看 docs 目录");
    let rendered = build_request_surface_hints(&surface);
    assert_eq!(rendered, "<none>");
    assert!(!rendered.contains("workspace_child_directory_hint"));

    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "用一句话说当前机器的包管理器是什么",
    );
    let rendered = build_request_surface_hints(&surface);
    assert_eq!(rendered, "<none>");
    assert!(!rendered.contains("workspace_root_request_shape"));
    assert!(!rendered.contains("semantic_request_shape"));
    assert!(!rendered.contains("table_request_shape"));
    assert!(!rendered.contains("output_request_shape"));
    assert!(!rendered.contains("output_compression_shape"));
}

#[test]
fn request_surface_hints_do_not_export_read_range_phrases() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "先读一下 README.md 前 4 行，再用一句话说重点",
    );
    let rendered = build_request_surface_hints(&surface);
    assert_eq!(rendered, "<none>");
}

#[test]
fn request_surface_hints_include_locator_target_pair() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "比较 README.md 和 AGENTS.md 哪个更大",
    );
    let rendered = build_request_surface_hints(&surface);
    assert!(rendered.contains("### REQUEST_SURFACE_HINTS"));
    assert!(rendered.contains("locator_target_pair:"));
    assert!(rendered.contains("README.md"));
    assert!(rendered.contains("AGENTS.md"));
}

#[test]
fn route_budget_uses_anchor_only_for_explicit_local_file_reads() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
    );
    assert!(request_qualifies_for_anchor_only_route_context(
        "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
        &surface,
    ));
    assert_eq!(
        classify_route_context_budget(
            "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo，只输出值",
            &surface,
            "<none>",
            "<none>",
            None,
            None,
            None,
        ),
        RouteContextBudgetTier::AnchorOnly
    );
}

#[test]
fn route_budget_uses_anchor_only_for_explicit_compare_targets() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
    );
    assert!(request_qualifies_for_anchor_only_route_context(
        "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
        &surface,
    ));
    assert_eq!(
        classify_route_context_budget(
            "比较 README.md 和 AGENTS.md 哪个更大，再用一句通俗话解释原因",
            &surface,
            "<none>",
            "<none>",
            None,
            None,
            None,
        ),
        RouteContextBudgetTier::AnchorOnly
    );
}

#[test]
fn route_budget_uses_anchor_only_for_filename_only_excerpt_reads() {
    let surface =
        crate::intent::surface_signals::analyze_prompt_surface("先读一下 README.md 前 4 行");
    assert!(request_qualifies_for_anchor_only_route_context(
        "先读一下 README.md 前 4 行",
        &surface,
    ));
    assert_eq!(
        classify_route_context_budget(
            "先读一下 README.md 前 4 行",
            &surface,
            "<none>",
            "<none>",
            None,
            None,
            None
        ),
        RouteContextBudgetTier::AnchorOnly
    );
}

#[test]
fn route_budget_keeps_full_context_for_ambiguous_followup_language() {
    assert_eq!(
        classify_route_context_budget(
            "看一下那个日志最后 4 行",
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            "<none>",
            "<none>",
            None,
            None,
            None
        ),
        RouteContextBudgetTier::Full
    );
}

#[test]
fn route_budget_keeps_full_context_for_followup_language_with_immediate_anchor() {
    let last_turn_full = "### LAST_TURN_FULL\n[TURN -1]\nAssistant: FILE:/home/guagua/rustclaw/logs/model_io.log\n[/TURN]";
    assert_eq!(
        classify_route_context_budget(
            "看一下那个日志最后 4 行",
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            last_turn_full,
            "<none>",
            None,
            None,
            None
        ),
        RouteContextBudgetTier::Full
    );
}

#[test]
fn route_budget_leaves_weak_active_locator_clarify_reply_to_normalizer() {
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: None,
        semantic_kind: None,
        source_request: "看一下那个日志最后 5 行".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    assert!(!request_can_fill_active_clarify_target(
        "/tmp/device_local/logs/model_io.log",
        &crate::intent::surface_signals::analyze_prompt_surface(
            "/tmp/device_local/logs/model_io.log",
        ),
        Some(&clarify_state)
    ));
    assert_eq!(
        classify_route_context_budget(
            "/tmp/device_local/logs/model_io.log",
            &crate::intent::surface_signals::analyze_prompt_surface(
                "/tmp/device_local/logs/model_io.log",
            ),
            "<none>",
            "<none>",
            None,
            Some(&clarify_state),
            None
        ),
        RouteContextBudgetTier::AnchorOnly
    );
}

#[test]
fn route_budget_leaves_active_clarify_candidate_selection_to_normalizer() {
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        candidate_targets: vec!["act_plan.log".to_string(), "clawd.log".to_string()],
        delivery_required: true,
        output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
        semantic_kind: None,
        source_request: "把那个文件发给我".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    assert!(!request_can_fill_active_clarify_target(
        "第二个",
        &crate::intent::surface_signals::analyze_prompt_surface("第二个"),
        Some(&clarify_state)
    ));
    assert_eq!(
        classify_route_context_budget(
            "第二个",
            &crate::intent::surface_signals::analyze_prompt_surface("第二个"),
            "<none>",
            "<none>",
            None,
            Some(&clarify_state),
            None
        ),
        RouteContextBudgetTier::Full
    );
}

#[test]
fn observed_facts_anchor_is_preserved_without_local_followup_word_budgeting() {
    let facts = crate::observed_facts::ObservedFacts {
        bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
        ..crate::observed_facts::ObservedFacts::default()
    };
    assert!(observed_facts_provide_immediate_anchor(Some(&facts)));
    assert_eq!(
        classify_route_context_budget(
            "看一下那个日志最后 4 行",
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            "<none>",
            "<none>",
            None,
            None,
            Some(&facts)
        ),
        RouteContextBudgetTier::Full
    );
}

#[test]
fn followup_frame_anchor_is_preserved_without_local_followup_word_budgeting() {
    let frame = crate::followup_frame::FollowupFrame {
        bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
        ..crate::followup_frame::FollowupFrame::default()
    };
    assert_eq!(
        classify_route_context_budget(
            "看一下那个日志最后 4 行",
            &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
            "<none>",
            "<none>",
            Some(&frame),
            None,
            None
        ),
        RouteContextBudgetTier::Full
    );
}

#[test]
fn text_anchor_probe_is_skipped_when_session_already_has_locator_anchor() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!needs_text_anchor_probe_for_route(
        "看一下那个日志最后 4 行",
        &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
        &snapshot
    ));
}

#[test]
fn text_anchor_probe_is_not_used_for_followup_word_matching() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!needs_text_anchor_probe_for_route(
        "看一下那个日志最后 4 行",
        &crate::intent::surface_signals::analyze_prompt_surface("看一下那个日志最后 4 行",),
        &snapshot
    ));
}

#[test]
fn text_anchor_probe_is_skipped_when_session_alias_binding_matches_prompt() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "那个 README".to_string(),
                target: "/tmp/device_local/README.md".to_string(),
                updated_at_ts: 1,
            }],
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!needs_text_anchor_probe_for_route(
        "读一下那个 README 开头，然后一句话总结",
        &crate::intent::surface_signals::analyze_prompt_surface(
            "读一下那个 README 开头，然后一句话总结",
        ),
        &snapshot
    ));
}

#[test]
fn execution_text_context_is_suppressed_when_session_has_clarify_state() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            candidate_targets: vec![],
            delivery_required: false,
            output_shape: None,
            semantic_kind: None,
            source_request: "读一下那个 README".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    assert!(session_snapshot_provides_execution_state_anchor(&snapshot));
}

#[test]
fn execution_text_context_is_suppressed_when_session_has_followup_or_observed_anchor() {
    let followup_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(session_snapshot_provides_execution_state_anchor(
        &followup_snapshot
    ));

    let observed_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };
    assert!(session_snapshot_provides_execution_state_anchor(
        &observed_snapshot
    ));
}

#[test]
fn execution_text_context_is_not_suppressed_without_session_anchor() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!session_snapshot_provides_execution_state_anchor(&snapshot));
}

#[test]
fn session_anchored_chat_wrapped_content_reads_prefer_light_memory_budget() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("/tmp/device_local/README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(should_prefer_light_execution_memory_from_session(
        &route, &snapshot
    ));
}

#[test]
fn open_or_unanchored_routes_do_not_force_light_memory_budget() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    let unanchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!should_prefer_light_execution_memory_from_session(
        &route,
        &unanchored_snapshot
    ));

    let mut planning_like = route.clone();
    planning_like.ask_mode = crate::AskMode::planner_execute_plain();
    let anchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("/tmp/device_local/README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!should_prefer_light_execution_memory_from_session(
        &planning_like,
        &anchored_snapshot
    ));
}

#[test]
fn session_anchored_normalizer_chat_keeps_full_memory_budget_for_recall() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::direct_answer();
    let anchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some("file_token".to_string()),
            semantic_kind: None,
            source_request: "把那个文件发给我".to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 60,
        }),
        active_observed_facts: None,
    };
    assert!(!should_prefer_light_execution_memory_from_session(
        &route,
        &anchored_snapshot
    ));
}

#[test]
fn session_anchored_clarify_delivery_act_prefers_light_memory_budget() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.output_contract.delivery_required = true;
    route.output_contract.response_shape = crate::OutputResponseShape::FileToken;
    let anchored_snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some("file_token".to_string()),
            semantic_kind: None,
            source_request: "把那个文件发给我".to_string(),
            source_task_id: "task-clarify".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 60,
        }),
        active_observed_facts: None,
    };
    assert!(should_prefer_light_execution_memory_from_session(
        &route,
        &anchored_snapshot
    ));
}

#[test]
fn ordinary_normalizer_chat_without_state_anchor_keeps_full_memory_budget() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::direct_answer();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!should_prefer_light_execution_memory_from_session(
        &route, &snapshot
    ));
}

#[test]
fn stateful_light_routes_suppress_execution_anchor_context() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("/tmp/device_local/README.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(should_suppress_execution_anchor_context(
        &route,
        &snapshot,
        crate::task_context_builder::ExecutionContextBudgetTier::Light,
    ));
}

#[test]
fn quantity_comparison_keeps_recent_execution_history() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/tmp/repo".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            bound_target: Some("document".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(route_needs_recent_execution_history(&route));
    assert!(!uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
    assert!(!should_suppress_execution_anchor_context(
        &route,
        &snapshot,
        crate::task_context_builder::ExecutionContextBudgetTier::Full,
    ));
}

#[test]
fn recent_observed_judgments_keep_recent_execution_history() {
    for semantic_kind in [
        crate::OutputSemanticKind::ContentExcerptSummary,
        crate::OutputSemanticKind::ContentExcerptWithSummary,
        crate::OutputSemanticKind::ContentPresenceCheck,
        crate::OutputSemanticKind::ExcerptKindJudgment,
        crate::OutputSemanticKind::RecentArtifactsJudgment,
        crate::OutputSemanticKind::FileBasename,
    ] {
        let mut route = base_route_result();
        route.output_contract.semantic_kind = semantic_kind;
        route.output_contract.response_shape = crate::OutputResponseShape::Strict;
        route.output_contract.requires_content_evidence = true;

        assert!(
            route_needs_recent_execution_history(&route),
            "{semantic_kind:?} should keep recent execution history"
        );
        assert!(
            !uses_light_execution_context_budget(&route, &route.resolved_intent),
            "{semantic_kind:?} should not use the light execution budget"
        );
    }
}

#[test]
fn unanchored_light_routes_keep_execution_anchor_context() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(!should_suppress_execution_anchor_context(
        &route,
        &snapshot,
        crate::task_context_builder::ExecutionContextBudgetTier::Light,
    ));
}

fn base_route_result() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: String::new(),
        route_confidence: Some(0.9),
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

#[test]
fn standalone_direct_answer_without_turn_analysis_disables_stable_facts_memory() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::direct_answer();

    assert!(current_request_only_memory_context(&route, None, false));
    assert_eq!(
        chat_memory_context_hint(&route, None, false),
        crate::memory::use_policy::ChatMemoryContextHint::CurrentRequestOnly
    );
    assert_eq!(
        planner_memory_context_hint(&route, None, false),
        crate::memory::use_policy::PlannerMemoryContextHint::StableFactsDisabled
    );
}

#[test]
fn explicit_preference_or_memory_turn_keeps_default_memory_policy() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::direct_answer();
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::PreferenceOrMemory),
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(!current_request_only_memory_context(
        &route,
        Some(&analysis),
        false
    ));
    assert_eq!(
        chat_memory_context_hint(&route, Some(&analysis), false),
        crate::memory::use_policy::ChatMemoryContextHint::Default
    );
    assert_eq!(
        planner_memory_context_hint(&route, Some(&analysis), false),
        crate::memory::use_policy::PlannerMemoryContextHint::Default
    );
}

#[test]
fn active_task_scope_update_uses_active_task_context_only_for_chat_memory() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::direct_answer();
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };

    assert!(active_task_context_only_memory_context(
        &route,
        Some(&analysis),
        true
    ));
    assert_eq!(
        chat_memory_context_hint(&route, Some(&analysis), true),
        crate::memory::use_policy::ChatMemoryContextHint::ActiveTaskContextOnly
    );
    assert_eq!(
        planner_memory_context_hint(&route, Some(&analysis), true),
        crate::memory::use_policy::PlannerMemoryContextHint::Default
    );
}

#[test]
fn light_execution_budget_detects_scalar_manifest_reads() {
    let mut route = base_route_result();
    route.resolved_intent = "读取 UI/package.json 里的 name 字段，只输出值".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    assert!(uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}

#[test]
fn execution_context_adds_recent_turns_to_chat_prompt_before_last_turn_fallback() {
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        route_view: None,
        execution_view: Some(ExecutionContextView {
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
            runtime_context: "<none>".to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "### RECENT_TURNS_FULL\n[TURN -2]\nUser: 请记住测试编号 client-like-continuous-1\nAssistant: 已记住\n[/TURN]".to_string(),
            last_turn_full: "### LAST_TURN_FULL\n[TURN -1]\nUser: other\nAssistant: other\n[/TURN]".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };
    let mut chat_context = "### MEMORY_CONTEXT\n<none>".to_string();
    let mut resolved = "刚才编号是什么".to_string();
    let mut execution = "刚才编号是什么".to_string();
    apply_execution_context_to_prompts(&bundle, &mut chat_context, &mut resolved, &mut execution);
    assert!(chat_context.contains("### RECENT_TURNS_FULL"));
    assert!(chat_context.contains("client-like-continuous-1"));
    assert!(!chat_context.contains("### LAST_TURN_FULL"));
}

#[test]
fn execution_context_adds_runtime_context_to_chat_and_planner_prompts() {
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        route_view: None,
        execution_view: Some(ExecutionContextView {
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
            runtime_context: "### RUNTIME_CONTEXT\ncurrent_process_cwd: /tmp/workspace\nworkspace_root: /tmp/workspace".to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };
    let mut chat_context = "### MEMORY_CONTEXT\n<none>".to_string();
    let mut resolved = "当前工作目录是哪个".to_string();
    let mut execution = "当前工作目录是哪个".to_string();
    apply_execution_context_to_prompts(&bundle, &mut chat_context, &mut resolved, &mut execution);
    assert!(chat_context.contains("### RUNTIME_CONTEXT"));
    assert!(chat_context.contains("current_process_cwd: /tmp/workspace"));
    assert!(execution.contains("### RUNTIME_CONTEXT"));
    assert!(execution.contains("workspace_root: /tmp/workspace"));
}

#[test]
fn execution_context_uses_recent_execution_context_fallback_for_planner_prompt() {
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        route_view: None,
        execution_view: Some(ExecutionContextView {
            budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Light,
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
            runtime_context: "<none>".to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context:
                "### RECENT_EXECUTION_ANCHOR\n- latest_request=数一下 document\n- latest_result=document: 34"
                    .to_string(),
            image_context: None,
        }),
    };
    let mut chat_context = "### MEMORY_CONTEXT\n<none>".to_string();
    let mut resolved = "上一个和上上个哪个更多".to_string();
    let mut execution = resolved.clone();

    apply_execution_context_to_prompts(&bundle, &mut chat_context, &mut resolved, &mut execution);

    assert!(execution.contains("### RECENT_EXECUTION_CONTEXT"));
    assert!(execution.contains("latest_result=document: 34"));
}

#[test]
fn execution_context_adds_active_ordered_anchor_to_planner_prompts() {
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        route_view: None,
        execution_view: Some(ExecutionContextView {
            budget_tier: crate::task_context_builder::ExecutionContextBudgetTier::Light,
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
            runtime_context: "<none>".to_string(),
            active_execution_anchor_context:
                "### ACTIVE_EXECUTION_ANCHOR\nfollowup_bound_target: /tmp/rustclaw/crates\nfollowup_ordered_entries: 1:claw-core | 2:clawcli | 3:clawd | 4:feishud | 5:larkd"
                    .to_string(),
            session_alias_context: "<none>".to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };
    let mut chat_context = "### MEMORY_CONTEXT\n<none>".to_string();
    let mut resolved = "获取 crates 目录下最后一个条目的路径和类型".to_string();
    let mut execution = resolved.clone();

    apply_execution_context_to_prompts(&bundle, &mut chat_context, &mut resolved, &mut execution);

    assert!(resolved.contains("### ACTIVE_EXECUTION_ANCHOR"));
    assert!(resolved.contains("followup_ordered_entries"));
    assert!(resolved.contains("5:larkd"));
    assert!(resolved.contains("Do not re-list, sort, or reinterpret"));
    assert!(execution.contains("followup_bound_target: /tmp/rustclaw/crates"));
    assert!(!chat_context.contains("### ACTIVE_EXECUTION_ANCHOR"));
}

#[test]
fn execution_context_adds_session_alias_bindings_to_planner_prompts() {
    let bundle = TaskContextBundle {
        raw_sources: TaskContextRawSources::default(),
        planner_view: PlannerContextView::default(),
        route_view: None,
        execution_view: Some(ExecutionContextView {
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
            runtime_context: "<none>".to_string(),
            active_execution_anchor_context: "<none>".to_string(),
            session_alias_context:
                "### SESSION_ALIAS_BINDINGS\n- alias: 甲目录\n  target: /tmp/docs/archive\n- alias: 乙文件\n  target: /tmp/docs/release_checklist.md"
                    .to_string(),
            recent_turns_full: "<none>".to_string(),
            last_turn_full: "<none>".to_string(),
            recent_execution_anchor: "<none>".to_string(),
            recent_execution_context: "<none>".to_string(),
            image_context: None,
        }),
    };
    let mut chat_context = "### MEMORY_CONTEXT\n<none>".to_string();
    let mut resolved = "列一下甲目录里的名字，再说明乙文件".to_string();
    let mut execution = resolved.clone();
    apply_execution_context_to_prompts(&bundle, &mut chat_context, &mut resolved, &mut execution);
    assert!(resolved.contains("### SESSION_ALIAS_BINDINGS"));
    assert!(resolved.contains("target: /tmp/docs/release_checklist.md"));
    assert!(resolved.contains("independent authoritative concrete target"));
    assert!(execution.contains("target: /tmp/docs/archive"));
    assert!(!chat_context.contains("### SESSION_ALIAS_BINDINGS"));
}

#[test]
fn light_execution_budget_detects_generic_explicit_scalar_path_reads() {
    let mut route = base_route_result();
    route.resolved_intent =
        "读取 /home/guagua/rustclaw/configs/config.toml 中的 tools.allow_sudo 配置项的值，并仅输出该值"
            .to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/home/guagua/rustclaw/configs/config.toml".to_string();
    assert!(uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}

#[test]
fn light_execution_budget_detects_explicit_tail_reads() {
    let mut route = base_route_result();
    route.resolved_intent = "看一下 /tmp/model_io.log 最后 5 行".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/model_io.log".to_string();
    assert!(uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}

#[test]
fn light_execution_budget_detects_bounded_listing_and_existence() {
    let mut listing = base_route_result();
    listing.resolved_intent = "列出 logs 目录下前 5 个文件名".to_string();
    listing.output_contract.semantic_kind = crate::OutputSemanticKind::FileNames;
    listing.output_contract.requires_content_evidence = true;
    listing.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    listing.output_contract.locator_hint = "logs".to_string();
    assert!(uses_light_execution_context_budget(
        &listing,
        &listing.resolved_intent
    ));

    let mut existence = base_route_result();
    existence.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPath;
    existence.resolved_intent = "看看 /tmp/rustclaw.service 在不在".to_string();
    assert!(uses_light_execution_context_budget(
        &existence,
        &existence.resolved_intent
    ));
}

#[test]
fn light_execution_budget_detects_scalar_path_only_pwd_route() {
    let mut route = base_route_result();
    route.resolved_intent = "只输出当前工作目录的绝对路径，不要解释".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    assert!(uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}

#[test]
fn light_execution_budget_detects_structured_chat_wrapped_content_reads() {
    let mut read_range = base_route_result();
    read_range.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    read_range.resolved_intent = "先读一下 README.md 前 4 行".to_string();
    read_range.output_contract.response_shape = crate::OutputResponseShape::Free;
    read_range.output_contract.requires_content_evidence = true;
    read_range.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    read_range.output_contract.locator_hint = "README.md".to_string();
    assert!(uses_light_execution_context_budget(
        &read_range,
        &read_range.resolved_intent
    ));

    let mut single_read = base_route_result();
    single_read.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    single_read.resolved_intent =
        "看一下 /home/guagua/rustclaw/configs/config.toml，然后一句话说它主要配了什么".to_string();
    single_read.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    single_read.output_contract.requires_content_evidence = true;
    single_read.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    single_read.output_contract.locator_hint =
        "/home/guagua/rustclaw/configs/config.toml".to_string();
    assert!(uses_light_execution_context_budget(
        &single_read,
        &single_read.resolved_intent
    ));
}

#[test]
fn light_execution_budget_skips_workspace_project_summary() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "Summarize the current repository, focusing only on the UI components".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::WorkspaceProjectSummary;
    assert!(!uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}

#[test]
fn light_execution_budget_detects_clarify_rewrite_bound_reads() {
    let mut route = base_route_result();
    route.resolved_intent = "Continue the previous request that was waiting for clarification: 读一下那个文件里的名字字段，只输出值\nUser now provides the missing target or content: package.json".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "package.json".to_string();
    assert!(uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}

#[test]
fn light_execution_budget_skips_non_structured_chat_wrapped_or_clarify_routes() {
    let mut chat_wrapped_execution = base_route_result();
    chat_wrapped_execution.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    chat_wrapped_execution.resolved_intent = "比较这两个文件大小，然后一句话总结".to_string();
    assert!(!uses_light_execution_context_budget(
        &chat_wrapped_execution,
        &chat_wrapped_execution.resolved_intent
    ));

    let mut delivery = base_route_result();
    delivery.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    delivery.resolved_intent = "把 README.md 发给我".to_string();
    delivery.output_contract.requires_content_evidence = true;
    delivery.output_contract.delivery_required = true;
    assert!(!uses_light_execution_context_budget(
        &delivery,
        &delivery.resolved_intent
    ));

    let mut clarify = base_route_result();
    clarify.needs_clarify = true;
    clarify.resolved_intent = "看一下那个日志".to_string();
    assert!(!uses_light_execution_context_budget(
        &clarify,
        &clarify.resolved_intent
    ));
}

#[test]
fn light_execution_budget_skips_unscoped_current_workspace_drafting_evidence() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "Write a short setup note grounded in the current workspace docs".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_hint.clear();

    assert!(!uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}

#[test]
fn light_execution_budget_skips_generic_current_workspace_hint_drafting_evidence() {
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "Write a short RustClaw setup note for the current workspace project".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.locator_hint = "rustclaw workspace".to_string();

    assert!(!uses_light_execution_context_budget(
        &route,
        &route.resolved_intent
    ));
}
