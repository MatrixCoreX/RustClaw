use super::super::execution_evidence_prompt_block;
use crate::task_journal::{TaskJournal, TaskJournalStepTrace};
use serde_json::{json, Value};

#[test]
fn verifier_distinguishes_executed_metadata_from_unexecuted_readback() {
    let mut journal = TaskJournal::default();
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"stat","size_bytes":0}}"#,
    ));
    journal.task_observations.push(json!({
        "schema_version":1, "owner_layer":"task_journal", "observation_kind":"checkpoint_step_provenance",
        "step_id":"step_1", "requested_action_type":"call_capability",
        "requested_capability":"filesystem.stat_paths", "resolved_capability":"filesystem.stat_paths",
        "resolved_tool_or_skill":"fs_basic", "dispatch_executed":true,
    }));
    journal.task_observations.push(json!({
        "schema_version":1, "owner_layer":"task_journal", "observation_kind":"checkpoint_step_provenance",
        "step_id":"step_2", "requested_action_type":"call_capability",
        "requested_capability":"filesystem.read_text_range", "resolved_tool_or_skill":"fs_basic",
        "dispatch_executed":true,
    }));
    let block: Value = serde_json::from_str(&execution_evidence_prompt_block(&journal)).unwrap();
    let operations = block["executed_operations"]["operations"]
        .as_array()
        .unwrap();
    assert_eq!(
        operations.len(),
        1,
        "unexecuted observation must not invent a dispatch"
    );
    assert_eq!(
        operations[0]["resolved_capability"],
        "filesystem.stat_paths"
    );
    assert_eq!(operations[0]["status"], "ok");
    assert_eq!(block["executed_operations"]["truncated"], false);
}

#[test]
fn failed_and_unknown_operations_keep_status_and_do_not_borrow_identity() {
    let mut journal = TaskJournal::default();
    journal
        .step_results
        .push(TaskJournalStepTrace::ok("step_1", "fs_basic", "{}"));
    journal.step_results.push(TaskJournalStepTrace::ok(
        "step_2",
        "respond",
        "completed read",
    ));
    journal.step_results[0].status = crate::executor::StepExecutionStatus::Error;
    let block: Value = serde_json::from_str(&execution_evidence_prompt_block(&journal)).unwrap();
    let ops = &block["executed_operations"]["operations"];
    assert_eq!(ops[0]["status"], "error");
    assert!(ops[0]["resolved_capability"].is_null());
    assert_eq!(ops.as_array().unwrap().len(), 1);
}
