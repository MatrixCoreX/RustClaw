use super::*;
use claw_core::capability_result::CapabilityResultEnvelope;

#[test]
fn host_write_receipt_survives_provider_evidence_projection_without_re_reading() {
    let content = "{\"title\":\"review sample\",\"value\":17,\"enabled\":true}\n";
    let mut result = CapabilityResultEnvelope::ok(
        "fixture.host_write",
        None,
        json!({"output":{
            "source":"workspace_mutation","status":"ok","action":"write_text",
            "state":"applied","isolation_root":"workspace://current",
            "target_path":"sample.txt","checkpoint_id":"cp-1","mutation_id":"mutation-1",
            "after":[{"kind":"file","path":"sample.txt","size_bytes":52,
                "sha256":"sha256:30622c20068bfbb1e571ec6eb1ad03e66ac910cb73ef278bca1480e2fd614660"}]
        }}),
    );
    result.effect = Some("mutate".into());
    result.provenance = json!({"source":"runtime_step","host_workspace_primitive":true,
        "continuation_args":{"path":"sample.txt","content":content},"step_id":"step_2"});
    crate::capability_result::bind_workspace_write_receipt(
        &mut result,
        &json!({"content":content}),
    );
    let canonical = serde_json::to_value(&result).unwrap();
    let projected = provider_safe_capability_result_evidence(&result);
    assert_eq!(projected["projection"], "structured_result");
    assert_eq!(
        projected["result"]["provenance"]["write_content_receipt"]["matches_submitted_content"],
        true
    );
    assert_eq!(
        projected["result"]["provenance"]["write_content_receipt"]["submitted_trailing_lf"],
        true
    );
    assert_eq!(
        projected["result"]["provenance"]["write_content_receipt"]["submitted_utf8_bytes"],
        52
    );
    assert_eq!(
        projected["result"]["provenance"]["continuation_args"]["content"],
        content
    );
    assert_eq!(
        projected["evidence_id"],
        result.canonical_evidence_identity().evidence_id
    );
    assert_eq!(serde_json::to_value(&result).unwrap(), canonical);
}

#[test]
fn verifier_record_view_roundtrips_without_modifying_the_journal_or_evidence() {
    let output = json!({"extra":{"paths":(0..12).map(|index|json!({
        "path":format!("documents/item-{index}.txt"),"exists":true,"kind":"file",
        "size_bytes":index+10,"modified_ts":123456
    })).collect::<Vec<_>>()}})
    .to_string();
    let step = crate::task_journal::TaskJournalStepTrace::ok("step_1", "fs_basic", &output);
    let original = crate::task_journal::observed_evidence_for_step_trace(&step).unwrap();
    let projected = provider_safe_step_evidence(&step);
    let mut restored = projected["observed_evidence"].clone();
    assert_eq!(
        restored["host_item_encoding"],
        crate::prompt_projection::RECORD_ENCODING
    );
    restored["items"] = crate::prompt_projection::tests::restore(&restored["items"]);
    restored
        .as_object_mut()
        .unwrap()
        .remove("host_item_encoding");
    assert_eq!(restored, original);
    assert_eq!(
        crate::task_journal::observed_evidence_for_step_trace(&step),
        Some(original)
    );
    assert_eq!(step.output_excerpt.as_deref(), Some(output.as_str()));
    assert_eq!(
        projected["output_excerpt_hash"],
        provider_safe_excerpt_hash(&output)
    );
    assert!(
        provider_safe_step_evidence(&crate::task_journal::TaskJournalStepTrace::default())
            ["observed_evidence"]
            .is_null()
    );
}

#[test]
fn equal_large_leaf_fields_are_losslessly_referenced_inside_the_same_result() {
    let body = "line\n".repeat(1200);
    let extra = json!({"excerpt":body,"a/b~c":["item".repeat(400)],"count":3,"host_only":true});
    let mut result = CapabilityResultEnvelope::ok(
        "fixture.read",
        None,
        json!({"extra":extra,"output":{"excerpt":body,"a/b~c":["item".repeat(400)],"count":3,"distinct":"keep"}}),
    );
    result.provenance = json!({"step_id":"s1"});
    let original = serde_json::to_value(&result).unwrap();
    let projection = deduplicated_capability_evidence(&result);
    let references = projection["data"]["output"]["field_references"]
        .as_object()
        .unwrap();
    assert_eq!(references.len(), 2);
    let mut restored = projection.clone();
    restored["data"]["output"]
        .as_object_mut()
        .unwrap()
        .remove("field_references");
    for (field, reference) in references {
        restored["data"]["output"][field] = projection
            .pointer(reference.as_str().unwrap())
            .unwrap()
            .clone();
    }
    assert_eq!(restored, original);
    assert_eq!(projection["data"]["output"]["count"], 3);
    assert_eq!(projection["data"]["output"]["distinct"], "keep");
    assert!(projection.to_string().len() + 6000 < original.to_string().len());
    assert_eq!(serde_json::to_value(&result).unwrap(), original);
    let model_evidence = provider_safe_capability_result_evidence(&result);
    assert_eq!(
        model_evidence["field_reference_contract"],
        "same_result_json_pointer_v1"
    );
    assert_eq!(
        model_evidence["evidence_id"],
        result.canonical_evidence_identity().evidence_id
    );
}

#[test]
fn unequal_payloads_and_reserved_reference_names_are_never_overwritten() {
    let payload = "x".repeat(1000);
    for data in [
        json!({"extra":{"excerpt":payload},"output":{"excerpt":"different".repeat(120)}}),
        json!({"extra":{"excerpt":payload},"output":{"excerpt":payload,"field_references":"user-data"}}),
        json!({"extra":{"v":payload},"output":{"extra":{"v":payload},"extra_reference":"user-data"}}),
    ] {
        let result = CapabilityResultEnvelope::ok("fixture.read", None, data);
        assert_eq!(
            deduplicated_capability_evidence(&result),
            serde_json::to_value(&result).unwrap()
        );
        assert!(
            provider_safe_capability_result_evidence(&result)["field_reference_contract"].is_null()
        );
    }
}

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
    assert_eq!(
        value["verification_retry_context"]["operation_audit_contract"]["operations"],
        json!([])
    );
    assert!(
        value["verification_retry_context"]["operation_audit_contract"]
            ["operation_row_required_fields"]
            .as_array()
            .unwrap()
            .contains(&json!("evidence_step_ids"))
    );
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

#[test]
fn whole_output_references_keep_the_verifier_evidence_identity_and_canonical_bytes() {
    let body = json!({"excerpt":"observed line\n".repeat(120),"count":17});
    let result =
        CapabilityResultEnvelope::ok("fixture.read", None, json!({"extra":body,"output":body}));
    let before = serde_json::to_value(&result).unwrap();
    let identity = result.canonical_evidence_identity();
    let projected = provider_safe_capability_result_evidence(&result);
    assert_eq!(
        projected["field_reference_contract"],
        crate::prompt_projection::REFERENCE_CONTRACT
    );
    assert_eq!(
        projected["result"]["data"]["output_reference"],
        "/data/extra"
    );
    assert_eq!(projected["result"]["data"]["extra"], body);
    assert_eq!(projected["evidence_id"], identity.evidence_id);
    assert_eq!(projected["sha256"], identity.sha256);
    assert_eq!(serde_json::to_value(&result).unwrap(), before);
}
