use super::*;
use crate::repo::task_plan::{TaskPlanStep, TaskPlanStepStatus, TaskPlanStepUpdate};

fn fixture() -> (AppState, ClaimedTask, LoopState) {
    let state = AppState::test_default_with_fixture_provider();
    let task = ClaimedTask {
        claim_attempt: 1,
        task_id: "plan-reconciliation".to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: None,
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut loop_state = LoopState::new();
    loop_state.last_user_visible_respond = Some("candidate".to_string());
    loop_state
        .successful_action_fingerprints
        .insert("completed-write".to_string(), 1);
    (state, task, loop_state)
}

fn set_plan(state: &AppState, task: &ClaimedTask, status: TaskPlanStepStatus) -> Value {
    crate::repo::set_task_plan(
        state,
        &task.task_id,
        0,
        vec![TaskPlanStep {
            step_id: "step-a".to_string(),
            title: "arbitrary multilingual title".to_string(),
            status,
        }],
    )
    .unwrap()
}

#[test]
fn unfinished_plan_requests_one_planner_reconciliation_without_mutating_it() {
    for status in [TaskPlanStepStatus::Pending, TaskPlanStepStatus::InProgress] {
        let (state, task, mut loop_state) = fixture();
        let original = set_plan(&state, &task, status);
        assert!(prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
        assert_eq!(
            loop_state.task_observations[0]["snapshot"]["plan_revision"],
            1
        );
        assert!(loop_state.last_user_visible_respond.is_none());
        assert_eq!(
            loop_state.task_observations[0]["candidate_response_prepared"],
            true
        );
        assert_eq!(
            loop_state.task_observations[0]["response_delivery_owner"],
            "runtime"
        );
        assert_eq!(
            loop_state.successful_action_fingerprints["completed-write"],
            1
        );
        let stored = crate::repo::read_task_plan(&state, &task.task_id, "read_plan").unwrap();
        assert_eq!(stored["steps"], original["steps"]);
        assert_eq!(stored["plan_revision"], 1);
        loop_state.last_user_visible_respond = Some("blocked outcome".to_string());
        assert!(!prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
        assert_eq!(
            loop_state.last_user_visible_respond.as_deref(),
            Some("blocked outcome")
        );
    }
}

#[test]
fn completed_cancelled_absent_and_nonterminal_plans_do_not_add_model_turns() {
    for status in [
        None,
        Some(TaskPlanStepStatus::Completed),
        Some(TaskPlanStepStatus::Cancelled),
    ] {
        let (state, task, mut loop_state) = fixture();
        if let Some(status) = status {
            set_plan(&state, &task, status);
        }
        assert!(!prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
        assert!(loop_state.task_observations.is_empty());
    }
    let (state, task, mut loop_state) = fixture();
    set_plan(&state, &task, TaskPlanStepStatus::Pending);
    loop_state.last_user_visible_respond = None;
    assert!(!prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
}

#[test]
fn restored_reconciliation_observation_bounds_retries_after_checkpoint() {
    let (state, task, mut loop_state) = fixture();
    set_plan(&state, &task, TaskPlanStepStatus::Pending);
    loop_state
        .task_observations
        .push(json!({"owner_layer":"agent_loop","reason_code":REASON}));
    assert!(!prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
}

fn multiple_step_plan(state: &AppState, task: &ClaimedTask) {
    crate::repo::set_task_plan(
        state,
        &task.task_id,
        0,
        (0..3)
            .map(|index| TaskPlanStep {
                step_id: format!("step-{index}"),
                title: format!("uninterpreted-{index}"),
                status: TaskPlanStepStatus::Pending,
            })
            .collect(),
    )
    .unwrap();
}

fn complete_step(
    state: &AppState,
    task: &ClaimedTask,
    revision: u64,
    id: &str,
) -> Result<Value, crate::repo::task_plan::TaskPlanError> {
    crate::repo::update_task_plan_steps(
        state,
        &task.task_id,
        revision,
        vec![TaskPlanStepUpdate {
            step_id: id.to_string(),
            title: None,
            status: Some(TaskPlanStepStatus::Completed),
        }],
    )
}

#[test]
fn progress_allows_one_final_reconciliation_with_the_latest_revision() {
    let (state, task, mut loop_state) = fixture();
    multiple_step_plan(&state, &task);
    assert!(prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    complete_step(&state, &task, 1, "step-0").unwrap();
    loop_state.last_user_visible_respond = Some("prepared response".into());
    assert!(prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    assert_eq!(
        loop_state.task_observations[1]["snapshot"]["plan_revision"],
        2
    );
    assert_eq!(loop_state.task_observations[1]["reconciliation_attempt"], 2);
    complete_step(&state, &task, 2, "step-1").unwrap();
    loop_state.last_user_visible_respond = Some("actual blocked outcome".into());
    assert!(!prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    assert_eq!(loop_state.task_observations[2]["reason_code"], INCOMPLETE);
    assert_eq!(loop_state.task_observations[2]["remaining_step_count"], 1);
    assert_eq!(
        loop_state.last_user_visible_respond.as_deref(),
        Some("actual blocked outcome")
    );
    assert_eq!(
        loop_state.successful_action_fingerprints["completed-write"],
        1
    );
    assert!(!prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    assert_eq!(loop_state.task_observations.len(), 3);
}

#[test]
fn stale_revision_never_overwrites_a_newer_plan_or_replays_work() {
    let (state, task, mut loop_state) = fixture();
    multiple_step_plan(&state, &task);
    assert!(prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    complete_step(&state, &task, 1, "step-0").unwrap();
    let error = complete_step(&state, &task, 1, "step-1").unwrap_err();
    assert_eq!(error.error_code, "task_plan_revision_conflict");
    loop_state.last_user_visible_respond = Some("candidate".into());
    assert!(prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    let snapshot = &loop_state.task_observations[1]["snapshot"];
    assert_eq!(snapshot["plan_revision"], 2);
    assert_eq!(snapshot["steps"][1]["status"], "pending");
    assert_eq!(
        loop_state.successful_action_fingerprints["completed-write"],
        1
    );
}

#[test]
fn revision_churn_without_completed_work_does_not_buy_another_attempt() {
    let (state, task, mut loop_state) = fixture();
    set_plan(&state, &task, TaskPlanStepStatus::Pending);
    assert!(prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    crate::repo::update_task_plan_steps(
        &state,
        &task.task_id,
        1,
        vec![TaskPlanStepUpdate {
            step_id: "step-a".into(),
            title: Some("a different title".into()),
            status: Some(TaskPlanStepStatus::InProgress),
        }],
    )
    .unwrap();
    loop_state.last_user_visible_respond = Some("blocked".into());
    assert!(!prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    assert_eq!(loop_state.task_observations[1]["reason_code"], INCOMPLETE);
    assert_eq!(
        loop_state.task_observations[1]["snapshot"]["steps"][0]["status"],
        "in_progress"
    );
}

#[test]
fn checkpoint_restored_attempts_cannot_reset_the_reconciliation_budget() {
    let (state, task, mut loop_state) = fixture();
    multiple_step_plan(&state, &task);
    assert!(prepare_task_plan_reconciliation(&state, &task, &mut loop_state).unwrap());
    complete_step(&state, &task, 1, "step-0").unwrap();
    let restored: Vec<Value> =
        serde_json::from_str(&serde_json::to_string(&loop_state.task_observations).unwrap())
            .unwrap();
    let mut resumed = LoopState::new();
    resumed.task_observations = restored;
    resumed.last_user_visible_respond = Some("candidate".into());
    assert!(prepare_task_plan_reconciliation(&state, &task, &mut resumed).unwrap());
    complete_step(&state, &task, 2, "step-1").unwrap();
    resumed.last_user_visible_respond = Some("candidate".into());
    assert!(!prepare_task_plan_reconciliation(&state, &task, &mut resumed).unwrap());
}
