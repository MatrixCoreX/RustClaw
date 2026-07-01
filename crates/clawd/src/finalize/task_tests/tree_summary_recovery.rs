use super::super::{
    deterministic_raw_tail_read_failure_recovery,
    task_tree_summary_recovery::deterministic_tree_summary_rows_failure_recovery,
};

fn raw_tail_recovery_fixture(
    output: serde_json::Value,
) -> (
    crate::AppState,
    crate::ClaimedTask,
    crate::RouteResult,
    crate::task_journal::TaskJournal,
) {
    let state = crate::AppState::test_default_with_fixture_provider();
    let task = crate::ClaimedTask {
        task_id: "task-raw-tail-text-boundary".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = super::route_result(crate::AskMode::planner_execute_plain());
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RawCommandOutput;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/workspace/logs/clawd-dev.log".to_string();
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-raw-tail-text-boundary", "ask", "prompt");
    journal.record_finalizer_summary(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        contract_ok: true,
        used_evidence_ids_count: 1,
        ..Default::default()
    });
    journal.push_step_result(&crate::executor::StepExecutionResult {
        step_id: "step_1".to_string(),
        skill: "fs_basic".to_string(),
        status: crate::executor::StepExecutionStatus::Ok,
        output: Some(output.to_string()),
        error: None,
        started_at: 1,
        finished_at: 2,
    });
    (state, task, route, journal)
}

#[test]
fn tree_summary_recovery_uses_extra_not_text_payload() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-tree-summary-extra", "ask", "");
    journal.record_final_stop_signal("synthesize_answer_failed");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "system_basic",
            r#"{"extra":{"action":"tree_summary","summary_rows":[{"kind":"dir","name":"docs","file_count":2,"truncated":false}]},"text":"{\"action\":\"tree_summary\",\"summary_rows\":[{\"kind\":\"dir\",\"name\":\"must_not_parse_text\",\"file_count\":99}]}"}"#,
        ));

    let recovered =
        deterministic_tree_summary_rows_failure_recovery(&journal).expect("tree summary recovery");

    assert_eq!(recovered, "name=docs file_count=2 truncated=false");
}

#[test]
fn raw_tail_recovery_ignores_json_hidden_in_visible_text() {
    let hidden = serde_json::json!({
        "action": "read_range",
        "mode": "tail",
        "requested_n": 2,
        "excerpt": "98|hidden warning\n99|hidden error"
    })
    .to_string();
    let (state, task, route, journal) = raw_tail_recovery_fixture(serde_json::json!({
        "text": hidden
    }));

    assert_eq!(
        deterministic_raw_tail_read_failure_recovery(
            &state,
            &task,
            "read tail lines",
            &route,
            &journal,
        ),
        None
    );
}

#[test]
fn raw_tail_recovery_accepts_extra_machine_payload() {
    let (state, task, route, journal) = raw_tail_recovery_fixture(serde_json::json!({
        "extra": {
            "action": "read_range",
            "mode": "tail",
            "requested_n": 2,
            "excerpt": "98|visible machine warning\n99|visible machine error"
        },
        "text": "display only"
    }));

    assert_eq!(
        deterministic_raw_tail_read_failure_recovery(
            &state,
            &task,
            "read tail lines",
            &route,
            &journal,
        )
        .as_deref(),
        Some("visible machine warning\nvisible machine error")
    );
}
