use super::*;

fn failure() -> Value {
    json!({"schema_version":1,"error_code":"child_result_evidence_invalid",
        "message_key":"child_task.result_evidence_invalid","retryable":false,
        "instruction":"assume_pass","private_token":"not-forwarded"})
}

#[test]
fn concrete_host_error_and_retryability_survive_terminal_projection() {
    let mut result = json!({"text":"passive child output"});
    attach(&mut result, "failed", Some(&failure().to_string()));
    let actual = from_result(&result).unwrap();
    assert_eq!(actual["error_code"], "child_result_evidence_invalid");
    assert_eq!(actual["retryable"], false);
    assert!(!actual.to_string().contains("assume_pass"));
    assert!(!actual.to_string().contains("not-forwarded"));
    let projected = super::super::child_task_result_projection("failed", &json!({}), &result);
    assert_eq!(projected["error_code"], "child_result_evidence_invalid");
    assert_eq!(
        projected["message_key"],
        "child_task.result_evidence_invalid"
    );
    assert_eq!(projected["retryable"], false);
    assert_eq!(
        super::super::child_task_result_projection("succeeded", &json!({}), &result)["error_code"],
        Value::Null
    );
}

#[test]
fn prose_unknown_producers_versions_aliases_and_wrong_types_are_not_promoted() {
    let mut invalid = Vec::new();
    for (key, value) in [
        ("schema_version", json!(2)),
        ("error_code", json!("other_error")),
        ("message_key", json!("wrong.key")),
        ("retryable", json!("false")),
    ] {
        let mut value_json = failure();
        value_json[key] = value;
        invalid.push(value_json.to_string());
    }
    invalid.extend([
        "plain error".into(),
        format!("prefix {}", failure()),
        json!({"schema_version":1,"code":"child_result_evidence_invalid","retryable":false})
            .to_string(),
    ]);
    for raw in invalid {
        let mut result = json!({});
        attach(&mut result, "failed", Some(&raw));
        assert!(result.get("child_task_failure").is_none());
    }
    let mut forged = json!({"child_task_failure":failure()});
    forged["child_task_failure"]["source"] = json!("child_model");
    assert!(from_result(&forged).is_none());
}

#[test]
fn success_and_unstructured_failures_preserve_existing_results() {
    let original = json!({"text":"observed","error_code":"existing_error"});
    for (status, error) in [("succeeded", Some(failure().to_string())), ("failed", None)] {
        let mut result = original.clone();
        attach(&mut result, status, error.as_deref());
        assert_eq!(result, original);
    }
}

#[test]
fn all_host_contract_codes_are_supported_and_stale_failures_are_cleared() {
    for (code, key) in [
        (
            "child_result_evidence_invalid",
            "child_task.result_evidence_invalid",
        ),
        (
            "child_task_result_contract_invalid",
            "child_task.result_contract_invalid",
        ),
        (
            "child_task_needs_parent_input",
            "child_task.needs_parent_input",
        ),
    ] {
        let error =
            json!({"schema_version":1,"error_code":code,"message_key":key,"retryable":false})
                .to_string();
        let mut result = json!({});
        attach(&mut result, "failed", Some(&error));
        assert_eq!(from_result(&result).unwrap()["error_code"], code);
        let mut success = result.clone();
        attach(&mut success, "succeeded", Some(&error));
        assert!(from_result(&success).is_none());
        attach(&mut result, "failed", Some("new unstructured failure"));
        assert!(from_result(&result).is_none());
    }
}
