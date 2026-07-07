use super::{
    apply_structured_anchor_evidence_repair, structured_anchor_route_requires_evidence_repair,
};

fn executable_filename_route() -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::act_with_chat_finalizer(),
        resolved_intent: "read README and summarize".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Unknown,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            locator_kind: crate::OutputLocatorKind::Filename,
            locator_hint: "README.md".to_string(),
            requires_content_evidence: true,
            ..Default::default()
        },
    }
}

fn listed_entry_snapshot() -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("list document entries".to_string()),
            last_primary_task_output: Some("hello.sh".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list document entries".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("document".to_string()),
            ordered_entries: vec!["hello.sh".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

#[test]
fn structured_anchor_route_with_derived_candidate_requires_evidence() {
    let snapshot = listed_entry_snapshot();
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = concat!(
        "User wants path and type for the observed entry hello.sh.\n",
        "answer_candidate: {\"path\":\"/tmp/hello.sh\",\"type\":\"file\"}"
    )
    .to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(structured_anchor_route_requires_evidence_repair(
        "What is the path and type for that entry?",
        &route,
        &snapshot,
        "<none>",
        true,
        None
    ));

    apply_structured_anchor_evidence_repair(&mut route);
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
}

#[test]
fn structured_anchor_route_with_exact_observed_candidate_requires_evidence() {
    let snapshot = listed_entry_snapshot();
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent =
        "User wants the observed entry name.\nanswer_candidate: hello.sh".to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(structured_anchor_route_requires_evidence_repair(
        "What is that entry name?",
        &route,
        &snapshot,
        "<none>",
        true,
        None
    ));
}

#[test]
fn structured_anchor_route_with_state_patch_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("list document entries".to_string()),
            last_primary_task_output: Some("hello.sh".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list document entries".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("document".to_string()),
            ordered_entries: vec!["hello.sh".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = "state_patch ack\nanswer_candidate: state_update_ack".to_string();
    route.output_contract = crate::IntentOutputContract::default();
    let turn_analysis = crate::intent_router::TurnAnalysis {
        turn_type: None,
        target_task_policy: None,
        should_interrupt_active_run: false,
        state_patch: Some(serde_json::json!({
            "alias_bindings": [{
                "alias": "note_file",
                "target": "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            }]
        })),
        attachment_processing_required: false,
    };

    assert!(!structured_anchor_route_requires_evidence_repair(
        "Update the alias binding and acknowledge.",
        &route,
        &snapshot,
        "<none>",
        true,
        Some(&turn_analysis)
    ));
}

#[test]
fn structured_anchor_route_with_resolved_target_basename_stays_chat() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("send selected log file".to_string()),
            last_primary_task_output: Some(
                "FILE:/home/guagua/rustclaw/logs/clawd-dev.log".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "send selected log file".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("/home/guagua/rustclaw/logs/clawd-dev.log".to_string()),
            ordered_entries: vec!["clawd-dev.log".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some("/home/guagua/rustclaw/logs/clawd-dev.log".to_string()),
            delivery_targets: vec!["/home/guagua/rustclaw/logs/clawd-dev.log".to_string()],
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = "answer_candidate: clawd-dev.log".to_string();
    route.output_contract = crate::IntentOutputContract::default();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(!structured_anchor_route_requires_evidence_repair(
        "Only say this file name.",
        &route,
        &snapshot,
        "<none>",
        true,
        None
    ));
}

#[test]
fn structured_anchor_route_with_prose_basename_requires_evidence() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("send selected log file".to_string()),
            last_primary_task_output: Some(
                "FILE:/home/guagua/rustclaw/logs/clawd-dev.log".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "send selected log file".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("/home/guagua/rustclaw/logs/clawd-dev.log".to_string()),
            ordered_entries: vec!["clawd-dev.log".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: Some(crate::observed_facts::ObservedFacts {
            bound_target: Some("/home/guagua/rustclaw/logs/clawd-dev.log".to_string()),
            delivery_targets: vec!["/home/guagua/rustclaw/logs/clawd-dev.log".to_string()],
            ..crate::observed_facts::ObservedFacts::default()
        }),
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = "User only wants the file basename clawd-dev.log".to_string();
    route.output_contract = crate::IntentOutputContract::default();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;

    assert!(structured_anchor_route_requires_evidence_repair(
        "Only say this file name.",
        &route,
        &snapshot,
        "<none>",
        true,
        None
    ));
}

#[test]
fn structured_anchor_route_with_recent_execution_token_candidate_requires_evidence() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read service notes opening".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some(
                "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
            ),
            source_task_id: "task-2".to_string(),
            updated_at_ts: 2,
            expires_at_ts: 3,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = concat!(
        "Compare the two most recent observed excerpts and return the selected filename.\n",
        "answer_candidate: README.md"
    )
    .to_string();
    route.output_contract = crate::IntentOutputContract::default();
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let recent_execution_context = "\
### RECENT_EXECUTION_EVENTS
- ts=3 kind=ask request=read README.md result=# RustClaw
- ts=2 kind=ask request=read scripts/nl_tests/fixtures/device_local/docs/service_notes.md result=# Service Notes";

    assert!(structured_anchor_route_requires_evidence_repair(
        "Which previous excerpt is more like a project description? Answer only the filename.",
        &route,
        &snapshot,
        recent_execution_context,
        true,
        None
    ));
}

#[test]
fn structured_anchor_route_with_existing_context_synthesis_candidate_requires_evidence() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("read README opening".to_string()),
            last_primary_task_output: Some(
                "# Device Local Fixture\n\nStable local files for regression tests.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "read README opening".to_string(),
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
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = concat!(
        "Summarize the previously displayed README opening.\n",
        "answer_candidate: This README describes stable local fixture files for regression tests."
    )
    .to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(structured_anchor_route_requires_evidence_repair(
        "Summarize it in one sentence.",
        &route,
        &snapshot,
        "<none>",
        true,
        None
    ));
}

#[test]
fn inline_json_followup_does_not_bind_to_workspace_evidence() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("transform records".to_string()),
            last_primary_task_output: Some("waiting for records".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "transform records".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("records".to_string()),
            ordered_entries: vec!["records".to_string()],
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = concat!(
        "Sort the provided JSON array by score and render a table.\n",
        "answer_candidate: | name | score |"
    )
    .to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(!structured_anchor_route_requires_evidence_repair(
        r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#,
        &route,
        &snapshot,
        "<none>",
        true,
        None
    ));
}

#[test]
fn active_text_mutation_with_structured_anchor_stays_chat() {
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
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let mut route = executable_filename_route();
    route.ask_mode = crate::AskMode::respond_trace();
    route.resolved_intent = "Clarify the current request without reading files.".to_string();
    route.output_contract = crate::IntentOutputContract::default();

    assert!(!structured_anchor_route_requires_evidence_repair(
        "A concept label without a concrete target.",
        &route,
        &snapshot,
        "<none>",
        true,
        Some(&analysis)
    ));
}
