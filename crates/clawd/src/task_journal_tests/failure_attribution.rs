use super::*;

#[test]
fn trace_json_infers_failure_attribution_from_standard_error_kind() {
    for (error_kind, expected) in [
        ("schema_validation_failed", "schema_error"),
        ("provider_retryable_response", "provider_error"),
        ("channel_send_failed", "delivery_error"),
    ] {
        let mut journal = TaskJournal::for_task(
            format!("task-{error_kind}"),
            "ask",
            "trigger structured error",
        );
        let err = crate::skills::structured_skill_error_from_parts(
            "runtime",
            error_kind,
            "structured failure",
            None,
            None,
        );
        journal.push_step_result(&crate::executor::StepExecutionResult {
            step_id: "step_1".to_string(),
            skill: "runtime".to_string(),
            status: crate::executor::StepExecutionStatus::Error,
            output: None,
            error: Some(err),
            started_at: 1,
            finished_at: 1,
        });

        let trace = journal.to_trace_json();
        let step = trace
            .get("step_results")
            .and_then(Value::as_array)
            .and_then(|steps| steps.first())
            .expect("step result should be present");

        assert_eq!(
            step.get("error_kind").and_then(Value::as_str),
            Some(error_kind)
        );
        assert_eq!(
            step.get("failure_attribution").and_then(Value::as_str),
            Some(expected)
        );
    }
}

#[test]
fn final_error_text_records_failure_attribution() {
    for (error_text, expected) in [
        (
            "provider=minimax failed: timeout while reading response",
            "provider_error",
        ),
        (
            "direct_answer_gate schema_validation_failed task_id=t1 err=missing field",
            "schema_error",
        ),
        (
            "wechat send status=500 body={\"err\":\"bad gateway\"}",
            "delivery_error",
        ),
        (
            r#"{"message_key":"answer_verifier_required_evidence_block","reason_code":"answer_verifier_required_evidence_block","missing_evidence_fields":["field_value"]}"#,
            "contract_gap",
        ),
    ] {
        let mut journal =
            TaskJournal::for_task(format!("task-{expected}"), "ask", "trigger final error");
        journal.record_final_failure_attribution_from_error(error_text);
        journal.record_final_status(TaskJournalFinalStatus::Failure);

        assert_eq!(
            journal
                .to_trace_json()
                .get("final_failure_attribution")
                .and_then(Value::as_str),
            Some(expected)
        );
    }
}

#[test]
fn rollout_attribution_serializes_machine_fields() {
    let mut journal = TaskJournal::for_task("task-rollout-attribution", "ask", "读取字段");
    let verifier = crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["field_value".to_string()],
        answer_incomplete_reason: "human-language reason should stay out of attribution"
            .to_string(),
        should_retry: true,
        retry_instruction: String::new(),
        confidence: 0.91,
    };
    journal.record_rollout_attribution(
        TaskJournalRolloutAttribution::answer_verifier_required_evidence_block(Some(&verifier)),
    );
    journal.record_rollout_attribution(
        TaskJournalRolloutAttribution::answer_verifier_required_evidence_block(Some(&verifier)),
    );

    let summary = journal.to_summary_json();
    let attribution = summary
        .get("rollout_attribution")
        .and_then(Value::as_array)
        .expect("rollout attribution should be present");
    assert_eq!(attribution.len(), 1);
    let item = &attribution[0];
    assert_eq!(
        item.get("switch_name").and_then(Value::as_str),
        Some("answer_verifier_enforce_required_scope")
    );
    assert_eq!(
        item.get("event").and_then(Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(item.get("outcome").and_then(Value::as_str), Some("blocked"));
    assert_eq!(
        item.get("reason_code").and_then(Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        item.get("failure_attribution").and_then(Value::as_str),
        Some("contract_gap")
    );
    assert_eq!(
        item.get("missing_evidence_fields")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("field_value")
    );
    assert_eq!(item.get("confidence").and_then(Value::as_f64), Some(0.91));
    assert_eq!(
        item.pointer("/boundary_context/decision_source")
            .and_then(Value::as_str),
        Some("contract_boundary")
    );
    assert_eq!(
        item.pointer("/boundary_context/rewrite_reason_code")
            .and_then(Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
    assert_eq!(
        item.pointer("/boundary_context/semantic_control_state")
            .and_then(Value::as_str),
        Some("none")
    );
    assert_eq!(
        item.pointer("/boundary_context/input_contract_ref")
            .and_then(Value::as_str),
        Some("answer_verifier_summary")
    );
    assert_eq!(
        item.pointer("/boundary_context/output_contract_ref")
            .and_then(Value::as_str),
        Some("required_evidence_contract")
    );
    assert!(
        serde_json::to_string(item)
            .expect("serialize attribution")
            .contains("human-language reason")
            == false
    );

    assert_eq!(
        journal
            .to_trace_json()
            .pointer("/rollout_attribution/0/reason_code")
            .and_then(Value::as_str),
        Some("answer_verifier_required_evidence_block")
    );
}
