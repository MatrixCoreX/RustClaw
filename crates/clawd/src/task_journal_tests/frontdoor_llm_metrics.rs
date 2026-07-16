use serde_json::Value;

use super::*;

#[test]
fn summary_reports_ordered_pre_planner_llm_metrics_without_prompt_text() {
    let mut journal = TaskJournal::for_task("task-frontdoor", "ask", "inspect workspace");
    journal.record_llm_call_sequence(vec![
        crate::LlmCallSequenceEntry {
            call_index: 1,
            prompt_label: "normalizer".to_string(),
            prompt_bytes: 1_200,
        },
        crate::LlmCallSequenceEntry {
            call_index: 2,
            prompt_label: "contract_repair".to_string(),
            prompt_bytes: 800,
        },
        crate::LlmCallSequenceEntry {
            call_index: 3,
            prompt_label: "plan".to_string(),
            prompt_bytes: 4_000,
        },
    ]);

    let summary = journal.to_summary_json();
    let frontdoor = summary
        .pointer("/task_metrics/frontdoor_llm")
        .and_then(Value::as_object)
        .expect("frontdoor metrics");

    assert_eq!(
        frontdoor.get("first_prompt_label").and_then(Value::as_str),
        Some("normalizer")
    );
    assert_eq!(
        frontdoor
            .get("first_planner_call_index")
            .and_then(Value::as_u64),
        Some(3)
    );
    assert_eq!(
        frontdoor
            .get("pre_planner_llm_calls")
            .and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        frontdoor
            .get("pre_planner_prompt_bytes")
            .and_then(Value::as_u64),
        Some(2_000)
    );
    assert_eq!(
        frontdoor
            .get("pre_planner_prompt_labels")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(2)
    );
    assert!(!serde_json::to_string(frontdoor)
        .expect("serialize frontdoor metrics")
        .contains("inspect workspace"));
}

#[test]
fn summary_reports_planner_first_target_state() {
    let mut journal = TaskJournal::for_task("task-planner-first", "ask", "hello");
    journal.record_llm_call_sequence(vec![crate::LlmCallSequenceEntry {
        call_index: 1,
        prompt_label: "plan".to_string(),
        prompt_bytes: 2_400,
    }]);

    let summary = journal.to_summary_json();
    assert_eq!(
        summary
            .pointer("/task_metrics/frontdoor_llm/pre_planner_llm_calls")
            .and_then(Value::as_u64),
        Some(0)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/frontdoor_llm/first_prompt_label")
            .and_then(Value::as_str),
        Some("plan")
    );
}
