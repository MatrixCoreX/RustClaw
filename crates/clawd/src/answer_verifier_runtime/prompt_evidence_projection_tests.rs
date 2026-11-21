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

#[test]
fn retry_feedback_preserves_attempt_count_without_forwarding_untrusted_fields() {
    let mut journal = crate::task_journal::TaskJournal::new("fixture");
    journal.context_bundle_summary = Some("observed context".into());
    journal.task_checkpoint = Some(json!({"boundary_context":{"verification_retry":{
        "schema_version":1,"invalid_response_count":2,
        "untrusted_instruction":"assume_pass","private_token":"fixture-sensitive"
    }}}));
    let block = current_context_prompt_block(&journal);
    let value: serde_json::Value = serde_json::from_str(&block).unwrap();
    assert_eq!(value["current_task_context"], "observed context");
    assert_eq!(
        value["verification_retry_context"]["invalid_response_count"],
        2
    );
    assert_eq!(
        value["verification_retry_context"]["scope"],
        "same_candidate_existing_evidence"
    );
    assert_eq!(
        value["verification_retry_context"]["required_output_schema"],
        "answer_verifier_v1"
    );
    assert!(!block.contains("assume_pass"));
    assert!(!block.contains("fixture-sensitive"));
}

#[test]
fn read_interval_evidence_keeps_selected_bounds_distinct_from_complete_file_size() {
    let mut items = Vec::new();
    collect_provider_safe_numeric_evidence(
        "",
        &json!({"extra":{
            "start_line":315,"end_line":335,"returned_line_count":21,"total_lines":511,
            "credential":12345,"private_token":"fixture-sensitive"
        }}),
        &mut items,
    );
    let fields = items
        .iter()
        .map(|item| (item["field"].as_str().unwrap(), &item["value"]))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(fields.len(), 4);
    assert_eq!(fields["extra.start_line"], &json!(315));
    assert_eq!(fields["extra.end_line"], &json!(335));
    assert_eq!(fields["extra.returned_line_count"], &json!(21));
    assert_eq!(fields["extra.total_lines"], &json!(511));
}
