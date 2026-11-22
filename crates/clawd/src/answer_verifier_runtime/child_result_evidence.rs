use serde_json::{json, Value};

use super::*;

pub(super) fn output_contract_for_task(
    route: &AnswerContract,
    task: &ClaimedTask,
    journal: &crate::task_journal::TaskJournal,
    candidate: &str,
) -> String {
    let base = output_contract_prompt_block(route);
    let Some(contract) = crate::repo::child_tasks::child_result_contract_for_task(task) else {
        return base;
    };
    if !crate::repo::child_tasks::child_result_contract_supported(&contract) {
        return base;
    }
    let Some(field) = contract.get("evidence_field").and_then(Value::as_str) else {
        return base;
    };
    let Ok(payload) = serde_json::from_str::<Value>(&task.payload_json) else {
        return base;
    };
    let successful = journal
        .step_results
        .iter()
        .filter(|step| step.status == crate::executor::StepExecutionStatus::Ok)
        .collect::<Vec<_>>();
    let references = successful
        .iter()
        .filter(|step| step.step_id.len() <= 160)
        .take(128)
        .map(|step| format!("step:{}", step.step_id))
        .collect::<Vec<_>>();
    let mut value: Value = serde_json::from_str(&base).expect("serialized output contract");
    value["child_result_evidence_contract"] = json!({
        "schema_version":1,
        "source":"host_child_result_contract",
        "evidence_field":field,
        "reference_format":"step:<successful_step_id>",
        "operation_check_reference_format":"journal_step_id",
        "successful_step_references":references,
        "reference_list_complete":references.len()==successful.len(),
        "candidate_machine_result_valid":crate::repo::child_tasks::child_machine_result_valid_for_task(task,candidate),
        "candidate_evidence_refs_valid":crate::repo::child_tasks::child_evidence_contract_valid(&payload,candidate,journal),
    });
    value.to_string()
}

#[cfg(test)]
#[path = "child_result_evidence_tests.rs"]
mod tests;
