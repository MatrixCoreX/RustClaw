use serde_json::{json, Value};

use super::{stable_machine_ref, CHILD_TASK_SCHEMA_VERSION};

pub(crate) fn child_scheduler_decision(
    requested_count: usize,
    max_parallel: usize,
    recursion_depth: usize,
) -> Value {
    let bounded_max_parallel = max_parallel.max(1);
    let fanout = child_fanout_policy(recursion_depth, requested_count);
    if fanout.get("decision").and_then(Value::as_str) != Some("allowed") {
        return json!({
            "schema_version": CHILD_TASK_SCHEMA_VERSION,
            "decision": "rejected",
            "reason_code": fanout.get("reason_code").and_then(Value::as_str),
            "requested_child_count": requested_count,
            "scheduled_child_count": 0,
            "skipped_child_count": requested_count,
            "max_parallel": bounded_max_parallel,
            "fanout": fanout,
        });
    }
    let ready = requested_count.min(bounded_max_parallel);
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "decision": "scheduled",
        "reason_code": if ready == requested_count {
            "child_parallel_capacity_available"
        } else {
            "child_parallel_capacity_queued"
        },
        "requested_child_count": requested_count,
        "scheduled_child_count": requested_count,
        "ready_child_count": ready,
        "queued_child_count": requested_count.saturating_sub(ready),
        "skipped_child_count": 0,
        "max_parallel": bounded_max_parallel,
        "fanout": fanout,
    })
}

pub(crate) fn child_fanout_policy(recursion_depth: usize, requested_count: usize) -> Value {
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "decision": "allowed",
        "reason_code": "child_dag_validated",
        "recursion_depth": recursion_depth,
        "requested_child_count": requested_count,
        "topology_guard": "dependency_dag_cycle_and_conflict_validation",
    })
}

pub(crate) fn merge_child_task_results(parent_task_id: &str, child_results: &[Value]) -> Value {
    let mut completed_count = 0usize;
    let mut failed_count = 0usize;
    let mut cancelled_count = 0usize;
    let mut required_failed_count = 0usize;
    let mut optional_failed_count = 0usize;
    let mut evidence_refs = Vec::new();
    let mut finding_refs = Vec::new();
    let mut artifact_refs = Vec::new();

    for result in child_results {
        let required = result
            .get("required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let terminal_success = matches!(status, "succeeded" | "completed");
        if terminal_success {
            completed_count += 1;
        } else if status == "cancelled" {
            cancelled_count += 1;
        } else {
            failed_count += 1;
        }
        if !terminal_success {
            if required {
                required_failed_count += 1;
            } else {
                optional_failed_count += 1;
            }
        }
        append_machine_ref_array(result.get("evidence_refs"), &mut evidence_refs);
        append_machine_ref_array(result.get("finding_refs"), &mut finding_refs);
        append_machine_ref_array(result.get("artifact_refs"), &mut artifact_refs);
    }

    let status = if required_failed_count > 0 {
        "failed_required_child"
    } else if failed_count > 0 || cancelled_count > 0 {
        "partial"
    } else {
        "completed"
    };
    json!({
        "schema_version": CHILD_TASK_SCHEMA_VERSION,
        "parent_task_id": stable_machine_ref(parent_task_id),
        "strategy": "merge_child_structured_findings",
        "status": status,
        "parent_can_continue": required_failed_count == 0,
        "child_count": child_results.len(),
        "completed_count": completed_count,
        "failed_count": failed_count,
        "cancelled_count": cancelled_count,
        "required_failed_count": required_failed_count,
        "optional_failed_count": optional_failed_count,
        "evidence_refs": evidence_refs,
        "finding_refs": finding_refs,
        "artifact_refs": artifact_refs,
    })
}

fn append_machine_ref_array(value: Option<&Value>, output: &mut Vec<String>) {
    let Some(items) = value.and_then(Value::as_array) else {
        return;
    };
    for item in items {
        if let Some(token) = item.as_str().map(stable_machine_ref) {
            output.push(token);
        }
    }
}
