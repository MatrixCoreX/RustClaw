use super::*;

#[test]
fn boundary_envelope_projects_only_machine_boundary_fields() {
    let schedule_intent = crate::ScheduleIntentOutput {
        kind: "create".to_string(),
        ..Default::default()
    };
    let output = IntentNormalizerOutput {
        raw_user_request: "check config/app.toml later".to_string(),
        resolved_user_intent: "legacy trace text should not matter".to_string(),
        resume_behavior: crate::ResumeBehavior::ResumeExecute,
        schedule_kind: crate::ScheduleKind::Create,
        schedule_intent: Some(schedule_intent.clone()),
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        needs_clarify: false,
        clarify_question: String::new(),
        reason: String::new(),
        confidence: 0.8,
        output_contract: crate::IntentOutputContract {
            locator_kind: crate::OutputLocatorKind::Path,
            locator_hint: "config/app.toml".to_string(),
            ..Default::default()
        },
        execution_recipe_hint: None,
        execution_recipe_plan_hint: None,
        route_trace_decision: crate::FirstLayerDecision::DirectAnswer,
        execution_finalize_style: crate::ActFinalizeStyle::ChatWrapped,
        turn_analysis: Some(TurnAnalysis {
            turn_type: Some(TurnType::TaskRequest),
            target_task_policy: Some(TargetTaskPolicy::ReuseActive),
            should_interrupt_active_run: false,
            state_patch: None,
            attachment_processing_required: true,
        }),
        attachment_processing_required: true,
        fallback_source: None,
        route_trace_record: route_trace::RouteTraceRecord {
            owner_layer: "normalizer_trace",
            reason_code: "test",
            outcome: "ok",
            route_trace_decision: crate::FirstLayerDecision::DirectAnswer,
            needs_clarify: false,
            output_contract_ref: "none".to_string(),
            repair_codes: Vec::new(),
            repair_classes: Vec::new(),
        },
    };

    let envelope = output.boundary_envelope();

    assert_eq!(envelope.raw_user_request, "check config/app.toml later");
    assert_eq!(envelope.explicit_locators, vec!["config/app.toml"]);
    assert_eq!(
        envelope.attachment_refs,
        vec!["current_request_attachments"]
    );
    assert_eq!(envelope.session_binding.as_deref(), Some("resume_execute"));
    assert_eq!(
        envelope.active_task_reference.as_deref(),
        Some("reuse_active")
    );
    assert_eq!(
        envelope
            .schedule_intent
            .as_ref()
            .map(|intent| intent.kind.as_str()),
        Some(schedule_intent.kind.as_str())
    );
    assert_eq!(envelope.language_hint.as_deref(), Some("en"));
    assert!(envelope.safety_budget_hint.is_none());
    let prompt_line = envelope.compact_prompt_line();
    assert!(prompt_line.contains("raw_chars=27"));
    assert!(prompt_line.contains("schedule_intent=create"));
    assert!(prompt_line.contains("attachment_refs=1"));
    assert!(prompt_line.contains("explicit_locators=1"));
    assert!(prompt_line.contains("active_task_reference=reuse_active"));
    assert!(prompt_line.contains("session_binding=resume_execute"));
    assert!(prompt_line.contains("language_hint=en"));
    assert!(!prompt_line.contains("check config/app.toml later"));
    assert!(!prompt_line.contains("config/app.toml"));
}
