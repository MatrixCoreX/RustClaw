use super::*;

fn structured_selector_route() -> crate::IntentOutputContract {
    crate::IntentOutputContract {
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        selection: crate::pipeline_types::OutputSelectionContract {
            structured_field_selector: Some("datetime,timezone,title".to_string()),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn push_structured_preview_step(journal: &mut TaskJournal, preview_fields: Value) {
    let mut extra = preview_fields.as_object().cloned().unwrap_or_default();
    extra.insert("action".to_string(), json!("preview"));
    extra.insert("status".to_string(), json!("ok"));
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "schedule".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            json!({
                "extra": extra,
                "text": "preview ready"
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
}

#[test]
fn structured_selector_requires_all_machine_fields_from_observation() {
    let route = structured_selector_route();
    let mut journal = TaskJournal::for_task("task-structured-selector", "ask", "preview");
    journal.record_output_contract(&route);
    push_structured_preview_step(
        &mut journal,
        json!({
            "datetime": "2026-07-19T09:00:00",
            "timezone": "Asia/Shanghai",
            "title": "Tier 2 review"
        }),
    );

    let coverage = evidence_coverage_for_output_contract(&route, &journal);
    assert!(coverage.is_complete(), "coverage: {coverage:?}");
    for field in ["datetime", "timezone", "title"] {
        assert!(
            coverage.observed_canonical.contains(field),
            "missing {field}: {coverage:?}"
        );
    }
}

#[test]
fn structured_selector_rejects_partial_machine_evidence() {
    let route = structured_selector_route();
    let mut journal = TaskJournal::for_task("task-structured-selector-partial", "ask", "preview");
    journal.record_output_contract(&route);
    push_structured_preview_step(
        &mut journal,
        json!({
            "timezone": "Asia/Shanghai",
            "title": "Tier 2 review"
        }),
    );

    let coverage = evidence_coverage_for_output_contract(&route, &journal);
    assert!(!coverage.is_complete());
    assert_eq!(coverage.missing_evidence, vec!["datetime"]);
}
