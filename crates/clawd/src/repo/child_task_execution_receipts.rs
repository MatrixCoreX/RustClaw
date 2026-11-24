//! Bounded, task-owned write evidence for a parent consuming a terminal child.
use claw_core::capability_result::CapabilityResultEnvelope;
use serde_json::{json, Value};

const MAX_RECEIPTS: usize = 8;
const MAX_RECEIPT_BYTES: usize = 12 * 1024;
const MAX_CONTENT_BYTES: usize = 4096;

pub(super) fn project(child_task_id: &str, result: &Value) -> Option<Value> {
    if child_task_id.is_empty() {
        return None;
    }
    // Only the host's persisted journal is eligible. Child prose, business JSON,
    // grants and similarly named producer output cannot become host receipts.
    let trace = result
        .pointer("/task_journal/trace")
        .or_else(|| result.pointer("/final_result_json/task_journal/trace"))?;
    if trace.get("task_id")?.as_str()? != child_task_id {
        return None;
    }
    let results = trace.get("capability_results")?.as_array()?;
    let steps = trace.get("step_results")?.as_array()?;
    let mut receipts = Vec::new();
    let mut eligible_count = 0;
    let mut used_bytes = 0;
    for raw in results {
        let Ok(mut envelope) = serde_json::from_value::<CapabilityResultEnvelope>(raw.clone())
        else {
            continue;
        };
        if envelope.validate().is_err()
            || envelope.provenance["task_id"].as_str() != Some(child_task_id)
        {
            continue;
        }
        let Some(step_id) = envelope.provenance["step_id"].as_str() else {
            continue;
        };
        if step_id.is_empty()
            || step_id.len() > 160
            || !steps.iter().any(|step| {
                step["step_id"].as_str() == Some(step_id)
                    && step["status"].as_str() == Some("ok")
                    && matches!(
                        step["requested_action_type"].as_str(),
                        Some("call_capability" | "call_tool" | "call_skill")
                    )
                    && ["requested_action_ref", "resolved_capability"]
                        .iter()
                        .any(|key| step[*key].as_str() == Some(envelope.capability.as_str()))
            })
        {
            continue;
        }
        let step_id = step_id.to_owned();
        let args = envelope.provenance["continuation_args"].clone();
        let complete_content = envelope
            .provenance
            .pointer("/continuation_args_projection/complete_fields")
            .and_then(Value::as_array)
            .is_some_and(|fields| fields.iter().any(|field| field.as_str() == Some("content")));
        if !complete_content {
            continue;
        }
        let identity = envelope.canonical_evidence_identity();
        // Recompute from the retained host snapshot and exact submitted bytes;
        // do not promote even a similarly named provenance value by copying it.
        crate::capability_result::bind_workspace_write_receipt(&mut envelope, &args);
        let Some(receipt) = envelope.provenance.get("write_content_receipt") else {
            continue;
        };
        eligible_count += 1;
        let content = args.get("content").and_then(Value::as_str);
        let content_included = content.is_some_and(|text| text.len() <= MAX_CONTENT_BYTES);
        let row = json!({
            "step_id": step_id,
            "capability": envelope.capability,
            "evidence_id": identity.evidence_id,
            "sha256": identity.sha256,
            "write_content_receipt": receipt,
            "submitted_content_included": content_included,
            "submitted_content": content.filter(|_| content_included),
        });
        let bytes = row.to_string().len();
        if receipts.len() < MAX_RECEIPTS && used_bytes + bytes <= MAX_RECEIPT_BYTES {
            used_bytes += bytes;
            receipts.push(row);
        }
    }
    (eligible_count > 0).then(|| {
        json!({
            "schema_version": 1,
            "source": "host_child_execution_receipts",
            "child_task_id": child_task_id,
            "source_ref": format!("task:{child_task_id}:task_journal.trace"),
            "scope": "committed_write_snapshot",
            "retained_trace_only": true,
            "eligible_receipt_count": eligible_count,
            "returned_receipt_count": receipts.len(),
            "truncated": receipts.len() < eligible_count,
            "receipts": receipts,
        })
    })
}

#[cfg(test)]
#[path = "child_task_execution_receipts_tests.rs"]
mod tests;
