use super::bind_ordered_entry_reference_from_active_frame;
use serde_json::json;

fn route_with_context(route_reason: &str, resolved_intent: &str) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: resolved_intent.to_string(),
        needs_clarify: false,
        route_reason: route_reason.to_string(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: crate::RiskCeiling::Low,
        resume_behavior: crate::ResumeBehavior::None,
        schedule_kind: crate::ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: crate::IntentOutputContract {
            exact_sentence_count: None,
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: crate::OutputLocatorKind::None,
            delivery_intent: crate::OutputDeliveryIntent::None,
            semantic_kind: crate::OutputSemanticKind::None,
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

fn snapshot_with_ordered_entry(entry: &str) -> crate::conversation_state::ActiveSessionSnapshot {
    crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/rustclaw_ordered_marker".to_string()),
            ordered_entries: vec![entry.to_string()],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    }
}

fn ordered_entry_analysis() -> crate::intent_router::TurnAnalysis {
    crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: Some(json!({
            "ordered_entry_ref": {
                "index": 1,
                "index_base": 1
            }
        })),
        attachment_processing_required: false,
    }
}

#[test]
fn ordered_entry_binding_uses_exact_machine_markers_and_target_lines() {
    let target = "/tmp/rustclaw_ordered_marker/selected.log";
    let mut route = route_with_context(
        concat!(
            "ordered_entry_reference_bound_from_active_frame_extra; ",
            "prefix:ordered_entry_reference_inferred_from_current_prompt_token_extra"
        ),
        "prior note ordered_entry_target_suffix: /tmp/rustclaw_ordered_marker/selected.log",
    );
    let snapshot = snapshot_with_ordered_entry("selected.log");
    let analysis = ordered_entry_analysis();

    assert!(bind_ordered_entry_reference_from_active_frame(
        &mut route,
        &snapshot,
        Some(&analysis),
        None,
    ));

    assert_eq!(route.output_contract.locator_hint, target);
    assert!(
        route
            .route_reason
            .split(';')
            .map(str::trim)
            .any(|part| part == "ordered_entry_reference_bound_from_active_frame")
    );
    assert!(
        route
            .resolved_intent
            .lines()
            .map(str::trim)
            .any(|line| line == format!("ordered_entry_target: {target}"))
    );
    assert_eq!(
        route
            .resolved_intent
            .lines()
            .filter(|line| line.trim() == format!("ordered_entry_target: {target}"))
            .count(),
        1
    );
}

#[test]
fn ordered_entry_binding_deduplicates_exact_target_line() {
    let target = "/tmp/rustclaw_ordered_marker/selected.log";
    let mut route = route_with_context(
        "ordered_entry_reference_bound_from_active_frame",
        &format!("ordered_entry_target: {target}"),
    );
    let snapshot = snapshot_with_ordered_entry("selected.log");
    let analysis = ordered_entry_analysis();

    assert!(bind_ordered_entry_reference_from_active_frame(
        &mut route,
        &snapshot,
        Some(&analysis),
        None,
    ));

    assert_eq!(
        route
            .resolved_intent
            .lines()
            .filter(|line| line.trim() == format!("ordered_entry_target: {target}"))
            .count(),
        1
    );
}
