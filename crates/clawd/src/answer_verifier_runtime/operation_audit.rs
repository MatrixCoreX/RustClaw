use serde::Deserialize;

use crate::answer_verifier::AnswerVerifierOut;
use crate::task_journal::TaskJournal;

#[derive(Debug, Deserialize)]
pub(super) struct ModelVerifierOut {
    #[serde(flatten)]
    pub(super) verdict: AnswerVerifierOut,
    pub(super) operation_checks: Vec<OperationCheck>,
}

#[derive(Debug, Deserialize)]
pub(super) struct OperationCheck {
    pub(super) requested_operation: String,
    pub(super) evidence_step_ids: Vec<String>,
    pub(super) required_dispatches: Vec<RequiredDispatch>,
    pub(super) method_observed: bool,
    pub(super) result_observed: bool,
    #[serde(default = "operation_is_applicable")]
    pub(super) applicable: bool,
    #[serde(default)]
    pub(super) blocked: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct RequiredDispatch {
    action_type: String,
    action_ref: String,
}

impl RequiredDispatch {
    fn matches(&self, operation: &serde_json::Value) -> bool {
        let field = |name| operation.get(name).and_then(serde_json::Value::as_str);
        match self.action_type.as_str() {
            "call_capability" => {
                field("resolved_capability") == Some(self.action_ref.as_str())
                    || (field("requested_action_type") == Some("call_capability")
                        && field("requested_capability") == Some(self.action_ref.as_str()))
            }
            "call_tool" | "call_skill" => {
                field("requested_action_type") == Some(self.action_type.as_str())
                    && field("requested_action_ref") == Some(self.action_ref.as_str())
            }
            _ => false,
        }
    }
}

fn operation_is_applicable() -> bool {
    true
}

pub(super) fn invalid_operation_audit() -> AnswerVerifierOut {
    AnswerVerifierOut {
        pass: false,
        missing_evidence_fields: vec!["verification_audit".to_string()],
        answer_incomplete_reason: "verification_audit_invalid".to_string(),
        should_retry: true,
        retry_instruction: String::new(),
        confidence: 1.0,
    }
}

pub(super) fn validate_operation_audit(
    model: ModelVerifierOut,
    journal: &TaskJournal,
) -> AnswerVerifierOut {
    let operations = journal
        .step_results
        .iter()
        .zip(journal.executed_operation_evidence())
        .filter(|(step, _)| super::is_external_execution_step(step))
        .map(|(_, operation)| operation)
        .collect::<Vec<_>>();
    let mut missing_operation = false;
    for check in model.operation_checks {
        if check.requested_operation.trim().is_empty() {
            return invalid_operation_audit();
        }
        if check.required_dispatches.is_empty() && check.evidence_step_ids.is_empty() {
            // Summarizing or formatting already-fetched results is a verdict
            // concern. An empty outcome-only row cannot prove or disprove
            // execution; claiming a method, blocker, or inapplicable branch
            // without steps remains an invalid audit.
            if !check.applicable || check.method_observed || check.blocked {
                return invalid_operation_audit();
            }
            continue;
        }
        let mut successful = false;
        let mut failed = false;
        let mut matching_success = false;
        let mut matching_failure = false;
        for id in &check.evidence_step_ids {
            let Some(step) = operations.iter().find(|step| {
                step.get("step_id").and_then(serde_json::Value::as_str) == Some(id.as_str())
            }) else {
                return invalid_operation_audit();
            };
            successful |= step.get("status").and_then(serde_json::Value::as_str) == Some("ok");
            failed |= step.get("status").and_then(serde_json::Value::as_str) == Some("error");
            if check
                .required_dispatches
                .iter()
                .any(|required| required.matches(step))
            {
                matching_success |=
                    step.get("status").and_then(serde_json::Value::as_str) == Some("ok");
                matching_failure |=
                    step.get("status").and_then(serde_json::Value::as_str) == Some("error");
            }
        }
        if !check.applicable {
            // The model decides the condition; the host requires real evidence
            // and rejects contradictory claims about an unexecuted branch.
            if !successful
                || failed
                || check.method_observed
                || check.blocked
                || !check.result_observed
            {
                return invalid_operation_audit();
            }
            continue;
        }
        let requires_method = !check.required_dispatches.is_empty();
        // Outcome-only / presentation rows have no required dispatch. Citing a
        // successful result step with method_observed=true is a model protocol
        // slip, not missing execution; only required methods need a matching
        // dispatch for method_observed.
        if (requires_method && check.method_observed && !matching_success)
            || (check.blocked
                && !(if requires_method {
                    matching_failure
                } else {
                    failed
                }))
            || (!check.blocked && check.result_observed && !successful)
        {
            return invalid_operation_audit();
        }
        missing_operation |= !check.blocked
            && ((requires_method && !check.method_observed) || !check.result_observed);
    }
    let mut verdict = model.verdict.normalized();
    if missing_operation {
        verdict.pass = false;
        if !verdict
            .missing_evidence_fields
            .iter()
            .any(|field| field == "requested_result")
        {
            verdict
                .missing_evidence_fields
                .push("requested_result".to_string());
        }
        verdict.should_retry = true;
        verdict.confidence = verdict.confidence.max(0.55);
        if verdict.answer_incomplete_reason.is_empty() {
            verdict.answer_incomplete_reason = "requested_operation_not_observed".to_string();
        }
    }
    verdict
}

#[cfg(test)]
#[path = "operation_audit_tests.rs"]
mod tests;
