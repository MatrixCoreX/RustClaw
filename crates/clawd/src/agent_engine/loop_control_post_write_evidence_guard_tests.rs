use super::*;

fn records(action: Option<&str>, text: &str) -> Vec<StepActionRecord> {
    let mut extra = json!({"path":"src/fixture.py","resolved_path":"src/fixture.py"});
    if let Some(action) = action {
        extra["action"] = json!(action);
    }
    let step = TaskJournalStepTrace::ok(
        "step_fixture",
        "fs_basic",
        json!({"extra":extra,"text":text}).to_string(),
    );
    step_action_records(0, &step)
}

#[test]
fn user_visible_text_cannot_create_a_write_observation() {
    for text in [
        "written 12 bytes",
        "written src/fixture.py",
        "unrelated presentation",
        "",
    ] {
        let result = records(None, text);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].action, "fs_basic");
        assert!(!is_code_or_test_write_action(&result[0].action));
    }
}

#[test]
fn structured_write_is_preserved_regardless_of_presentation() {
    for text in ["written 12 bytes", "not a write", ""] {
        let result = records(Some("write_text"), text);
        assert!(is_code_or_test_write_action(&result[0].action));
        assert_eq!(result[0].path.as_deref(), Some("src/fixture.py"));
        assert!(!result[0].content_evidence);
    }
}

#[test]
fn structured_read_cannot_be_reclassified_by_a_written_prefix() {
    let result = records(Some("read_text_range"), "written 12 bytes");
    assert_eq!(result[0].action, "read_text_range");
    assert!(!is_code_or_test_write_action(&result[0].action));
    assert!(!result[0].content_evidence);
}

#[test]
fn nested_presentation_json_does_not_supply_machine_action_fields() {
    let result = records(None, r#"{"action":"write_text","path":"src/fixture.py"}"#);
    assert_eq!(result[0].action, "fs_basic");
    assert!(!is_code_or_test_write_action(&result[0].action));
}
