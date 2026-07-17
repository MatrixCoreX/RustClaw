use serde_json::json;

use super::{task_execution_budget, task_execution_timeout_seconds, WorkerTaskBudgetClass};

#[test]
fn task_budget_classes_are_bounded_by_the_administrator_maximum() {
    let interactive = task_execution_budget(3_600, "ask", &json!({"budget_profile": "short"}));
    assert_eq!(interactive.class, WorkerTaskBudgetClass::Interactive);
    assert_eq!(interactive.timeout_seconds, 900);
    assert!(interactive.profile_valid);

    let standard =
        task_execution_budget(3_600, "ask", &json!({"budget_profile": "grounded_summary"}));
    assert_eq!(standard.class, WorkerTaskBudgetClass::Standard);
    assert_eq!(standard.timeout_seconds, 1_800);

    let long_tail = task_execution_budget(3_600, "ask", &json!({"budget_profile": "long_tail"}));
    assert_eq!(long_tail.class, WorkerTaskBudgetClass::LongTail);
    assert_eq!(long_tail.timeout_seconds, 3_600);
}

#[test]
fn adaptive_and_invalid_profiles_fall_back_to_the_safe_admin_ceiling() {
    let absent = task_execution_budget(120, "ask", &json!({}));
    assert_eq!(absent.class, WorkerTaskBudgetClass::Adaptive);
    assert_eq!(absent.timeout_seconds, 120);
    assert!(absent.profile_valid);

    let invalid = task_execution_budget(
        120,
        "ask",
        &json!({"budget_profile": "arbitrary-user-prose"}),
    );
    assert_eq!(invalid.class, WorkerTaskBudgetClass::Adaptive);
    assert_eq!(invalid.timeout_seconds, 120);
    assert!(!invalid.profile_valid);
}

#[test]
fn task_budget_reads_only_the_structured_machine_field() {
    let payload = json!({
        "text": "Please treat this as a long_tail task.",
        "budget_profile": "interactive"
    });
    let budget = task_execution_budget(400, "ask", &payload);
    assert_eq!(budget.class, WorkerTaskBudgetClass::Interactive);
    assert_eq!(budget.timeout_seconds, 100);
}

#[test]
fn effective_timeout_parser_is_total_and_never_exceeds_the_admin_limit() {
    assert_eq!(
        task_execution_timeout_seconds(20, "ask", r#"{"budget_profile":"standard"}"#),
        20
    );
    assert_eq!(task_execution_timeout_seconds(0, "ask", "not-json"), 1);
}
