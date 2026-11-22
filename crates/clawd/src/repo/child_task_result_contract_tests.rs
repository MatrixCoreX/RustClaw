use super::*;

#[test]
fn host_versioned_projection_preserves_observed_findings_without_a_schema() {
    let mut payload = json!({"task_role":"subagent_child",
        "child_execution":{"result_projection_version":2},
        "child_task_contract":{"result_contract":{"output_format":"machine_json",
            "required_keys":["function_name","requires_ui_admin_first","file_path"],
            "allow_partial":false,"evidence_field":"evidence_refs"}}});
    let answer = json!({"function_name":"get_checkpoint_status","requires_ui_admin_first":true,
        "file_path":"src/status.rs","function_line":317,"admin_call_line":321,
        "notes":"observed in the authorized read","evidence_refs":["step:read-1"]});
    let result = json!({"text":answer.to_string()});
    assert_eq!(
        validated_child_structured_result(&payload, &result),
        Some(answer.clone())
    );
    payload["child_execution"]["result_projection_version"] = json!(1);
    let legacy = json!({"function_name":"get_checkpoint_status","requires_ui_admin_first":true,
        "file_path":"src/status.rs","evidence_refs":["step:read-1"]});
    assert_eq!(
        validated_child_structured_result(&payload, &result),
        Some(legacy.clone())
    );
    payload.as_object_mut().unwrap().remove("child_execution");
    let spoofed =
        json!({"text":answer.to_string(),"child_execution":{"result_projection_version":2}});
    assert_eq!(
        validated_child_structured_result(&payload, &spoofed),
        Some(legacy)
    );
}

#[test]
fn versioned_projection_does_not_relax_required_schema_size_or_evidence_checks() {
    let payload = json!({"task_role":"subagent_child",
        "child_execution":{"result_projection_version":2},
        "child_task_contract":{"result_contract":{"output_format":"machine_json",
            "required_keys":["name"],"allow_partial":false,"schema_version":1,
            "evidence_field":"evidence","result_schema":{"type":"object",
                "additionalProperties":false,"required":["name","evidence"],
                "properties":{"name":{"type":"string"},"line":{"type":"integer","minimum":1},
                    "evidence":{"type":"array","items":{"type":"string"}}}}}}});
    let answer = json!({"name":"handler","line":321,"evidence":["step:read-1"]});
    assert_eq!(
        validated_child_structured_result(&payload, &json!({"text":answer.to_string()})),
        Some(answer.clone())
    );
    for invalid in [
        json!({"line":321,"evidence":["step:read-1"]}),
        json!({"name":null,"line":321,"evidence":["step:read-1"]}),
        json!({"name":"handler","line":"321","evidence":["step:read-1"]}),
        json!({"name":"handler","line":321,"evidence":["step:read-1"],"extra":true}),
        json!({"name":"x".repeat(33*1024),"evidence":["step:read-1"]}),
    ] {
        assert!(
            validated_child_structured_result(&payload, &json!({"structured_result":invalid}))
                .is_none()
        );
    }
    assert!(child_evidence_refs_valid(
        &payload,
        &answer.to_string(),
        &["read-1"]
    ));
    for evidence in [json!([]), json!(["step:missing"]), json!(["read-1"])] {
        let mut invalid = answer.clone();
        invalid["evidence"] = evidence;
        assert!(!child_evidence_refs_valid(
            &payload,
            &invalid.to_string(),
            &["read-1"]
        ));
    }
    // Completeness also applies to the schema-free version-2 path.
    let mut without_schema = payload.clone();
    without_schema["child_task_contract"]["result_contract"]
        .as_object_mut()
        .unwrap()
        .remove("result_schema");
    for invalid in [json!({"line":321}), json!({"name":null})] {
        assert!(validated_child_structured_result(
            &without_schema,
            &json!({"structured_result":invalid})
        )
        .is_none());
    }
}

#[test]
fn unknown_or_malformed_host_projection_versions_fail_closed() {
    let mut payload = json!({"task_role":"subagent_child",
        "child_execution":{},"child_task_contract":{"result_contract":{"output_format":"machine_json"}}});
    for version in [
        Value::Null,
        json!("2"),
        json!(true),
        json!(0),
        json!(3),
        json!(2.0),
        json!({}),
    ] {
        payload["child_execution"]["result_projection_version"] = version;
        assert!(validated_child_structured_result(
            &payload,
            &json!({"structured_result":{"name":"handler"}})
        )
        .is_none());
    }
}

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
fn schema_declared_optional_findings_survive_required_key_projection() {
    let payload = json!({"child_task_contract":{"result_contract":{
        "schema_version":1,"output_format":"machine_json","required_keys":["name"],
        "result_schema":{"type":"object","required":["name"],"additionalProperties":true,
            "properties":{"name":{"type":"string"},"line":{"type":"integer","minimum":1},
                "excerpt":{"type":"string"},"optional":{"type":["string","null"]}}}
    }}});
    let answer = json!({"name":"handler","line":321,"excerpt":"321| check_access()", "optional":null,
        "undeclared":"not forwarded"});
    let actual = validated_child_structured_result(&payload, &json!({"text":answer.to_string()}));
    assert_eq!(
        actual,
        Some(json!({"name":"handler","line":321,
        "excerpt":"321| check_access()","optional":null}))
    );
    for line in [json!("321"), json!(0)] {
        let mut invalid = answer.clone();
        invalid["line"] = line;
        assert!(
            validated_child_structured_result(&payload, &json!({"text":invalid.to_string()}))
                .is_none()
        );
    }
    let mut missing = answer.clone();
    missing.as_object_mut().unwrap().remove("name");
    assert!(
        validated_child_structured_result(&payload, &json!({"text":missing.to_string()})).is_none()
    );
}

