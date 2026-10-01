use serde::Deserialize;

use crate::answer_verifier::AnswerVerifierOut;
use crate::task_journal::TaskJournal;

#[derive(Debug, Deserialize)]
pub(super) struct ModelVerifierOut {
    #[serde(flatten)]
    pub(super) verdict: AnswerVerifierOut,
    pub(super) operation_checks: Vec<OperationCheck>,
    pub(super) output_field_checks: Vec<OutputFieldCheck>,
    #[serde(default)]
    pub(super) unsupported_claims: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub(super) struct OutputFieldCheck {
    pub(super) requested_field: String,
    pub(super) exact_label_present: bool,
}

pub(super) fn host_output_field_checks(
    output_contract: &crate::IntentOutputContract,
    candidate_answer: &str,
) -> Vec<OutputFieldCheck> {
    if !output_contract.requests_exact_structured_fields() {
        return Vec::new();
    }
    output_contract
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(crate::machine_selector::exact_machine_field_selector)
        .unwrap_or_default()
        .into_iter()
        .map(|requested_field| OutputFieldCheck {
            exact_label_present: candidate_answer.contains(&requested_field),
            requested_field,
        })
        .collect()
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
    fn missing_evidence_field(&self) -> String {
        format!("required_dispatch:{}:{}", self.action_type, self.action_ref)
    }

    fn matches(&self, operation: &serde_json::Value) -> bool {
        let field = |name| operation.get(name).and_then(serde_json::Value::as_str);
        match self.action_type.as_str() {
            "call_capability" => {
                field("resolved_capability") == Some(self.action_ref.as_str())
                    || (field("requested_action_type") == Some("call_capability")
                        && field("requested_capability") == Some(self.action_ref.as_str()))
                    || operation
                        .get("runtime_managed_capabilities")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|capabilities| {
                            capabilities.iter().any(|capability| {
                                capability.as_str() == Some(self.action_ref.as_str())
                            })
                        })
                    || executed_skill_action_alias_matches(&self.action_ref, operation)
            }
            "call_tool" | "call_skill" => {
                field("requested_action_type") == Some(self.action_type.as_str())
                    && field("requested_action_ref") == Some(self.action_ref.as_str())
            }
            _ => false,
        }
    }
}

fn executed_skill_action_alias_matches(action_ref: &str, operation: &serde_json::Value) -> bool {
    let Some(executed_skill) = operation
        .get("executed_skill")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let Some((alias_skill, alias_action)) = action_ref.rsplit_once('.') else {
        return false;
    };
    if alias_skill != executed_skill {
        return false;
    }
    ["resolved_capability", "requested_capability"]
        .into_iter()
        .filter_map(|field| operation.get(field).and_then(serde_json::Value::as_str))
        .filter_map(|capability| capability.rsplit_once('.').map(|(_, action)| action))
        .any(|action| action == alias_action)
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
    let mut missing_dispatches = std::collections::BTreeSet::new();
    for check in model.operation_checks {
        if check.requested_operation.trim().is_empty() {
            return invalid_operation_audit();
        }
        if !check.applicable
            && check.evidence_step_ids.is_empty()
            && !check.method_observed
            && check.result_observed
            && !check.blocked
        {
            // Models occasionally encode a prohibition/absence constraint as
            // an inapplicable operation row. It proves nothing and must not
            // invalidate an otherwise independently audited verdict.
            continue;
        }
        if check.required_dispatches.is_empty() && check.evidence_step_ids.is_empty() {
            // Summarizing or formatting already-fetched results is a verdict
            // concern. An empty row cannot prove or disprove execution, so its
            // method/result booleans are non-authoritative. A blocker or an
            // inapplicable branch still requires concrete evidence.
            if !check.applicable || check.blocked {
                return invalid_operation_audit();
            }
            continue;
        }
        let mut successful = false;
        let mut failed = false;
        let mut matching_success = false;
        let mut matching_failure = false;
        for id in &check.evidence_step_ids {
            let matching_attempts = operations
                .iter()
                .filter(|step| {
                    step.get("step_id").and_then(serde_json::Value::as_str) == Some(id.as_str())
                })
                .collect::<Vec<_>>();
            if matching_attempts.is_empty() {
                return invalid_operation_audit();
            }
            // A retry keeps the logical step ID, so audit every recorded
            // attempt instead of letting the first failed attempt hide a
            // later successful execution of the same dispatch.
            for step in matching_attempts {
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
        if !check.blocked && requires_method && !check.method_observed {
            missing_dispatches.extend(
                check
                    .required_dispatches
                    .iter()
                    .map(RequiredDispatch::missing_evidence_field),
            );
        }
    }
    let mut output_format_gap = false;
    let mut audited_output_fields = std::collections::BTreeSet::new();
    for check in model.output_field_checks {
        let requested_field = check.requested_field.trim();
        if requested_field.is_empty() || !audited_output_fields.insert(requested_field.to_string())
        {
            return invalid_operation_audit();
        }
        output_format_gap |= !check.exact_label_present;
    }
    let has_unsupported_claims = !model.unsupported_claims.is_empty();
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
        for dispatch in missing_dispatches {
            if !verdict
                .missing_evidence_fields
                .iter()
                .any(|field| field == &dispatch)
            {
                verdict.missing_evidence_fields.push(dispatch);
            }
        }
        verdict.should_retry = true;
        verdict.confidence = verdict.confidence.max(0.55);
        if verdict.answer_incomplete_reason.is_empty() {
            verdict.answer_incomplete_reason = "requested_operation_not_observed".to_string();
        }
    }
    if has_unsupported_claims {
        verdict.pass = false;
        if !verdict
            .missing_evidence_fields
            .iter()
            .any(|field| field == "unsupported_claims")
        {
            verdict
                .missing_evidence_fields
                .push("unsupported_claims".to_string());
        }
        verdict.should_retry = true;
        verdict.confidence = verdict.confidence.max(0.55);
        if verdict.answer_incomplete_reason.is_empty() {
            verdict.answer_incomplete_reason = "unsupported_claims_observed".to_string();
        }
    }
    if output_format_gap {
        verdict.pass = false;
        if !verdict
            .missing_evidence_fields
            .iter()
            .any(|field| field == "output_format")
        {
            verdict
                .missing_evidence_fields
                .push("output_format".to_string());
        }
        verdict.should_retry = true;
        verdict.confidence = verdict.confidence.max(0.55);
        if verdict.answer_incomplete_reason.is_empty() {
            verdict.answer_incomplete_reason = "requested_output_field_label_missing".to_string();
        }
    }
    verdict
}

pub(super) fn validate_output_field_audit(
    mut verdict: AnswerVerifierOut,
    checks: &[OutputFieldCheck],
    candidate_answer: &str,
) -> AnswerVerifierOut {
    let output_format_gap = checks.iter().any(|check| {
        !check.exact_label_present || !candidate_answer.contains(check.requested_field.as_str())
    });
    if output_format_gap {
        verdict.pass = false;
        if !verdict
            .missing_evidence_fields
            .iter()
            .any(|field| field == "output_format")
        {
            verdict
                .missing_evidence_fields
                .push("output_format".to_string());
        }
        verdict.should_retry = true;
        verdict.confidence = verdict.confidence.max(0.55);
        if verdict.answer_incomplete_reason.is_empty() {
            verdict.answer_incomplete_reason = "requested_output_field_label_missing".to_string();
        }
    }
    verdict
}

#[cfg(test)]
#[path = "operation_audit_tests.rs"]
mod tests;
