use super::*;

fn sample_task(index: usize, task_id: &str, status: &str) -> ActiveTaskItem {
    ActiveTaskItem {
        index,
        task_id: task_id.to_string(),
        kind: "ask".to_string(),
        status: status.to_string(),
        summary: "task-summary".to_string(),
        age_seconds: 12,
    }
}

#[test]
fn list_extra_is_machine_renderable_contract() {
    let tasks = vec![sample_task(
        1,
        "00000000-0000-4000-8000-000000000001",
        "running",
    )];

    let extra = task_list_extra(&tasks);

    assert_eq!(extra.get("action").and_then(Value::as_str), Some("list"));
    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("task_control.list.ok")
    );
    assert_eq!(extra.get("task_count").and_then(Value::as_u64), Some(1));
    assert_eq!(
        extra.pointer("/items/0/task_id").and_then(Value::as_str),
        Some("00000000-0000-4000-8000-000000000001")
    );
    assert!(extra.to_string().starts_with('{'));
}

#[test]
fn empty_list_extra_uses_message_key_not_sentence_contract() {
    let extra = task_list_extra(&[]);

    assert_eq!(extra.get("status").and_then(Value::as_str), Some("empty"));
    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("task_control.list.empty")
    );
    assert_eq!(
        extra
            .pointer("/field_value/has_unfinished")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn cancel_all_result_extra_is_structured_for_success_and_empty() {
    let tasks = vec![
        sample_task(1, "00000000-0000-4000-8000-000000000001", "running"),
        sample_task(2, "00000000-0000-4000-8000-000000000002", "queued"),
    ];

    let success = cancel_all_result_extra(&tasks, 2);
    assert_eq!(
        success.get("message_key").and_then(Value::as_str),
        Some("task_control.cancel_all.ok")
    );
    assert_eq!(
        success.get("canceled_count").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        success
            .pointer("/field_value/task_ids/1")
            .and_then(Value::as_str),
        Some("00000000-0000-4000-8000-000000000002")
    );

    let empty = cancel_all_result_extra(&[], 0);
    assert_eq!(empty.get("status").and_then(Value::as_str), Some("empty"));
    assert_eq!(
        empty.get("message_key").and_then(Value::as_str),
        Some("task_control.cancel_all.empty")
    );
}

#[test]
fn cancel_one_result_extra_carries_task_identity() {
    let task = sample_task(3, "00000000-0000-4000-8000-000000000003", "running");

    let extra = cancel_one_result_extra(&task);

    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("task_control.cancel_one.ok")
    );
    assert_eq!(
        extra
            .pointer("/field_value/task_id")
            .and_then(Value::as_str),
        Some("00000000-0000-4000-8000-000000000003")
    );
    assert_eq!(
        extra.pointer("/field_value/index").and_then(Value::as_u64),
        Some(3)
    );
}

#[test]
fn parse_input_reports_machine_error_for_missing_cancel_index() {
    let err = parse_input(&json!({"action":"cancel_one"})).expect_err("missing index");

    assert_eq!(err, "cancel_one_missing_index");
}
