use super::*;

fn test_state() -> crate::AppState {
    let state = crate::AppState::test_default_with_fixture_provider();
    let db = state.core.db.get().expect("task plan test db");
    ensure_task_plan_schema(&db).expect("task plan schema");
    drop(db);
    state
}

fn steps() -> Vec<TaskPlanStep> {
    vec![
        TaskPlanStep {
            step_id: "inspect".to_string(),
            title: "Inspect current state".to_string(),
            status: TaskPlanStepStatus::InProgress,
        },
        TaskPlanStep {
            step_id: "implement".to_string(),
            title: "Implement the change".to_string(),
            status: TaskPlanStepStatus::Pending,
        },
    ]
}

#[test]
fn set_read_and_update_plan_preserve_revision_and_step_order() {
    let state = test_state();
    let created = set_task_plan(&state, "task-plan-1", 0, steps()).expect("set plan");
    assert_eq!(created["plan_revision"], 1);
    assert_eq!(created["steps"][0]["step_id"], "inspect");
    assert_eq!(created["checkpoint"]["ref"], "task_plan:task-plan-1:1");

    let updated = update_task_plan_steps(
        &state,
        "task-plan-1",
        1,
        vec![
            TaskPlanStepUpdate {
                step_id: "inspect".to_string(),
                title: None,
                status: Some(TaskPlanStepStatus::Completed),
            },
            TaskPlanStepUpdate {
                step_id: "implement".to_string(),
                title: Some("Implement and verify".to_string()),
                status: Some(TaskPlanStepStatus::InProgress),
            },
        ],
    )
    .expect("update plan");
    assert_eq!(updated["plan_revision"], 2);
    assert_eq!(updated["steps"][0]["status"], "completed");
    assert_eq!(updated["steps"][1]["status"], "in_progress");
    assert_eq!(updated["steps"][1]["title"], "Implement and verify");

    let restored = read_task_plan(&state, "task-plan-1", "read_plan").expect("read plan");
    assert_eq!(restored["plan_revision"], 2);
    assert_eq!(restored["steps"], updated["steps"]);
}

#[test]
fn revision_conflict_is_structured_and_does_not_overwrite_plan() {
    let state = test_state();
    set_task_plan(&state, "task-plan-2", 0, steps()).expect("set plan");
    let error = update_task_plan_steps(
        &state,
        "task-plan-2",
        0,
        vec![TaskPlanStepUpdate {
            step_id: "inspect".to_string(),
            title: None,
            status: Some(TaskPlanStepStatus::Completed),
        }],
    )
    .expect_err("stale revision must fail");
    assert_eq!(error.error_code, "task_plan_revision_conflict");
    assert!(error.retryable);
    assert_eq!(error.expected_revision, Some(0));
    assert_eq!(error.current_revision, Some(1));

    let restored = read_task_plan(&state, "task-plan-2", "read_plan").expect("read plan");
    assert_eq!(restored["plan_revision"], 1);
    assert_eq!(restored["steps"][0]["status"], "in_progress");
}

#[test]
fn plan_rejects_duplicate_ids_and_multiple_in_progress_steps() {
    let state = test_state();
    let mut duplicate = steps();
    duplicate[1].step_id = "inspect".to_string();
    assert_eq!(
        set_task_plan(&state, "task-plan-3", 0, duplicate)
            .expect_err("duplicate must fail")
            .error_code,
        "task_plan_invalid"
    );

    let mut multiple_active = steps();
    multiple_active[1].status = TaskPlanStepStatus::InProgress;
    assert_eq!(
        set_task_plan(&state, "task-plan-3", 0, multiple_active)
            .expect_err("multiple active steps must fail")
            .detail
            .as_deref(),
        Some("multiple_in_progress_steps_not_allowed")
    );
}

#[test]
fn read_before_set_returns_revision_zero_and_no_checkpoint() {
    let state = test_state();
    let result = read_task_plan(&state, "task-plan-empty", "read_plan").expect("read empty");
    assert_eq!(result["plan_revision"], 0);
    assert!(result["steps"].is_null());
    assert!(result["checkpoint"].is_null());
}
