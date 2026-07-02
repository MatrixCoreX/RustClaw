use super::*;
use serde_json::json;

fn route_with_capability_ref(capability_ref: &str) -> crate::RouteResult {
    crate::RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: capability_ref.to_string(),
        needs_clarify: false,
        clarify_question: String::new(),
        route_reason: capability_ref.to_string(),
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
            response_shape: crate::OutputResponseShape::Strict,
            requires_content_evidence: true,
            semantic_kind: crate::OutputSemanticKind::None,
            ..Default::default()
        },
    }
}

#[test]
fn service_status_capability_refs_complete_field_value_evidence_without_semantic_kind() {
    for capability_ref in [
        "capability_ref=service.status",
        "capability_ref=service_control.status",
        "capability_ref=system.runtime_status",
        "capability_ref=system.health_check",
    ] {
        let route = route_with_capability_ref(capability_ref);
        let mut journal = TaskJournal::for_task("task-service-evidence", "ask", capability_ref);
        journal.record_route_result(&route);
        journal.push_task_observation(json!({
            "observed_evidence": {
                "extractor": {
                    "extractor_ref": "service_status.machine_status_v1"
                },
                "items": [
                    {
                        "field": "status",
                        "excerpt": "ok"
                    }
                ]
            }
        }));

        let coverage = evidence_coverage_for_route(&route, &journal);

        assert!(coverage.is_complete(), "{capability_ref}: {coverage:?}");
        assert!(
            coverage.observed_canonical.contains("field_value"),
            "{capability_ref}: {coverage:?}"
        );
    }
}
