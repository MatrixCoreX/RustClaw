use crate::{AppState, ClaimedTask, TaskCostBlocker};

use super::record_cost_wait_checkpoint;

#[test]
fn hard_cost_ceiling_writes_resumable_policy_checkpoint() {
    let state = AppState::test_default_with_fixture_provider();
    let task = ClaimedTask {
        claim_attempt: 0,
        task_id: "task-cost-wait".to_string(),
        user_id: 7,
        chat_id: 8,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let blocker = TaskCostBlocker {
        status_code: "llm_cost_hard_ceiling".to_string(),
        scope: "task".to_string(),
        observed_cost_usd_nanos: 6_000_000_000,
        limit_cost_usd_nanos: 5_000_000_000,
        retry_after_seconds: 600,
        message_key: "llm.cost_hard_ceiling".to_string(),
    };
    let mut journal = crate::task_journal::TaskJournal::for_task(&task.task_id, "ask", "opaque");

    let checkpoint_id = record_cost_wait_checkpoint(&state, &task, &mut journal, &blocker);

    assert!(checkpoint_id.starts_with("llm-cost:task-cost-wait:"));
    assert_eq!(
        journal
            .task_lifecycle
            .as_ref()
            .and_then(|value| value.get("state"))
            .and_then(serde_json::Value::as_str),
        Some("waiting")
    );
    assert_eq!(
        journal
            .task_lifecycle
            .as_ref()
            .and_then(|value| value.get("blocker_kind"))
            .and_then(serde_json::Value::as_str),
        Some("cost_policy")
    );
    assert_eq!(
        journal
            .task_checkpoint
            .as_ref()
            .and_then(|value| value.pointer("/boundary_context/blocker_kind"))
            .and_then(serde_json::Value::as_str),
        Some("cost_policy")
    );
    assert_eq!(
        journal
            .task_checkpoint
            .as_ref()
            .and_then(|value| value.get("resume_entrypoint"))
            .and_then(serde_json::Value::as_str),
        Some("next_planner_round")
    );
}
