use super::*;

#[test]
fn verifier_scalar_observation_ignores_json_hidden_in_visible_text() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-verifier-scalar-text-boundary",
        "ask",
        "read field",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "text": "{\"action\":\"read_field\",\"field_path\":\"package.name\",\"exists\":true,\"value\":\"hidden\",\"value_text\":\"hidden\"}"
            })
            .to_string(),
        ));

    let values = recent_structured_scalar_values_from_journal(&journal, 1);
    assert!(
        values.iter().all(|value| value.text != "hidden"),
        "{values:?}"
    );
}

#[test]
fn verifier_scalar_observation_accepts_extra_machine_payload() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-verifier-scalar-extra",
        "ask",
        "read field",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "extra": {
                    "action": "read_field",
                    "field_path": "package.name",
                    "exists": true,
                    "value": "rustclaw",
                    "value_text": "rustclaw"
                },
                "text": "display only"
            })
            .to_string(),
        ));

    let values = recent_structured_scalar_values_from_journal(&journal, 1);
    assert!(
        values.iter().any(|value| value.text == "rustclaw"),
        "{values:?}"
    );
}

#[test]
fn verifier_find_ext_ignores_json_hidden_in_visible_text() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-verifier-find-ext-text-boundary",
        "ask",
        "count extension",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "text": "{\"action\":\"find_ext\",\"ext\":\"rs\",\"count\":1,\"results\":[\"src/main.rs\"]}"
            })
            .to_string(),
        ));

    assert_eq!(observed_find_ext_results(&journal), None);
}

#[test]
fn verifier_find_ext_accepts_extra_machine_payload() {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        "task-verifier-find-ext-extra",
        "ask",
        "count extension",
    );
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "step_1",
            "fs_basic",
            json!({
                "extra": {
                    "action": "find_ext",
                    "ext": "rs",
                    "count": 1,
                    "results": ["src/main.rs"]
                },
                "text": "display only"
            })
            .to_string(),
        ));

    assert_eq!(
        observed_find_ext_results(&journal),
        Some((vec!["src/main.rs".to_string()], 1))
    );
}

#[test]
fn verifier_health_check_ignores_json_hidden_in_visible_text() {
    let output = json!({
        "text": "{\"clawd_process_count\":1,\"clawd_health_port_open\":true}"
    })
    .to_string();

    assert_eq!(health_check_value_from_output(&output), None);
}

#[test]
fn verifier_health_check_accepts_extra_machine_payload() {
    let output = json!({
        "extra": {
            "clawd_process_count": 1,
            "clawd_health_port_open": true
        },
        "text": "display only"
    })
    .to_string();

    let value = health_check_value_from_output(&output).expect("health payload");
    assert_eq!(
        value
            .get("clawd_process_count")
            .and_then(serde_json::Value::as_i64),
        Some(1)
    );
}
