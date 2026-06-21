use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepairSignalSource {
    Verifier,
    Executor,
    AnswerVerifier,
}

impl RepairSignalSource {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Verifier => "verifier",
            Self::Executor => "executor",
            Self::AnswerVerifier => "answer_verifier",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepairSignal {
    pub(crate) source: RepairSignalSource,
    pub(crate) owner_layer: Option<&'static str>,
    pub(crate) step_id: Option<String>,
    pub(crate) kind: Option<String>,
    pub(crate) status_code: String,
    pub(crate) message_key: String,
    pub(crate) reason_code: Option<String>,
    pub(crate) failure_attribution: String,
    pub(crate) retryable: Option<bool>,
    pub(crate) missing_fields: Vec<String>,
    pub(crate) rejected_action: Option<String>,
    pub(crate) suggested_contract_action: Option<String>,
    pub(crate) forbidden_repeat_fingerprint: Option<String>,
    pub(crate) detail: Option<String>,
}

impl RepairSignal {
    pub(crate) fn from_verifier_issue_parts(
        step_id: &str,
        kind: crate::verifier::VerifyIssueKind,
        detail: &str,
    ) -> Self {
        Self {
            source: RepairSignalSource::Verifier,
            owner_layer: Some("plan_verifier"),
            step_id: (!step_id.trim().is_empty()).then(|| step_id.trim().to_string()),
            kind: Some(kind.as_str().to_string()),
            status_code: kind.status_code().to_string(),
            message_key: kind.message_key().to_string(),
            reason_code: Some(kind.reason_code().to_string()),
            failure_attribution: kind.failure_attribution().as_str().to_string(),
            retryable: Some(verifier_issue_retryable(kind)),
            missing_fields: Vec::new(),
            rejected_action: None,
            suggested_contract_action: None,
            forbidden_repeat_fingerprint: None,
            detail: (!detail.trim().is_empty()).then(|| crate::truncate_for_log(detail)),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({
            "source": self.source.as_token(),
            "owner_layer": self.owner_layer,
            "step_id": self.step_id.as_deref(),
            "kind": self.kind.as_deref(),
            "status_code": self.status_code,
            "message_key": self.message_key,
            "reason_code": self.reason_code.as_deref(),
            "failure_attribution": self.failure_attribution,
            "retryable": self.retryable,
            "missing_fields": self.missing_fields,
            "rejected_action": self.rejected_action.as_deref(),
            "suggested_contract_action": self.suggested_contract_action.as_deref(),
            "forbidden_repeat_fingerprint": self.forbidden_repeat_fingerprint.as_deref(),
            "detail": self.detail.as_deref(),
        })
    }

    pub(crate) fn with_forbidden_repeat_fingerprint(mut self, fingerprint: Option<String>) -> Self {
        self.forbidden_repeat_fingerprint = fingerprint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        self
    }
}

fn verifier_issue_retryable(kind: crate::verifier::VerifyIssueKind) -> bool {
    use crate::verifier::VerifyIssueKind as Kind;
    matches!(
        kind,
        Kind::MissingRequiredArg
            | Kind::UnresolvedTemplateArg
            | Kind::InvalidDependsOn
            | Kind::PrimaryFallbackConflict
            | Kind::RecipeInspectBeforeMutateRequired
            | Kind::RecipeValidationAfterMutateRequired
            | Kind::RecipeTargetScopeRequired
            | Kind::ContractActionRejected
            | Kind::ContractPolicyViolation
            | Kind::ContractPreferredActionAvailable
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifier_issue_maps_to_machine_repair_signal() {
        let signal = RepairSignal::from_verifier_issue_parts(
            "step_1",
            crate::verifier::VerifyIssueKind::MissingRequiredArg,
            "missing path",
        );
        let value = signal.to_json();
        assert_eq!(
            value.get("source").and_then(Value::as_str),
            Some("verifier")
        );
        assert_eq!(
            value.get("owner_layer").and_then(Value::as_str),
            Some("plan_verifier")
        );
        assert_eq!(
            value.get("status_code").and_then(Value::as_str),
            Some("missing_required_arg")
        );
        assert_eq!(
            value.get("message_key").and_then(Value::as_str),
            Some("clawd.verify.missing_required_arg")
        );
        assert_eq!(
            value.get("failure_attribution").and_then(Value::as_str),
            Some("model_error")
        );
        assert_eq!(value.get("retryable").and_then(Value::as_bool), Some(true));
        assert_eq!(
            value
                .get("missing_fields")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn verifier_permission_gate_maps_to_non_retryable_signal() {
        let signal = RepairSignal::from_verifier_issue_parts(
            "step_2",
            crate::verifier::VerifyIssueKind::RiskBudgetExceeded,
            "risk ceiling",
        );
        let value = signal.to_json();
        assert_eq!(
            value.get("status_code").and_then(Value::as_str),
            Some("risk_budget_exceeded")
        );
        assert_eq!(
            value.get("failure_attribution").and_then(Value::as_str),
            Some("permission_denied")
        );
        assert_eq!(value.get("retryable").and_then(Value::as_bool), Some(false));
    }
}
