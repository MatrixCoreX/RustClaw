use crate::providers::client::LlmUsageSnapshot;
use crate::providers::LlmCallCostRecord;
use crate::AppState;

use super::TaskJournal;

fn known_record(call_index: u64) -> LlmCallCostRecord {
    LlmCallCostRecord {
        logical_call_index: call_index,
        prompt_label: "plan".to_string(),
        provider: "vendor-minimax".to_string(),
        model: "MiniMax-M3".to_string(),
        provider_status: "ok".to_string(),
        provider_attempts: 1,
        usage: Some(LlmUsageSnapshot {
            prompt_tokens: Some(1_000),
            completion_tokens: Some(200),
            total_tokens: Some(1_200),
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: Some(50),
            cached_tokens: Some(100),
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }),
        cost_status: "known".to_string(),
        unknown_reason: None,
        estimated_cost_usd_nanos: Some(516_000),
        pricing_effective_from: Some("2026-07-18".to_string()),
        pricing_source: Some("https://example.invalid/pricing".to_string()),
        pricing_currency: Some("USD".to_string()),
    }
}

#[test]
fn journal_projects_per_call_usage_and_task_cost_without_request_content() {
    let state = AppState::test_default_with_fixture_provider();
    let task_id = "task-cost-known";
    state.note_task_llm_call_with_label_and_prompt_size(task_id, "plan", 123);
    state.note_task_llm_cost_record(task_id, known_record(1));

    let mut journal = TaskJournal::for_task(task_id, "ask", "opaque user input");
    journal.record_runtime_llm_metrics(&state, task_id);
    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/task_metrics/llm_cost/status")
            .and_then(serde_json::Value::as_str),
        Some("known")
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/llm_cost/estimated_cost_usd_nanos")
            .and_then(serde_json::Value::as_u64),
        Some(516_000)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/llm_cost_records/0/usage/prompt_tokens")
            .and_then(serde_json::Value::as_u64),
        Some(1_000)
    );
    let serialized = summary
        .get("task_metrics")
        .expect("task metrics")
        .to_string();
    assert!(!serialized.contains("opaque user input"));
    assert!(!serialized.contains("api_key"));
}

#[test]
fn journal_marks_unobserved_llm_call_cost_unknown_and_clear_removes_records() {
    let state = AppState::test_default_with_fixture_provider();
    let task_id = "task-cost-unknown";
    state.note_task_llm_call_with_label_and_prompt_size(task_id, "chat", 10);
    let summary = state.task_llm_cost_summary(task_id);
    assert_eq!(summary.status, "unknown");
    assert_eq!(summary.unknown_reasons, vec!["logical_call_not_observed"]);

    state.note_task_llm_cost_record(task_id, known_record(1));
    assert_eq!(state.task_llm_cost_records(task_id).len(), 1);
    state.clear_task_llm_call_count(task_id);
    assert!(state.task_llm_cost_records(task_id).is_empty());
}
