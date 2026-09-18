use super::*;
use claw_core::capability_result::CapabilityResultEnvelope;

#[test]
fn equal_nested_extra_is_referenced_without_losing_output_fields_or_identity() {
    let extra = json!({"fixture_data":"x".repeat(8000),"size_bytes":17});
    let mut result = CapabilityResultEnvelope::ok(
        "fixture.inspect",
        None,
        json!({
            "extra":extra,"output":{"extra":extra,"status":"ok","text":"distinct visible result"},
        }),
    );
    result.provenance = json!({"step_id":"s1","task_id":"t1"});
    let original = serde_json::to_value(&result).unwrap();
    let projected = deduplicated_capability_evidence(&result);
    assert_eq!(projected["data"]["extra"], extra);
    assert_eq!(
        projected["data"]["output"]["text"],
        "distinct visible result"
    );
    assert_eq!(projected["data"]["output"]["extra_reference"], "data.extra");
    assert_eq!(projected["provenance"], original["provenance"]);
    assert!(projected.to_string().len() + 7000 < original.to_string().len());
    assert_eq!(serde_json::to_value(&result).unwrap(), original);
    let evidence = provider_safe_capability_result_evidence(&result);
    assert_eq!(evidence["step_id"], "s1");
    assert_eq!(evidence["projection"], "structured_result");
}

#[test]
fn differing_or_absent_fields_are_never_deduplicated() {
    for data in [
        json!({"extra":{"size_bytes":17},"output":{"extra":{"size_bytes":18}}}),
        json!({"output":{"extra":{"size_bytes":17}}}),
        json!({"extra":null,"output":{"extra":null}}),
    ] {
        let result = CapabilityResultEnvelope::ok("fixture.inspect", None, data);
        assert_eq!(
            deduplicated_capability_evidence(&result),
            serde_json::to_value(&result).unwrap()
        );
    }
}
