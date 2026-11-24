//! Bounded host-owned protocol feedback; never imports a provider's prose.
use serde_json::{json, Value};
use std::collections::BTreeSet;

const MAX_OPERATIONS: usize = 32;
const MAX_DISPATCHES: usize = 8;

pub(super) fn project(journal: &crate::task_journal::TaskJournal) -> Value {
    contract(
        journal
            .step_results
            .iter()
            .zip(journal.executed_operation_evidence())
            .filter(|(step, _)| super::is_external_execution_step(step))
            .map(|(_, operation)| operation),
    )
}

fn token(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str).filter(|value| {
        !value.is_empty()
            && value.len() <= 128
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_.:-".contains(&b))
    })
}

fn contract(operations: impl IntoIterator<Item = Value>) -> Value {
    let mut rows = Vec::new();
    let mut truncated = false;
    for (index, operation) in operations.into_iter().enumerate() {
        if index == MAX_OPERATIONS {
            truncated = true;
            break;
        }
        let Some(step_id) = token(operation.get("step_id")) else {
            truncated = true;
            continue;
        };
        let Some(status) = token(operation.get("status")) else {
            truncated = true;
            continue;
        };
        let mut dispatches = BTreeSet::new();
        for field in ["resolved_capability", "requested_capability"] {
            if let Some(reference) = token(operation.get(field)) {
                dispatches.insert(("call_capability", reference));
            }
        }
        if let Some(action_type @ ("call_tool" | "call_skill")) =
            token(operation.get("requested_action_type"))
        {
            if let Some(reference) = token(operation.get("requested_action_ref")) {
                dispatches.insert((action_type, reference));
            }
        }
        if let Some(managed) = operation
            .get("runtime_managed_capabilities")
            .and_then(Value::as_array)
        {
            for reference in managed
                .iter()
                .take(MAX_DISPATCHES + 1)
                .filter_map(|v| token(Some(v)))
            {
                dispatches.insert(("call_capability", reference));
            }
            truncated |= managed.len() > MAX_DISPATCHES;
        }
        truncated |= dispatches.len() > MAX_DISPATCHES;
        rows.push(json!({"step_id":step_id,"status":status,"dispatches":dispatches.into_iter()
            .take(MAX_DISPATCHES).map(|(action_type, action_ref)| json!({"action_type":action_type,"action_ref":action_ref})).collect::<Vec<_>>()}));
    }
    let schema = crate::prompt_utils::PromptSchemaId::AnswerVerifier.schema_value();
    json!({
        "schema_version":1,
        "operation_row_required_fields":schema.pointer("/properties/operation_checks/items/required"),
        "evidence_scope":"current_task_executed_operations_only",
        "delegation_scope":"parent_dispatch_and_host_owned_child_results",
        "presentation_check_location":"verdict_not_operation_rows",
        "operations":rows,
        "truncated":truncated,
    })
}

#[cfg(test)]
#[path = "verification_retry_contract_tests.rs"]
mod tests;
