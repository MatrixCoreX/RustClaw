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
