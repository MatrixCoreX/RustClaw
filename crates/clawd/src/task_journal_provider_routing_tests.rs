use crate::providers::CircuitBreakerSnapshot;
use crate::AppState;

use super::TaskJournal;

fn snapshot(state: &str, failures: u32, remaining_cooldown_ms: u64) -> CircuitBreakerSnapshot {
    CircuitBreakerSnapshot {
        state: state.to_string(),
        consecutive_failures: failures,
        current_cooldown_ms: 60_000,
        remaining_cooldown_ms,
    }
}

#[test]
fn journal_projects_provider_fallback_amplification_and_breaker_state() {
    let state = AppState::test_default_with_fixture_provider();
    let task_id = "task-provider-routing";
    state.note_task_llm_call_with_label_and_prompt_size(task_id, "plan", 64);
    state.note_task_provider_route_with_label(
        task_id,
        "plan",
        "vendor-primary:model-a",
        true,
        false,
        false,
        false,
        snapshot("closed", 0, 0),
    );
    state.note_task_provider_attempts_with_label(
        task_id,
        "plan",
        2,
        1,
        Some("network"),
        Some("network"),
    );
    state.note_task_provider_breaker_snapshot_with_label(
        task_id,
        "plan",
        "vendor-primary:model-a",
        snapshot("open", 3, 59_000),
    );
    state.note_task_provider_route_with_label(
        task_id,
        "plan",
        "vendor-secondary:model-b",
        true,
        true,
        false,
        false,
        snapshot("closed", 0, 0),
    );
    state.note_task_provider_attempts_with_label(task_id, "plan", 1, 0, None, None);

    let mut journal = TaskJournal::for_task(task_id, "ask", "opaque");
    journal.record_runtime_llm_metrics(&state, task_id);
    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/task_metrics/provider_routing/provider_selections")
            .and_then(serde_json::Value::as_u64),
        Some(2)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/provider_routing/provider_fallbacks")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/provider_routing/provider_retries")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/provider_routing/fallback_amplification_millis")
            .and_then(serde_json::Value::as_u64),
        Some(2_000)
    );
    assert_eq!(
        summary
            .pointer(
                "/task_metrics/by_prompt/plan/provider_breaker_snapshots/vendor-primary:model-a/state",
            )
            .and_then(serde_json::Value::as_str),
        Some("open")
    );
}

#[test]
fn journal_projects_circuit_skip_without_counting_provider_selection() {
    let state = AppState::test_default_with_fixture_provider();
    let task_id = "task-provider-circuit-skip";
    state.note_task_llm_call_with_label_and_prompt_size(task_id, "plan", 64);
    state.note_task_provider_route_with_label(
        task_id,
        "plan",
        "vendor-primary:model-a",
        false,
        false,
        true,
        false,
        snapshot("open", 3, 42_000),
    );

    let mut journal = TaskJournal::for_task(task_id, "ask", "opaque");
    journal.record_runtime_llm_metrics(&state, task_id);
    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/task_metrics/provider_routing/provider_selections")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/provider_routing/circuit_skips")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/provider_routing/fallback_amplification_millis")
            .and_then(serde_json::Value::as_u64),
        Some(0)
    );
}
