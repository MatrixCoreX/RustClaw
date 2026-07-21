use crate::providers::{CircuitBreakerSnapshot, LlmProviderRouteEvaluation};
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

#[test]
fn journal_projects_provider_route_candidates_and_machine_exclusions() {
    let state = AppState::test_default_with_fixture_provider();
    let task_id = "task-provider-routing-plan";
    state.note_task_llm_call_with_label_and_prompt_size(task_id, "plan", 64);
    state.note_task_provider_routing_plan_with_label(
        task_id,
        "plan",
        vec![
            LlmProviderRouteEvaluation {
                provider: "vendor-primary".to_string(),
                model: "model-a".to_string(),
                eligible: true,
                exclusion_codes: Vec::new(),
                rank: Some(1),
                breaker_state: "closed".to_string(),
                required_context_window_tokens: 4_096,
                estimated_prompt_tokens: 16,
                prompt_token_estimator: "generic_unicode_estimate_v1".to_string(),
                prompt_byte_count: 64,
                prompt_char_count: 64,
                context_window_tokens: Some(1_000_000),
                input_modalities: vec!["text".to_string(), "image".to_string()],
                native_tools: true,
                latency_sample_count: 2,
                routing_latency_ms: 120,
                price_score_microusd_per_million: Some(500_000),
                static_priority: 1,
            },
            LlmProviderRouteEvaluation {
                provider: "vendor-secondary".to_string(),
                model: "model-b".to_string(),
                eligible: false,
                exclusion_codes: vec!["required_input_modality_unsupported".to_string()],
                rank: None,
                breaker_state: "closed".to_string(),
                required_context_window_tokens: 4_096,
                estimated_prompt_tokens: 16,
                prompt_token_estimator: "generic_unicode_estimate_v1".to_string(),
                prompt_byte_count: 64,
                prompt_char_count: 64,
                context_window_tokens: Some(1_000_000),
                input_modalities: vec!["text".to_string()],
                native_tools: true,
                latency_sample_count: 0,
                routing_latency_ms: 500,
                price_score_microusd_per_million: None,
                static_priority: 2,
            },
        ],
    );

    let mut journal = TaskJournal::for_task(task_id, "ask", "opaque");
    journal.record_runtime_llm_metrics(&state, task_id);
    let summary = journal.to_summary_json();

    assert_eq!(
        summary
            .pointer("/task_metrics/by_prompt/plan/provider_route_evaluations/0/provider")
            .and_then(serde_json::Value::as_str),
        Some("vendor-primary")
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/by_prompt/plan/provider_route_evaluations/0/rank")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    assert_eq!(
        summary
            .pointer("/task_metrics/by_prompt/plan/provider_route_evaluations/1/exclusion_codes/0",)
            .and_then(serde_json::Value::as_str),
        Some("required_input_modality_unsupported")
    );
}
