use super::*;
use claw_core::capability_result::CapabilityResultEnvelope;
use serde_json::{json, Value};

#[test]
fn planner_observation_references_equal_raw_output_and_keeps_canonical_evidence() {
    let body = json!({"excerpt":"line α\n".repeat(150),"count":17});
    let result =
        CapabilityResultEnvelope::ok("fixture.read", None, json!({"extra":body,"output":body}));
    let before = serde_json::to_value(&result).unwrap();
    let rendered = generic_capability_observation(&result).unwrap();
    let mut projected: Value = serde_json::from_str(
        rendered
            .strip_prefix("capability_result_observation=")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        projected["field_reference_contract"],
        crate::prompt_projection::REFERENCE_CONTRACT
    );
    assert_eq!(projected["data"]["output_reference"], "/data/extra");
    projected["data"]["output"] = projected.pointer("/data/extra").unwrap().clone();
    projected["data"]
        .as_object_mut()
        .unwrap()
        .remove("output_reference");
    assert_eq!(projected["data"], result.data);
    assert_eq!(serde_json::to_value(&result).unwrap(), before);
}

#[test]
fn distinct_raw_values_with_equal_bounded_previews_are_never_referenced_as_equal() {
    let prefix = "x".repeat(32000);
    let result = CapabilityResultEnvelope::ok(
        "fixture.read",
        None,
        json!({
            "extra":{"excerpt":format!("{prefix}left")},
            "output":{"excerpt":format!("{prefix}right")}
        }),
    );
    let rendered = generic_capability_observation(&result).unwrap();
    let projected: Value = serde_json::from_str(
        rendered
            .strip_prefix("capability_result_observation=")
            .unwrap(),
    )
    .unwrap();
    assert!(projected.get("field_reference_contract").is_none());
    assert!(projected.pointer("/data/output_reference").is_none());
    assert!(projected.pointer("/data/output/field_references").is_none());
}

#[test]
fn a_target_dropped_by_preview_bounds_is_not_replaced_with_a_dangling_reference() {
    let mut extra = serde_json::Map::new();
    for i in 0..60 {
        extra.insert(format!("a{i:02}"), json!(i));
    }
    let body = "retained content".repeat(100);
    extra.insert("zz".into(), json!(body));
    let result = CapabilityResultEnvelope::ok(
        "fixture.read",
        None,
        json!({
            "extra":extra,"output":{"zz":body,"unique":1}
        }),
    );
    let rendered = generic_capability_observation(&result).unwrap();
    let projected: Value = serde_json::from_str(
        rendered
            .strip_prefix("capability_result_observation=")
            .unwrap(),
    )
    .unwrap();
    assert!(projected.get("field_reference_contract").is_none());
    assert_eq!(projected["data"]["output"]["zz"], body);
}
