use super::*;

#[test]
fn storage_diagnostics_project_only_numeric_evidence_and_known_roles() {
    let input = json!({"wait_reason":"storage_pressure","api_key":"secret-fixture", "storage_pressure":{"volumes":[
        {"role":"system","state":"critical","available_bytes":100,"critical_reserve_bytes":200,"path":"/private/fixture"},
        {"role":"workspace","state":"normal","available_bytes":999},
        {"role":"untrusted-fixture","state":"critical","available_bytes":1}
    ]}});
    let result = project(&input).unwrap();
    assert_eq!(result["volumes"].as_array().unwrap().len(), 1);
    assert_eq!(result["volumes"][0]["shortfall_bytes"], 101);
    assert!(result.get("api_key").is_none());
    assert!(result["volumes"][0].get("path").is_none());
}

#[test]
fn unavailable_measurements_stay_unknown_and_numeric_overflow_is_not_invented() {
    let result = project(&json!({"wait_reason":"memory_unavailable","resource_grant":{
        "host":{"memory_available_mb":u64::MAX},"request":{"memory_mb":32},"host_safety_reserve_mb":16
    }})).unwrap();
    assert!(result["memory"]["available_bytes"].is_null());
    assert_eq!(result["memory"]["required_bytes"], 32 * 1024 * 1024);
    assert_eq!(result["memory"]["safety_reserve_bytes"], 16 * 1024 * 1024);
    assert_eq!(
        project(&json!({"wait_reason":"arbitrary prose fixture"})).unwrap()["reason_code"],
        "resource_pressure"
    );
}

#[test]
fn natural_language_error_does_not_become_a_resource_protocol() {
    assert!(from_error("waiting for memory fixture").is_none());
}

#[test]
fn canonical_retryable_skill_error_round_trips_to_public_diagnostics() {
    let raw = crate::skills::structured_skill_error_from_parts(
        "fixture",
        "resource_admission_unavailable",
        "fixture diagnostic",
        None,
        Some(
            json!({"retryable":true,"side_effect_applied":false,"failure_phase":"pre_dispatch","wait_reason":"storage_pressure","storage_pressure":{"volumes":[
                {"role":"system","state":"critical","available_bytes":1,"critical_reserve_bytes":2}
            ]}}),
        ),
    );
    let value = from_error(&raw).unwrap();
    assert_eq!(value["reason_code"], "storage_pressure");
    assert_eq!(value["volumes"][0]["shortfall_bytes"], 2);
}

#[test]
fn diagnostics_require_the_complete_host_admission_refusal_contract() {
    let raw = crate::skills::structured_skill_error_from_parts(
        "fixture",
        "resource_admission_unavailable",
        "fixture",
        None,
        Some(json!({"retryable":true,"wait_reason":"storage_pressure"})),
    );
    assert!(from_error(&raw).is_none());
}
