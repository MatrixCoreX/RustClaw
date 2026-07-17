use crate::providers::client::LlmUsageSnapshot;
use crate::providers::LlmCallCostRecord;
use crate::{AppState, ClaimedTask};

fn task(task_id: &str, user_id: i64) -> ClaimedTask {
    ClaimedTask {
        task_id: task_id.to_string(),
        user_id,
        chat_id: 8,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    }
}

fn known_record(call_index: u64, provider: &str, cost_usd_nanos: u64) -> LlmCallCostRecord {
    LlmCallCostRecord {
        logical_call_index: call_index,
        prompt_label: "plan".to_string(),
        provider: provider.to_string(),
        model: "model-a".to_string(),
        provider_status: "ok".to_string(),
        provider_attempts: 1,
        usage: Some(LlmUsageSnapshot {
            prompt_tokens: Some(100),
            completion_tokens: Some(20),
            total_tokens: Some(120),
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            cached_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        }),
        cost_status: "known".to_string(),
        unknown_reason: None,
        estimated_cost_usd_nanos: Some(cost_usd_nanos),
        pricing_effective_from: Some("2026-07-18".to_string()),
        pricing_source: None,
        pricing_currency: Some("USD".to_string()),
    }
}

fn governed_state() -> AppState {
    let mut state = AppState::test_default_with_fixture_provider();
    state.policy.llm_cost_governance.enabled = true;
    state.policy.llm_cost_governance.soft_task_usd = Some(0.50);
    state.policy.llm_cost_governance.soft_user_24h_usd = Some(1.0);
    state.policy.llm_cost_governance.soft_provider_24h_usd = Some(2.0);
    state.policy.llm_cost_governance.hard_task_usd = Some(5.0);
    state
}

#[test]
fn durable_ledger_restores_task_cost_and_logical_call_index_after_memory_clear() {
    let state = governed_state();
    let task = task("task-cost-resume", 7);
    state.note_task_llm_call_with_label_and_prompt_size(&task.task_id, "plan", 10);
    state
        .note_task_llm_cost_record(&task, known_record(1, "vendor-primary", 600_000_000))
        .expect("persist cost record");
    state.clear_task_llm_call_count(&task.task_id);

    assert_eq!(state.task_llm_call_count(&task.task_id), 0);
    state.restore_task_llm_call_count_from_cost_ledger(&task.task_id);
    assert_eq!(state.task_llm_call_count(&task.task_id), 1);
    assert_eq!(
        state
            .task_llm_cost_summary(&task.task_id)
            .estimated_cost_usd_nanos,
        600_000_000
    );
}

#[test]
fn soft_budgets_emit_task_user_and_provider_machine_signals() {
    let state = governed_state();
    let first = task("task-cost-soft-a", 7);
    let second = task("task-cost-soft-b", 7);
    state
        .note_task_llm_cost_record(&first, known_record(1, "vendor-primary", 1_200_000_000))
        .expect("persist first cost");
    state
        .note_task_llm_cost_record(&second, known_record(1, "vendor-primary", 1_100_000_000))
        .expect("persist second cost");

    let snapshot = state
        .evaluate_llm_cost_budget(&second, Some("vendor-primary"))
        .expect("evaluate budget");

    assert_eq!(snapshot.status, "soft_exceeded");
    assert!(snapshot
        .signals
        .iter()
        .any(|signal| signal == "soft_task_cost_exceeded"));
    assert!(snapshot
        .signals
        .iter()
        .any(|signal| signal == "soft_user_24h_cost_exceeded"));
    assert!(snapshot
        .signals
        .iter()
        .any(|signal| signal == "soft_provider_24h_cost_exceeded"));
    assert!(!snapshot.hard_exceeded);
}

#[test]
fn hard_task_ceiling_sets_policy_blocker_without_discarding_ledger() {
    let state = governed_state();
    let task = task("task-cost-hard", 9);
    state
        .note_task_llm_cost_record(&task, known_record(1, "vendor-primary", 5_500_000_000))
        .expect("persist hard-limit cost");

    let snapshot = state
        .evaluate_llm_cost_budget(&task, Some("vendor-primary"))
        .expect("evaluate hard budget");
    let blocker = state
        .task_cost_blocker(&task.task_id)
        .expect("cost blocker");

    assert_eq!(snapshot.status, "hard_exceeded");
    assert!(snapshot.hard_exceeded);
    assert_eq!(blocker.status_code, "llm_cost_hard_ceiling");
    assert_eq!(blocker.observed_cost_usd_nanos, 5_500_000_000);
    assert_eq!(state.task_llm_cost_records(&task.task_id).len(), 1);
}
