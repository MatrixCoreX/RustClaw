use super::*;

fn waiting(status_code: &str) -> TaskProviderBlocker {
    TaskProviderBlocker {
        provider: "fixture-provider".into(),
        status_code: status_code.into(),
        retry_after_seconds: 60,
        external_provider_blocked: true,
        message_key: "provider.rate_limited".into(),
    }
}

#[test]
fn quota_outcome_is_terminal_task_scoped_and_cleared_only_by_task_cleanup() {
    let state = AppState::test_default_with_fixture_provider();
    state.note_task_provider_quota_exhausted("quota-task", "fixture-provider".into());
    state.note_task_provider_blocker("other-task", waiting("rate_limited"));
    assert!(state.task_provider_blocker("quota-task").is_none());
    assert!(state.task_provider_quota_exhausted("other-task").is_none());
    state.clear_task_provider_blocker("quota-task");
    state.note_task_provider_blocker("quota-task", waiting("timeout"));
    assert_eq!(
        state.task_provider_quota_exhausted("quota-task").as_deref(),
        Some("fixture-provider")
    );
    assert!(state.task_provider_blocker("quota-task").is_none());
    state.clear_task_llm_call_count("quota-task");
    assert!(state.task_provider_quota_exhausted("quota-task").is_none());
    assert!(state.task_provider_blocker("other-task").is_some());
}

#[test]
fn historical_quota_blocker_input_cannot_reintroduce_waiting() {
    let state = AppState::test_default_with_fixture_provider();
    state.note_task_provider_blocker("quota-task", waiting("quota_exhausted"));
    assert!(state.task_provider_quota_exhausted("quota-task").is_some());
    assert!(state.task_provider_blocker("quota-task").is_none());
    state.clear_task_provider_outcome("quota-task");
    state.note_task_provider_blocker("quota-task", waiting("rate_limited"));
    assert!(state.task_provider_blocker("quota-task").is_some());
}

#[test]
fn resource_capacity_refusal_is_not_an_api_quota_failure() {
    let state = AppState::test_default_with_fixture_provider();
    let mut blocker = waiting("quota_exhausted");
    blocker.provider = "resource_broker".into();
    blocker.external_provider_blocked = false;
    state.note_task_provider_blocker("resource-task", blocker);
    assert!(state
        .task_provider_quota_exhausted("resource-task")
        .is_none());
    assert!(state.task_provider_blocker("resource-task").is_some());
}
