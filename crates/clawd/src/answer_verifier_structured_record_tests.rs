use super::*;

fn schedule_preview_fixture() -> (AnswerContract, crate::task_journal::TaskJournal) {
    let output_contract = crate::IntentOutputContract {
        semantic_kind: crate::OutputSemanticKind::SchedulePreview,
        response_shape: crate::OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..Default::default()
    };
    let route = AnswerContract::new("preview", output_contract.clone());
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-schedule-answer", "ask", "preview");
    journal.record_output_contract(&output_contract);
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "schedule".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(
            serde_json::json!({
                "extra": {"action": "preview", "status": "ok"},
                "text": concat!(
                    "datetime=2026-07-19T09:00:00\n",
                    "timezone=Asia/Shanghai\n",
                    "title=Tier 2 review"
                )
            })
            .to_string(),
        ),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    (route, journal)
}

#[test]
fn schedule_preview_answer_accepts_exact_observed_machine_record() {
    let (route, journal) = schedule_preview_fixture();
    let candidate = "datetime=2026-07-19T09:00:00\ntimezone=Asia/Shanghai\ntitle=Tier 2 review";

    assert!(schedule_preview_answer_is_grounded_in_observation(
        &route, &journal, candidate
    ));
    assert!(local_structured_record_answer_verifier_gap(&route, &journal, candidate).is_none());
}

#[test]
fn schedule_preview_answer_rejects_unobserved_datetime() {
    let (route, journal) = schedule_preview_fixture();
    let candidate = "datetime=2026-07-18T09:00:00\ntimezone=Asia/Shanghai\ntitle=Tier 2 review";

    let gap = local_structured_record_answer_verifier_gap(&route, &journal, candidate)
        .expect("unobserved date must be rejected");
    assert_eq!(gap.missing_evidence_fields, vec!["datetime"]);
    assert!(gap.should_retry);
}

#[test]
fn schedule_preview_answer_rejects_extra_model_prose() {
    let (route, journal) = schedule_preview_fixture();
    let candidate = concat!(
        "datetime=2026-07-19T09:00:00\n",
        "timezone=Asia/Shanghai\n",
        "title=Tier 2 review\n",
        "additional model narrative"
    );

    assert!(!schedule_preview_answer_is_grounded_in_observation(
        &route, &journal, candidate
    ));
    assert!(local_structured_record_answer_verifier_gap(&route, &journal, candidate).is_some());
}
