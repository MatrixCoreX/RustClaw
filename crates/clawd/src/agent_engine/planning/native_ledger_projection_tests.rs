use super::*;

fn restore(mut row: Value, last: &str) -> Value {
    let reference = row
        .as_object_mut()
        .unwrap()
        .remove("observed_output_reference")
        .unwrap();
    assert_eq!(reference["scope"], "value_excerpt_only");
    let value = match reference["projection"].as_str().unwrap() {
        "identity" => last.trim().to_string(),
        "agent_trace_v1" => crate::truncate_for_agent_trace(last.trim()),
        _ => panic!("unknown projection"),
    };
    assert_eq!(reference["excerpt_chars"], value.chars().count());
    assert_eq!(
        reference["excerpt_sha256"],
        format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
    );
    row["observed_output"] = json!(value);
    row
}

#[test]
fn full_and_bounded_views_restore_exact_trace_bytes_and_keep_execution_metadata() {
    for last in ["α\tline\r\n".repeat(100), "α\tline\r\n".repeat(4000)] {
        let excerpt = crate::truncate_for_agent_trace(last.trim());
        let row = json!({"attempt_id":"a7","action_ref":"fixture.read","status":"ok",
            "args_fingerprint":"opaque-hash","retryable":false,"missing_evidence":[],"observed_output":excerpt});
        let input = json!([row.clone()]);
        let before = input.clone();
        let projected: Value = serde_json::from_str(&project(&input.to_string(), &last)).unwrap();
        assert_eq!(restore(projected[0].clone(), &last), row);
        assert_eq!(input, before);
    }
}

#[test]
fn partial_ledger_keeps_counts_origin_and_older_distinct_observations() {
    let last = "x".repeat(8000);
    let prior = json!({"attempt_id":"a1","observed_output":"y".repeat(1000)});
    let latest =
        json!({"attempt_id":"a11","observed_output":crate::truncate_for_agent_trace(&last)});
    let input = json!({"complete":false,"original_count":11,"returned_count":2,
        "canonical_location":"task_checkpoint.attempt_ledger","recent_attempts":[prior,latest]});
    let mut projected: Value = serde_json::from_str(&project(&input.to_string(), &last)).unwrap();
    let restored = restore(projected["recent_attempts"][1].clone(), &last);
    projected["recent_attempts"][1] = restored;
    assert_eq!(projected, input);
}

#[test]
fn distinct_values_small_payloads_collisions_and_unknown_shapes_stay_verbatim() {
    let last = "x".repeat(1000);
    for input in [
        "(empty)".to_string(),
        json!({}).to_string(),
        json!([]).to_string(),
        json!([{"observed_output":"different".repeat(200)}]).to_string(),
        json!([{"observed_output":"small"}]).to_string(),
        json!([{"observed_output":last,"observed_output_reference":"existing-data"}]).to_string(),
    ] {
        assert_eq!(project(&input, &last), input);
    }
}

#[test]
fn a_trace_reference_does_not_assert_unrecorded_tail_or_merge_two_operations() {
    let last = "x".repeat(12000);
    let excerpt = crate::truncate_for_agent_trace(last.trim());
    let input = json!([{"attempt_id":"a1","observed_output":excerpt},{"attempt_id":"a2","observed_output":excerpt}]);
    let value: Value = serde_json::from_str(&project(&input.to_string(), &last)).unwrap();
    assert_eq!(value.as_array().unwrap().len(), 2);
    assert_eq!(value[0], input[0]);
    assert_eq!(
        value[1]["observed_output_reference"]["projection"],
        "agent_trace_v1"
    );
    assert_eq!(restore(value[1].clone(), &last), input[1]);
}
