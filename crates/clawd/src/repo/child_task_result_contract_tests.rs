use super::*;

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
