use super::*;
use serde_json::json;

fn unclassified_service_evidence_contract() -> crate::IntentOutputContract {
    crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: crate::OutputSemanticKind::None,
        ..Default::default()
    }
}

#[test]
fn service_capability_refs_complete_machine_field_evidence_without_semantic_kind() {
    for capability_ref in [
        "capability_ref=service.status",
        "capability_ref=service_control.status",
        "capability_ref=system.runtime_status",
        "capability_ref=system.health_check",
    ] {
        let route = unclassified_service_evidence_contract();
        let mut journal = TaskJournal::for_task("task-service-evidence", "ask", capability_ref);
        journal.record_output_contract(&route.clone());
        journal.push_task_observation(json!({
            "observed_evidence": {
                "extractor": {
                    "extractor_ref": "capability_result.machine_field_v1"
                },
                "items": [
                    {
                        "field": "status",
                        "excerpt": "ok"
                    }
                ]
            }
        }));

        let coverage = evidence_coverage_for_output_contract(&route.clone(), &journal);

        assert!(coverage.is_complete(), "{capability_ref}: {coverage:?}");
        assert!(
            coverage.observed_canonical.contains("field_value"),
            "{capability_ref}: {coverage:?}"
        );
    }
}
