use serde_json::{json, Value};

#[test]
fn direct_run_skill_observation_records_redacted_extra_evidence() {
    let token = "sk-test_abcdefghijklmnopqrstuvwxyz1234567890";
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "run_skill", "run_skill:demo");

    super::record_run_skill_task_observation(
        &mut journal,
        "demo",
        "ok",
        Some("done"),
        None,
        Some(&json!({"ok": true})),
        Some(&json!({
            "api_token": token,
            "result": {
                "path": "/tmp/output.txt",
                "exists": true
            }
        })),
        Some(true),
        Some(&json!({
            "schema_version": 1,
            "source": "skills_registry",
            "skill": "demo",
            "eligible": false,
            "admission_version": "external-v1"
        })),
    );

    let trace = journal.to_trace_json();
    let trace_text = trace.to_string();
    assert!(!trace_text.contains(token));
    assert_eq!(
        trace
            .get("task_observations")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );

    let items = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("observed_evidence"))
        .and_then(|evidence| evidence.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items");

    let token_item = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("extra.api_token"))
        .expect("extra api token item");
    assert_eq!(
        token_item.get("redacted").and_then(Value::as_bool),
        Some(true)
    );
    let admission = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("external_skill_admission"))
        .expect("external skill admission trace");
    assert_eq!(
        admission.get("eligible").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn direct_run_skill_failure_records_error_observation() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-2", "run_skill", "run_skill:demo");

    super::record_run_skill_task_observation(
        &mut journal,
        "demo",
        "error",
        None,
        Some("missing required field: path"),
        None,
        None,
        None,
        None,
    );

    let trace = journal.to_trace_json();
    let observed = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("observed_evidence"))
        .expect("observed evidence");
    assert_eq!(
        observed.get("source").and_then(Value::as_str),
        Some("step_output")
    );
    assert_eq!(
        trace
            .get("task_observations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|entry| entry.get("source"))
            .and_then(Value::as_str),
        Some("direct_run_skill")
    );
}
