use serde_json::Value;

use super::TaskJournal;

#[test]
fn answer_verifier_summary_serializes_repair_envelope() {
    let mut journal = TaskJournal::for_task("task-answer-verifier-envelope", "ask", "inspect");
    journal.answer_verifier_summary = Some(crate::task_journal::TaskJournalAnswerVerifierSummary {
        pass: false,
        missing_evidence_fields: vec!["content_excerpt".to_string(), "path".to_string()],
        answer_incomplete_reason: "model_omitted_required_evidence".to_string(),
        should_retry: true,
        retry_instruction: "collect_required_evidence_fields:content_excerpt,path".to_string(),
        confidence: 0.92,
    });

    let summary = journal.to_summary_json();
    assert_eq!(
        summary
            .pointer("/answer_verifier_summary/repair_signal/source")
            .and_then(Value::as_str),
        Some("answer_verifier")
    );
    assert_eq!(
        summary
            .pointer("/answer_verifier_summary/repair_signal/repair_class")
            .and_then(Value::as_str),
        Some("loop_bounded_recovery")
    );
    assert_eq!(
        summary
            .pointer("/answer_verifier_summary/repair_signal/repair_envelope/issue_codes/0")
            .and_then(Value::as_str),
        Some("answer_verifier_missing_evidence_repair")
    );
    assert_eq!(
        summary
            .pointer("/answer_verifier_summary/repair_signal/repair_envelope/missing_evidence/1")
            .and_then(Value::as_str),
        Some("path")
    );

    let trace = journal.to_trace_json();
    assert_eq!(
        trace
            .pointer("/answer_verifier_summary/repair_signal/next_recovery_kind")
            .and_then(Value::as_str),
        Some("replan")
    );
}