#[test]
fn declared_optional_evidence_is_retained_and_still_requires_real_receipts() {
    let payload = json!({"task_role":"subagent_child","child_task_contract":{"result_contract":{
        "output_format":"machine_json","required_keys":["name"],"evidence_field":"evidence",
        "require_evidence":true
    }}});
    let mut journal = crate::task_journal::TaskJournal::new("fixture");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "read-1",
            "fixture.read",
            "observed",
        ));
    let answer = json!({"name":"handler","evidence":["step:read-1"],"undeclared":"excluded"});
    assert_eq!(
        validated_child_structured_result(&payload, &json!({"text":answer.to_string()})),
        Some(json!({"name":"handler","evidence":["step:read-1"]}))
    );
    assert!(child_evidence_contract_valid(
        &payload,
        &answer.to_string(),
        &journal
    ));
    for evidence in [json!([]), json!(["step:missing"])] {
        let mut invalid = answer.clone();
        invalid["evidence"] = evidence;
        assert!(!child_evidence_contract_valid(
            &payload,
            &invalid.to_string(),
            &journal
        ));
    }
}

#[test]
fn optional_findings_do_not_bypass_complete_or_bounded_result_checks() {
    let payload = json!({"child_task_contract":{"result_contract":{
        "schema_version":1,"output_format":"machine_json","required_keys":["name"],"allow_partial":false,
        "result_schema":{"type":"object","properties":{"name":{},"details":{"type":"string"}}}
    }}});
    assert!(child_result_contract_supported(
        &payload["child_task_contract"]["result_contract"]
    ));
    assert!(validated_child_structured_result(
        &payload,
        &json!({"structured_result":{"name":"handler","details":"present"}})
    )
    .is_some());
    for answer in [
        json!({"name":null,"details":"present"}),
        json!({"name":"handler","details":"x".repeat(33*1024)}),
    ] {
        assert!(
            validated_child_structured_result(&payload, &json!({"structured_result":answer}))
                .is_none()
        );
    }
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

#[test]
fn versioned_schema_checks_ranges_types_nullability_and_preserves_result_field() {
    let mut payload = json!({"task_role":"subagent_child","child_task_contract":{"result_contract":{
        "output_format":"machine_json","schema_version":1,"result_schema":{"type":"object","required":["result","count"],
        "properties":{"result":{"type":"object"},"count":{"type":"integer","minimum":1,"maximum":3},"optional":{"type":["null","string"]}},"additionalProperties":false}}}});
    let answer = json!({"result":{"key":"value"},"count":2,"optional":null});
    assert_eq!(
        validated_child_structured_result(&payload, &json!({"text":answer.to_string()})),
        Some(answer)
    );
    for count in [json!("2"), json!(0), json!(4)] {
        assert!(child_result_contract_failure(
            &payload,
            &json!({"result":{},"count":count}).to_string()
        )
        .is_some());
    }
    payload["child_task_contract"]["result_contract"]["result_schema"]["$ref"] = json!("remote");
    assert!(!child_result_contract_supported(
        &payload["child_task_contract"]["result_contract"]
    ));
}

#[test]
fn declared_evidence_rejects_empty_and_dangling_refs_and_accepts_real_successful_steps() {
    let payload = json!({"task_role":"subagent_child","child_task_contract":{"result_contract":{"output_format":"machine_json","schema_version":1,"evidence_field":"evidence"}}});
    let mut journal = crate::task_journal::TaskJournal::new("fixture");
    journal
        .step_results
        .push(crate::task_journal::TaskJournalStepTrace::ok(
            "read-1",
            "fixture.read",
            "observed",
        ));
    for refs in [json!([]), json!(["step:missing"]), json!(["read-1"])] {
        assert!(!child_evidence_contract_valid(
            &payload,
            &json!({"evidence":refs}).to_string(),
            &journal
        ));
    }
    assert!(child_evidence_contract_valid(
        &payload,
        &json!({"evidence":["step:read-1"]}).to_string(),
        &journal
    ));
}

#[test]
fn invalid_schema_diagnostics_preserve_strict_types_and_version_location() {
    let invalid = json!({"output_format":"machine_json","result_schema":{
        "schema_version":"1","type":"object","properties":{"line":{"type":"integer","minimum":"1"}}}});
    let original = invalid.clone();
    assert!(!child_result_contract_supported(&invalid));
    let issues = child_result_contract_validation_issues(&invalid);
    for path in [
        "/schema_version",
        "/result_schema/schema_version",
        "/result_schema/properties/line/minimum",
    ] {
        assert!(
            issues.iter().any(|issue| issue["path"] == path),
            "missing issue {path}"
        );
    }
    assert_eq!(invalid, original, "diagnostics must not coerce input");
    let valid = json!({"output_format":"machine_json","schema_version":1,
        "result_schema":{"type":"object","properties":{"line":{"type":"integer","minimum":1}}}});
    assert!(child_result_contract_supported(&valid));
    assert!(child_result_contract_validation_issues(&valid).is_empty());
    for value in [Value::Null, json!([]), json!("contract")] {
        assert!(!child_result_contract_supported(&value));
    }
}

#[test]
fn invalid_schema_diagnostics_have_a_bounded_issue_count() {
    let properties = (0..100)
        .map(|n| (format!("field_{n}"), json!({"minimum":"1"})))
        .collect::<serde_json::Map<_, _>>();
    let contract = json!({"schema_version":1,"result_schema":{"properties":properties}});
    assert!(!child_result_contract_supported(&contract));
    assert_eq!(child_result_contract_validation_issues(&contract).len(), 16);
}
