use super::*;

#[test]
fn default_machine_contract_keeps_real_findings_without_an_explicit_key_list() {
    let payload = json!({"task_role":"subagent_child","child_task_contract":{
        "result_contract":{"output_format":"machine_json"}
    }});
    let result = json!({"findings":{"name":"Panel","line":9},"evidence_refs":["read:1"]});
    assert_eq!(
        validated_child_structured_result(&payload, &json!({"text":result.to_string()})),
        Some(result.clone())
    );
    assert!(child_result_contract_failure(&payload, &result.to_string()).is_none());
    assert!(
        child_result_contract_failure(&payload, "natural prose without machine data").is_some()
    );
    let mut invalid = payload.clone();
    invalid["child_task_contract"]["result_contract"]["required_keys"] = json!("not-an-array");
    assert!(child_result_contract_failure(&invalid, &result.to_string()).is_some());
    let oversized = json!({"findings":"x".repeat(33*1024)});
    assert!(child_result_contract_failure(&payload, &oversized.to_string()).is_some());
}

#[test]
fn declared_child_result_requires_every_field_without_parsing_prose() {
    let payload = json!({"task_role":"subagent_child","child_task_contract":{
        "scope":{},"result_contract":{"output_format":"machine_json","required_keys":["name","line"]}
    }});
    for answer in ["name is Panel on line 9", r#"{"name":"Panel"}"#] {
        assert_eq!(
            child_result_contract_failure(&payload, answer).unwrap()["error_code"],
            "child_task_result_contract_invalid"
        );
    }
    assert!(child_result_contract_failure(&payload, r#"{"name":"Panel","line":9}"#).is_none());
    assert!(
        child_result_contract_failure(&json!({"text":"ordinary request"}), "natural answer")
            .is_none()
    );
    let oversized = json!({"name":"x".repeat(33*1024),"line":9}).to_string();
    assert!(child_result_contract_failure(&payload, &oversized).is_some());
    let projection = validated_child_structured_result(
        &payload,
        &json!({"text":r#"{"name":"Panel","line":9,"unrequested":"omitted"}"#}),
    )
    .unwrap();
    assert_eq!(projection, json!({"name":"Panel","line":9}));
}

#[test]
fn complete_child_result_rejects_null_required_values_without_rejecting_valid_empty_values() {
    let mut payload = json!({"task_role":"subagent_child","child_task_contract":{
        "result_contract":{"output_format":"machine_json","allow_partial":false,
            "required_keys":["first_line","evidence"]}
    }});
    let missing_value = json!({"first_line":null,"evidence":{"error_code":"not_found"}});
    assert_eq!(
        child_result_contract_failure(&payload, &missing_value.to_string()).unwrap()["error_code"],
        "child_task_result_contract_invalid"
    );
    for value in [json!(""), json!(false), json!(0), json!([]), json!({})] {
        let answer = json!({"first_line":value,"evidence":[],"optional":null});
        assert!(child_result_contract_failure(&payload, &answer.to_string()).is_none());
    }
    payload["child_task_contract"]["result_contract"]["allow_partial"] = json!(true);
    assert!(child_result_contract_failure(&payload, &missing_value.to_string()).is_none());
    payload["child_task_contract"]["result_contract"]
        .as_object_mut()
        .unwrap()
        .remove("allow_partial");
    assert!(child_result_contract_failure(&payload, &missing_value.to_string()).is_none());
}
