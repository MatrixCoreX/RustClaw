use super::*;

fn result(content: &str) -> CapabilityResultEnvelope {
    let mut result = CapabilityResultEnvelope::ok(
        "fixture.host_write",
        None,
        json!({"output":{
            "source":"workspace_mutation","status":"ok","action":"write_text","state":"applied",
            "isolation_root":"workspace://current","target_path":"sample.txt",
            "checkpoint_id":"checkpoint-1","mutation_id":"mutation-1",
            "after":[{"kind":"file","path":"sample.txt","size_bytes":content.len(),
                "sha256":format!("sha256:{:x}",Sha256::digest(content.as_bytes()))}]
        }}),
    );
    result.effect = Some("mutate".into());
    result.provenance = json!({"source":"runtime_step","host_workspace_primitive":true});
    result
}

#[test]
fn committed_json_with_final_lf_has_exact_host_byte_proof() {
    // Regression: the verifier mistook this requested 52nd LF byte for extra content.
    let content = "{\"title\":\"review sample\",\"value\":17,\"enabled\":true}\n";
    let mut value = result(content);
    let data_before = value.data.clone();
    bind_workspace_write_receipt(&mut value, &json!({"content":content}));
    let receipt = &value.provenance["write_content_receipt"];
    assert_eq!(receipt["status"], "verified");
    assert_eq!(receipt["matches_submitted_content"], true);
    assert_eq!(receipt["submitted_utf8_bytes"], 52);
    assert_eq!(receipt["observed_size_bytes"], 52);
    assert_eq!(receipt["submitted_trailing_lf"], true);
    assert_eq!(receipt["submitted_trailing_crlf"], false);
    assert_eq!(
        receipt["submitted_sha256"],
        "sha256:30622c20068bfbb1e571ec6eb1ad03e66ac910cb73ef278bca1480e2fd614660"
    );
    assert_eq!(receipt["checkpoint_id"], "checkpoint-1");
    assert_eq!(value.data, data_before);
}

#[test]
fn receipt_preserves_empty_unicode_nul_and_line_endings_without_normalization() {
    for content in ["", "中文\0é", "line", "line\n", "line\r\n", "line\n\n"] {
        let mut value = result(content);
        bind_workspace_write_receipt(&mut value, &json!({"content":content}));
        let receipt = &value.provenance["write_content_receipt"];
        assert_eq!(receipt["matches_submitted_content"], true, "{content:?}");
        assert_eq!(receipt["submitted_utf8_bytes"], content.len());
        assert_eq!(receipt["submitted_trailing_lf"], content.ends_with('\n'));
        assert_eq!(
            receipt["submitted_trailing_crlf"],
            content.ends_with("\r\n")
        );
    }
}

#[test]
fn same_length_changed_content_and_missing_lf_cannot_get_verified_receipts() {
    for observed in ["other\n", "value", "value\r\n", "value\n\n"] {
        let mut value = result(observed);
        bind_workspace_write_receipt(&mut value, &json!({"content":"value\n"}));
        assert_eq!(
            value.provenance["write_content_receipt"]["status"],
            "mismatch"
        );
        assert_eq!(
            value.provenance["write_content_receipt"]["matches_submitted_content"],
            false
        );
    }
    let mut value = result("value\n");
    value.data["output"]["after"][0]["size_bytes"] = json!(5);
    bind_workspace_write_receipt(&mut value, &json!({"content":"value\n"}));
    assert_eq!(
        value.provenance["write_content_receipt"]["status"],
        "mismatch"
    );
}

#[test]
fn producer_output_and_unrelated_mutations_cannot_attest_host_write_content() {
    for (pointer, replacement) in [
        ("/provenance/host_workspace_primitive", json!(false)),
        ("/provenance/source", json!("tool_output")),
        ("/effect", json!("observe")),
        ("/status", json!("error")),
        ("/data/output/source", json!("host_scope_mutation")),
        ("/data/output/status", json!("error")),
        ("/data/output/checkpoint_id", json!("")),
        ("/data/output/mutation_id", Value::Null),
        ("/data/output/action", json!("append_text")),
        ("/data/output/state", json!("rewound")),
        (
            "/data/output/isolation_root",
            json!("host://service-account"),
        ),
        ("/data/output/target_path", json!("different.txt")),
        ("/data/output/after", json!([])),
        ("/data/output/after/0/kind", json!("directory")),
        ("/data/output/after/0/size_bytes", json!("6")),
        ("/data/output/after/0/sha256", Value::Null),
    ] {
        let mut raw = serde_json::to_value(result("value\n")).unwrap();
        *raw.pointer_mut(pointer).unwrap() = replacement;
        let mut value: CapabilityResultEnvelope = serde_json::from_value(raw).unwrap();
        value.provenance["write_content_receipt"] = json!({"status":"verified"});
        bind_workspace_write_receipt(&mut value, &json!({"content":"value\n"}));
        assert!(
            value.provenance.get("write_content_receipt").is_none(),
            "{pointer}"
        );
    }
}

#[test]
fn idempotent_no_op_is_verified_and_non_text_arguments_are_not() {
    let mut value = result("same\n");
    value.data["output"]["state"] = json!("no_op");
    bind_workspace_write_receipt(&mut value, &json!({"content":"same\n"}));
    assert_eq!(
        value.provenance["write_content_receipt"]["status"],
        "verified"
    );
    for args in [json!({}), json!({"content":null}), json!({"content":42})] {
        bind_workspace_write_receipt(&mut value, &args);
        assert!(value.provenance.get("write_content_receipt").is_none());
    }
}
