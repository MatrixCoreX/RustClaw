use serde_json::json;

use super::{
    child_task_contract_policy::{
        child_fanout_policy, child_scheduler_decision, merge_child_task_results,
    },
    child_task_event_json, parent_cancel_child_directive, ChildTaskBudget, ChildTaskLifecycleEvent,
    ChildTaskMergePolicy, ChildTaskPermissionProfile, ChildTaskSpec,
};

fn sample_child_spec(required: bool) -> ChildTaskSpec {
    ChildTaskSpec {
        parent_task_id: "task-parent".to_string(),
        child_task_id: "task-child-1".to_string(),
        role: "explorer".to_string(),
        scope: json!({
            "scope_ref": "workspace:current",
            "context_refs": ["artifact:plan"]
        }),
        permission_profile: ChildTaskPermissionProfile::ReadOnly,
        required,
        budget: ChildTaskBudget::readonly_default(),
        result_contract: json!({
            "kind": "structured_findings",
            "required_keys": ["finding_refs", "evidence_refs"]
        }),
        merge_policy: ChildTaskMergePolicy::StructuredFindings,
    }
}

#[test]
fn child_task_schema_exposes_required_machine_contract() {
    let spec = sample_child_spec(true);
    let value = spec.to_json();

    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["parent_task_id"], "task-parent");
    assert_eq!(value["child_task_id"], "task-child-1");
    assert_eq!(value["role"], "explorer");
    assert_eq!(value["permission_profile"], "read_only");
    assert_eq!(value["required"], true);
    assert_eq!(value["merge_policy"], "structured_findings");
    assert_eq!(value["budget"]["max_rounds"], 2);
    assert_eq!(value["result_contract"]["kind"], "structured_findings");
}

#[test]
fn child_task_lifecycle_events_are_stable_machine_tokens() {
    let spec = sample_child_spec(false);
    let queued = child_task_event_json(ChildTaskLifecycleEvent::Queued, &spec, None);
    let failed = child_task_event_json(
        ChildTaskLifecycleEvent::Failed,
        &spec,
        Some("child_timeout"),
    );

    assert_eq!(queued["event_type"], "subagent_queued");
    assert_eq!(queued["status"], "queued");
    assert_eq!(queued["required"], false);
    assert_eq!(failed["event_type"], "subagent_failed");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["reason_code"], "child_timeout");
}

#[test]
fn parent_cancel_directive_targets_child_task_ids_only() {
    let directive = parent_cancel_child_directive(
        "task-parent",
        &["task-child-1".to_string(), "task-child-2".to_string()],
    );

    assert_eq!(directive["directive"], "cancel_child_tasks");
    assert_eq!(directive["parent_task_id"], "task-parent");
    assert_eq!(directive["reason_code"], "parent_cancelled");
    assert_eq!(directive["child_task_ids"][0], "task-child-1");
    assert_eq!(directive["child_task_ids"][1], "task-child-2");
}

#[test]
fn fanout_policy_blocks_recursive_or_excessive_children() {
    let allowed = child_fanout_policy(1, 4);
    let too_deep = child_fanout_policy(2, 4);
    let too_many = child_fanout_policy(1, 17);

    assert_eq!(allowed["decision"], "allowed");
    assert_eq!(too_deep["decision"], "rejected");
    assert_eq!(too_deep["reason_code"], "child_recursion_depth_exceeded");
    assert_eq!(too_many["decision"], "rejected");
    assert_eq!(too_many["reason_code"], "child_fanout_limit_exceeded");
}

#[test]
fn scheduler_decision_bounds_parallel_children() {
    let scheduled = child_scheduler_decision(3, 4, 1);
    let bounded = child_scheduler_decision(6, 4, 1);
    let rejected = child_scheduler_decision(2, 4, 2);

    assert_eq!(scheduled["decision"], "scheduled");
    assert_eq!(scheduled["scheduled_child_count"], 3);
    assert_eq!(bounded["decision"], "bounded_partial");
    assert_eq!(bounded["scheduled_child_count"], 4);
    assert_eq!(bounded["skipped_child_count"], 2);
    assert_eq!(rejected["decision"], "rejected");
    assert_eq!(rejected["scheduled_child_count"], 0);
}

#[test]
fn merge_child_results_propagates_required_failure_without_parsing_prose() {
    let child_results = vec![
        json!({
            "child_task_id": "task-child-ok",
            "status": "succeeded",
            "required": true,
            "evidence_refs": ["child-evidence:1"],
            "finding_refs": ["child-finding:1"],
            "artifact_refs": ["child-artifact:1"]
        }),
        json!({
            "child_task_id": "task-child-optional",
            "status": "failed",
            "required": false,
            "error_code": "child_timeout",
            "text": "ignored visible fallback"
        }),
        json!({
            "child_task_id": "task-child-required",
            "status": "failed",
            "required": true,
            "error_code": "missing_required_evidence"
        }),
    ];

    let merged = merge_child_task_results("task-parent", &child_results);

    assert_eq!(merged["status"], "failed_required_child");
    assert_eq!(merged["parent_can_continue"], false);
    assert_eq!(merged["completed_count"], 1);
    assert_eq!(merged["failed_count"], 2);
    assert_eq!(merged["required_failed_count"], 1);
    assert_eq!(merged["optional_failed_count"], 1);
    assert_eq!(merged["evidence_refs"][0], "child-evidence:1");
    assert_eq!(merged["finding_refs"][0], "child-finding:1");
    assert_eq!(merged["artifact_refs"][0], "child-artifact:1");
}

#[test]
fn merge_child_results_isolates_optional_failures() {
    let child_results = vec![
        json!({
            "child_task_id": "task-child-ok",
            "status": "completed",
            "required": true
        }),
        json!({
            "child_task_id": "task-child-optional",
            "status": "cancelled",
            "required": false,
            "error_code": "parent_cancelled"
        }),
    ];

    let merged = merge_child_task_results("task-parent", &child_results);

    assert_eq!(merged["status"], "partial");
    assert_eq!(merged["parent_can_continue"], true);
    assert_eq!(merged["cancelled_count"], 1);
    assert_eq!(merged["required_failed_count"], 0);
    assert_eq!(merged["optional_failed_count"], 1);
}
