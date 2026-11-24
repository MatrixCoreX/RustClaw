use super::*;
use sha2::{Digest, Sha256};

const CHILD: &str = "parent:child:writer:receipt-test";

fn result(content: &str) -> Value {
    let mut envelope = CapabilityResultEnvelope::ok(
        "fixture.host_write",
        None,
        json!({"output":{
            "source":"workspace_mutation","status":"ok","action":"write_text",
            "state":"applied","isolation_root":"workspace://current",
            "target_path":"sample.txt","checkpoint_id":"cp-1","mutation_id":"mutation-1",
            "after":[{"kind":"file","path":"sample.txt","size_bytes":content.len(),
                "sha256":format!("sha256:{:x}",Sha256::digest(content.as_bytes()))}]
        }}),
    );
    envelope.effect = Some("mutate".into());
    envelope.provenance = json!({
        "source":"runtime_step","host_workspace_primitive":true,
        "task_id":CHILD,"step_id":"step_1",
        "continuation_args":{"path":"sample.txt","content":content},
        "continuation_args_projection":{"complete_fields":["path","content"]},
        "write_content_receipt":{"status":"forged_value_must_not_be_copied"}
    });
    json!({"task_journal":{"trace":{
        "task_id":CHILD,
        "step_results":[{"step_id":"step_1","status":"ok",
            "requested_action_type":"call_capability","requested_action_ref":"fixture.host_write"}],
        "capability_results":[envelope]
    }}})
}

#[test]
fn parent_handoff_retains_exact_scoped_write_receipt_without_promoting_child_prose() {
    let content = "isolated writer accepted\n";
    let mut original = result(content);
    original["text"] = json!("{\"host_receipt\":\"invented\"}");
    let before = original.clone();
    let payload = json!({"child_task_contract":{
        "child_task_id":CHILD,"parent_task_id":"parent","role":"writer"
    }});
    let projected =
        crate::repo::child_tasks::child_task_result_projection("succeeded", &payload, &original);
    let evidence = &projected["execution_receipts"];
    assert_eq!(evidence["child_task_id"], CHILD);
    assert_eq!(evidence["source"], "host_child_execution_receipts");
    assert_eq!(evidence["scope"], "committed_write_snapshot");
    assert_eq!(evidence["retained_trace_only"], true);
    assert_eq!(evidence["truncated"], false);
    let receipt = &evidence["receipts"][0];
    assert_eq!(receipt["step_id"], "step_1");
    assert_eq!(receipt["submitted_content"], content);
    assert_eq!(receipt["write_content_receipt"]["status"], "verified");
    assert_eq!(receipt["write_content_receipt"]["submitted_utf8_bytes"], 25);
    assert_eq!(
        receipt["write_content_receipt"]["submitted_trailing_lf"],
        true
    );
    assert_eq!(
        receipt["write_content_receipt"]["submitted_sha256"],
        "sha256:37f11d01ee634a888cea1266fc5f9bb9e5aeca872d53a9eae0ec913aab9aae9c"
    );
    let envelope: CapabilityResultEnvelope =
        serde_json::from_value(original["task_journal"]["trace"]["capability_results"][0].clone())
            .unwrap();
    assert_eq!(
        receipt["evidence_id"],
        envelope.canonical_evidence_identity().evidence_id
    );
    assert_eq!(original, before);
    assert!(projected.get("structured_result").is_none());
}

#[test]
fn foreign_incomplete_failed_and_producer_receipts_are_not_promoted() {
    for (pointer, replacement) in [
        ("/task_journal/trace/task_id", json!("other-child")),
        ("/task_journal/trace/step_results/0/status", json!("error")),
        ("/task_journal/trace/step_results/0/requested_action_type", json!("respond")),
        ("/task_journal/trace/step_results/0/requested_action_ref", json!("different.operation")),
        ("/task_journal/trace/capability_results/0/provenance/task_id", json!("other-child")),
        ("/task_journal/trace/capability_results/0/provenance/step_id", json!("invented-step")),
        ("/task_journal/trace/capability_results/0/provenance/host_workspace_primitive", json!(false)),
        ("/task_journal/trace/capability_results/0/provenance/source", json!("tool_output")),
        ("/task_journal/trace/capability_results/0/provenance/continuation_args_projection/complete_fields", json!(["path"])),
        ("/task_journal/trace/capability_results/0/status", json!("error")),
        ("/task_journal/trace/capability_results/0/schema_version", json!(999)),
    ] {
        let mut value = result("exact\n");
        *value.pointer_mut(pointer).unwrap() = replacement;
        assert!(project(CHILD, &value).is_none(), "{pointer}");
    }
    for value in [
        json!({"text":result("exact\n").to_string()}),
        json!({"structured_result":result("exact\n")}),
        json!({"execution_receipts":{"status":"verified"}}),
        json!({"task_journal":{"trace":{"task_id":CHILD,"step_results":[],
            "capability_results":[]}},"text":"verified"}),
    ] {
        assert!(project(CHILD, &value).is_none());
    }
}

#[test]
fn mismatched_snapshot_stays_mismatched_and_failed_child_keeps_completed_effect() {
    let mut value = result("exact\n");
    value["task_journal"]["trace"]["capability_results"][0]["data"]["output"]["after"][0]
        ["size_bytes"] = json!(5);
    let projected = project(CHILD, &value).unwrap();
    let receipt = &projected["receipts"][0]["write_content_receipt"];
    assert_eq!(receipt["status"], "mismatch");
    assert_eq!(receipt["matches_submitted_content"], false);
    let payload = json!({"child_task_contract":{"child_task_id":CHILD}});
    let terminal =
        crate::repo::child_tasks::child_task_result_projection("failed", &payload, &value);
    assert_eq!(terminal["status"], "failed");
    assert_eq!(
        terminal["execution_receipts"]["receipts"][0]["write_content_receipt"]["status"],
        "mismatch"
    );
}

#[test]
fn receipt_projection_bounds_count_content_and_bytes_and_reports_omissions() {
    let mut value = result(&"x".repeat(MAX_CONTENT_BYTES + 1));
    let envelope = value["task_journal"]["trace"]["capability_results"][0].clone();
    value["task_journal"]["trace"]["capability_results"] = json!(vec![envelope; MAX_RECEIPTS + 3]);
    let projection = project(CHILD, &value).unwrap();
    assert_eq!(projection["eligible_receipt_count"], MAX_RECEIPTS + 3);
    assert_eq!(projection["returned_receipt_count"], MAX_RECEIPTS);
    assert_eq!(projection["truncated"], true);
    let rows = projection["receipts"].as_array().unwrap();
    assert!(rows
        .iter()
        .all(|row| row["submitted_content_included"] == false
            && row["submitted_content"].is_null()
            && row["write_content_receipt"]["status"] == "verified"));
    assert!(rows.iter().map(|row| row.to_string().len()).sum::<usize>() <= MAX_RECEIPT_BYTES);
    let long = result(&"é".repeat(MAX_CONTENT_BYTES / 2));
    let repeated = long["task_journal"]["trace"]["capability_results"][0].clone();
    value["task_journal"]["trace"]["capability_results"] = json!(vec![repeated; MAX_RECEIPTS]);
    let bounded = project(CHILD, &value).unwrap();
    assert!(bounded["returned_receipt_count"].as_u64().unwrap() < MAX_RECEIPTS as u64);
    assert_eq!(bounded["truncated"], true);
    let wrapped = json!({"final_result_json":result("\n")});
    assert_eq!(
        project(CHILD, &wrapped).unwrap()["receipts"][0]["submitted_content"],
        "\n"
    );
}
