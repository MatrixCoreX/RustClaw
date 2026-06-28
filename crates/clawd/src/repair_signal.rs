use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepairSignalSource {
    Verifier,
    Executor,
    AnswerVerifier,
    Runtime,
}

impl RepairSignalSource {
    pub(crate) fn as_token(self) -> &'static str {
        match self {
            Self::Verifier => "verifier",
            Self::Executor => "executor",
            Self::AnswerVerifier => "answer_verifier",
            Self::Runtime => "runtime",
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
    pub(crate) repair_attempt: Option<usize>,
    pub(crate) round_no: Option<usize>,
    pub(crate) retryable: Option<bool>,
    pub(crate) no_progress_count: Option<usize>,
    pub(crate) missing_fields: Vec<String>,
    pub(crate) rejected_action: Option<String>,
    pub(crate) suggested_contract_action: Option<String>,
    pub(crate) forbidden_repeat_fingerprint: Option<String>,
    pub(crate) side_effect_fingerprint: Option<String>,
    pub(crate) max_attempts: Option<usize>,
    pub(crate) budget_exhausted: Option<bool>,
    pub(crate) permission_decision: Option<Value>,
    pub(crate) contract_failure_policy: Option<Value>,
    pub(crate) provider_status: Option<Value>,
    pub(crate) checkpoint_id: Option<String>,
    pub(crate) resume_entrypoint: Option<String>,
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
            repair_attempt: None,
            round_no: None,
            retryable: Some(verifier_issue_retryable(kind)),
            no_progress_count: None,
            missing_fields: Vec::new(),
            rejected_action: None,
            suggested_contract_action: None,
            forbidden_repeat_fingerprint: None,
            side_effect_fingerprint: None,
            max_attempts: None,
            budget_exhausted: None,
            permission_decision: None,
            contract_failure_policy: None,
            provider_status: None,
            checkpoint_id: None,
            resume_entrypoint: None,
            detail: (!detail.trim().is_empty()).then(|| crate::truncate_for_log(detail)),
        }
    }

    pub(crate) fn from_answer_verifier_parts(
        missing_evidence_fields: &[String],
        should_retry: bool,
        confidence: f64,
    ) -> Self {
        let status_code = if missing_evidence_fields.is_empty() {
            "answer_verifier_retry_required"
        } else {
            "missing_required_evidence"
        };
        Self {
            source: RepairSignalSource::AnswerVerifier,
            owner_layer: Some("answer_verifier"),
            step_id: None,
            kind: Some("answer_verifier_gap".to_string()),
            status_code: status_code.to_string(),
            message_key: format!("clawd.answer_verifier.{status_code}"),
            reason_code: Some("answer_verifier_missing_evidence_repair".to_string()),
            failure_attribution: "contract_gap".to_string(),
            repair_attempt: None,
            round_no: None,
            retryable: Some(should_retry),
            no_progress_count: None,
            missing_fields: missing_evidence_fields
                .iter()
                .map(|field| field.trim().to_string())
                .filter(|field| !field.is_empty())
                .collect(),
            rejected_action: None,
            suggested_contract_action: None,
            forbidden_repeat_fingerprint: None,
            side_effect_fingerprint: None,
            max_attempts: None,
            budget_exhausted: None,
            permission_decision: None,
            contract_failure_policy: None,
            provider_status: None,
            checkpoint_id: None,
            resume_entrypoint: None,
            detail: Some(format!("confidence={:.3}", confidence.clamp(0.0, 1.0))),
        }
    }

    pub(crate) fn from_checkpoint_resume_parts(
        checkpoint_id: &str,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint,
        signal: &str,
    ) -> Self {
        Self {
            source: RepairSignalSource::Runtime,
            owner_layer: Some("task_lifecycle"),
            step_id: None,
            kind: Some("agent_loop_stop_signal".to_string()),
            status_code: "agent_loop_checkpoint".to_string(),
            message_key: "clawd.task.checkpoint_resume".to_string(),
            reason_code: Some("checkpoint_resume_recovery".to_string()),
            failure_attribution: "runtime_budget".to_string(),
            repair_attempt: None,
            round_no: None,
            retryable: Some(true),
            no_progress_count: None,
            missing_fields: Vec::new(),
            rejected_action: None,
            suggested_contract_action: None,
            forbidden_repeat_fingerprint: None,
            side_effect_fingerprint: None,
            max_attempts: None,
            budget_exhausted: None,
            permission_decision: None,
            contract_failure_policy: None,
            provider_status: None,
            checkpoint_id: (!checkpoint_id.trim().is_empty())
                .then(|| checkpoint_id.trim().to_string()),
            resume_entrypoint: Some(resume_entrypoint.as_str().to_string()),
            detail: (!signal.trim().is_empty()).then(|| signal.trim().to_string()),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        let boundary = crate::repair_boundary_inventory::repair_inventory_for_signal(
            self.reason_code.as_deref(),
            Some(&self.status_code),
            self.owner_layer,
            Some(self.source.as_token()),
        );
        let envelope = self.to_envelope_json(boundary);
        json!({
            "schema_version": 1,
            "source": self.source.as_token(),
            "owner_layer": self.owner_layer,
            "repair_class": boundary.map(|item| item.repair_class.as_token()),
            "next_recovery_kind": boundary.map(|item| item.next_recovery_kind),
            "repair_boundary": boundary.map(|item| item.trace_value()),
            "repair_envelope": envelope,
            "step_id": self.step_id.as_deref(),
            "kind": self.kind.as_deref(),
            "status_code": self.status_code,
            "message_key": self.message_key,
            "reason_code": self.reason_code.as_deref(),
            "failure_attribution": self.failure_attribution,
            "repair_attempt": self.repair_attempt,
            "round_no": self.round_no,
            "retryable": self.retryable,
            "no_progress_count": self.no_progress_count,
            "missing_fields": self.missing_fields,
            "rejected_action": self.rejected_action.as_deref(),
            "suggested_contract_action": self.suggested_contract_action.as_deref(),
            "forbidden_repeat_fingerprint": self.forbidden_repeat_fingerprint.as_deref(),
            "side_effect_fingerprint": self.side_effect_fingerprint.as_deref(),
            "max_attempts": self.max_attempts,
            "budget_exhausted": self.budget_exhausted,
            "permission_decision": self.permission_decision.as_ref(),
            "contract_failure_policy": self.contract_failure_policy.as_ref(),
            "provider_status": self.provider_status.as_ref(),
            "checkpoint_id": self.checkpoint_id.as_deref(),
            "resume_entrypoint": self.resume_entrypoint.as_deref(),
            "signal": self.detail.as_deref(),
            "detail": self.detail.as_deref(),
        })
    }

    fn to_envelope_json(
        &self,
        boundary: Option<crate::repair_boundary_inventory::RepairBoundaryInventoryItem>,
    ) -> Value {
        let issue_codes = self.issue_codes();
        let permission_decision = self.permission_decision.as_ref();
        let repair_class = boundary
            .map(|item| item.repair_class.as_token())
            .unwrap_or("loop_bounded_recovery");
        let next_recovery_kind = boundary
            .map(|item| item.next_recovery_kind)
            .unwrap_or("replan");
        json!({
            "schema_version": 1,
            "repair_source": self.source.as_token(),
            "repair_class": repair_class,
            "repair_attempt": self.repair_attempt,
            "round_no": self.round_no,
            "step_id": self.step_id.as_deref(),
            "owner_layer": self.owner_layer,
            "issue_codes": issue_codes,
            "missing_evidence": self.missing_fields,
            "failed_action_ref": self.rejected_action.as_deref(),
            "blocked_action_ref": self.rejected_action.as_deref(),
            "observed_action_refs": Vec::<String>::new(),
            "verifier_confidence": answer_verifier_confidence(self.detail.as_deref()),
            "permission_decision": self.permission_decision.as_ref(),
            "contract_failure_policy": self.contract_failure_policy.as_ref(),
            "risk_level": envelope_machine_field(permission_decision, "risk_level"),
            "action_effect": envelope_machine_field(permission_decision, "action_effect"),
            "requires_confirmation": envelope_machine_field(permission_decision, "requires_confirmation"),
            "needs_confirmation": envelope_machine_field(permission_decision, "needs_confirmation"),
            "dry_run_required": envelope_machine_field(permission_decision, "dry_run_required"),
            "provider_status": self.provider_status.as_ref(),
            "retryable": self.retryable,
            "no_progress_count": self.no_progress_count,
            "attempt_fingerprint": self.forbidden_repeat_fingerprint.as_deref(),
            "side_effect_fingerprint": self.side_effect_fingerprint.as_deref(),
            "max_attempts": self.max_attempts,
            "budget_exhausted": self.budget_exhausted,
            "checkpoint_id": self.checkpoint_id.as_deref(),
            "resume_entrypoint": self.resume_entrypoint.as_deref(),
            "next_recovery_kind": next_recovery_kind,
            "message_key": self.message_key,
            "error_code": self.status_code,
        })
    }

    fn issue_codes(&self) -> Vec<String> {
        let mut codes = Vec::new();
        if let Some(reason_code) = self.reason_code.as_deref().map(str::trim) {
            if !reason_code.is_empty() {
                codes.push(reason_code.to_string());
            }
        }
        let status_code = self.status_code.trim();
        if !status_code.is_empty() && !codes.iter().any(|code| code == status_code) {
            codes.push(status_code.to_string());
        }
        codes
    }

    pub(crate) fn with_forbidden_repeat_fingerprint(mut self, fingerprint: Option<String>) -> Self {
        self.forbidden_repeat_fingerprint = fingerprint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        self
    }

    pub(crate) fn with_loop_budget(
        mut self,
        round_no: usize,
        max_attempts: usize,
        no_progress_count: usize,
        budget_exhausted: bool,
    ) -> Self {
        self.repair_attempt = Some(round_no);
        self.round_no = Some(round_no);
        self.max_attempts = Some(max_attempts);
        self.no_progress_count = Some(no_progress_count);
        self.budget_exhausted = Some(budget_exhausted);
        self
    }

    pub(crate) fn with_rejected_action(mut self, rejected_action: Option<String>) -> Self {
        self.rejected_action = rejected_action
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        self
    }

    pub(crate) fn with_contract_failure_policy(mut self, contract_policy: Option<Value>) -> Self {
        self.permission_decision = contract_policy
            .as_ref()
            .and_then(|policy| policy.get("permission_decision").cloned());
        self.rejected_action = self.rejected_action.or_else(|| {
            contract_policy
                .as_ref()
                .and_then(|policy| policy.get("original_action_ref"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });
        self.suggested_contract_action = self.suggested_contract_action.or_else(|| {
            contract_policy
                .as_ref()
                .and_then(|policy| policy.get("replacement_action_ref"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });
        self.contract_failure_policy = contract_policy;
        self
    }

    pub(crate) fn with_provider_status(mut self, provider_status: Option<Value>) -> Self {
        self.provider_status = provider_status;
        self
    }
}

fn envelope_machine_field(source: Option<&Value>, key: &str) -> Value {
    source
        .and_then(|value| value.get(key).cloned())
        .unwrap_or(Value::Null)
}

fn answer_verifier_confidence(detail: Option<&str>) -> Value {
    let Some(detail) = detail else {
        return Value::Null;
    };
    let Some(raw) = detail.trim().strip_prefix("confidence=") else {
        return Value::Null;
    };
    raw.parse::<f64>()
        .ok()
        .and_then(serde_json::Number::from_f64)
        .map(Value::Number)
        .unwrap_or(Value::Null)
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
        assert_eq!(value.get("schema_version").and_then(Value::as_u64), Some(1));
        assert_eq!(
            value.get("owner_layer").and_then(Value::as_str),
            Some("plan_verifier")
        );
        assert_eq!(
            value.get("repair_class").and_then(Value::as_str),
            Some("loop_bounded_recovery")
        );
        assert_eq!(
            value.get("next_recovery_kind").and_then(Value::as_str),
            Some("replan")
        );
        assert!(value.get("repair_boundary").is_some());
        let envelope = value
            .get("repair_envelope")
            .and_then(Value::as_object)
            .expect("repair envelope object");
        assert_eq!(
            envelope.get("repair_source").and_then(Value::as_str),
            Some("verifier")
        );
        assert_eq!(
            envelope.get("repair_class").and_then(Value::as_str),
            Some("loop_bounded_recovery")
        );
        assert_eq!(
            envelope.get("next_recovery_kind").and_then(Value::as_str),
            Some("replan")
        );
        assert!(envelope
            .get("issue_codes")
            .and_then(Value::as_array)
            .is_some_and(|codes| codes
                .iter()
                .any(|code| code.as_str() == Some("verify_missing_required_arg"))));
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
        assert_eq!(
            value.get("repair_class").and_then(Value::as_str),
            Some("permission_contract_repair")
        );
        assert_eq!(
            value.get("next_recovery_kind").and_then(Value::as_str),
            Some("needs_user")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/repair_class")
                .and_then(Value::as_str),
            Some("permission_contract_repair")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/next_recovery_kind")
                .and_then(Value::as_str),
            Some("needs_user")
        );
    }

    #[test]
    fn answer_verifier_gap_maps_to_repair_envelope() {
        let signal = RepairSignal::from_answer_verifier_parts(
            &["content_excerpt".to_string(), "path".to_string()],
            true,
            0.91,
        );
        let value = signal.to_json();

        assert_eq!(
            value.get("source").and_then(Value::as_str),
            Some("answer_verifier")
        );
        assert_eq!(
            value.get("repair_class").and_then(Value::as_str),
            Some("loop_bounded_recovery")
        );
        assert_eq!(
            value.get("next_recovery_kind").and_then(Value::as_str),
            Some("replan")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/missing_evidence/0")
                .and_then(Value::as_str),
            Some("content_excerpt")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/verifier_confidence")
                .and_then(Value::as_f64),
            Some(0.91)
        );
        assert!(value
            .pointer("/repair_envelope/max_attempts")
            .is_some_and(Value::is_null));
        assert_eq!(
            value
                .pointer("/repair_envelope/issue_codes/0")
                .and_then(Value::as_str),
            Some("answer_verifier_missing_evidence_repair")
        );
    }

    #[test]
    fn repair_envelope_excludes_user_visible_text_fields() {
        let signal = RepairSignal::from_verifier_issue_parts(
            "step_1",
            crate::verifier::VerifyIssueKind::MissingRequiredArg,
            "missing path",
        );
        let envelope = signal
            .to_json()
            .get("repair_envelope")
            .cloned()
            .expect("repair envelope");

        for forbidden in [
            "text",
            "error_text",
            "detail",
            "signal",
            "user_message",
            "localized_reply_text",
        ] {
            assert!(
                envelope.get(forbidden).is_none(),
                "repair_envelope_forbidden_field={forbidden}"
            );
        }
    }

    #[test]
    fn rejected_action_is_exposed_as_failed_action_ref() {
        let signal = RepairSignal::from_verifier_issue_parts(
            "step_3",
            crate::verifier::VerifyIssueKind::ContractActionRejected,
            "contract rejected",
        )
        .with_rejected_action(Some("fs_basic.read_text_range".to_string()));

        let value = signal.to_json();
        assert_eq!(
            value
                .pointer("/repair_envelope/failed_action_ref")
                .and_then(Value::as_str),
            Some("fs_basic.read_text_range")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/blocked_action_ref")
                .and_then(Value::as_str),
            Some("fs_basic.read_text_range")
        );
    }

    #[test]
    fn contract_policy_is_exposed_in_repair_envelope() {
        let signal = RepairSignal::from_verifier_issue_parts(
            "step_4",
            crate::verifier::VerifyIssueKind::ContractActionRejected,
            "contract rejected",
        )
        .with_contract_failure_policy(Some(json!({
            "original_action_ref": "run_cmd",
            "replacement_action_ref": "fs_basic.list_dir",
            "decision": "rejected_not_allowed",
            "permission_decision": {
                "allowed": false,
                "denied_by_policy": true,
                "risk_level": "high",
                "action_effect": "mutate",
                "needs_confirmation": true,
                "dry_run_required": false
            }
        })));

        let value = signal.to_json();
        assert_eq!(
            value
                .pointer("/repair_envelope/failed_action_ref")
                .and_then(Value::as_str),
            Some("run_cmd")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/permission_decision/denied_by_policy")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/risk_level")
                .and_then(Value::as_str),
            Some("high")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/action_effect")
                .and_then(Value::as_str),
            Some("mutate")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/needs_confirmation")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/contract_failure_policy/replacement_action_ref")
                .and_then(Value::as_str),
            Some("fs_basic.list_dir")
        );
    }

    #[test]
    fn checkpoint_resume_signal_exposes_resume_envelope() {
        let signal = RepairSignal::from_checkpoint_resume_parts(
            "ckpt-123",
            crate::task_lifecycle::ResumeEntrypoint::NextPlannerRound,
            "max_rounds",
        );
        let value = signal.to_json();

        assert_eq!(value.get("source").and_then(Value::as_str), Some("runtime"));
        assert_eq!(
            value.get("repair_class").and_then(Value::as_str),
            Some("checkpoint_resume_repair")
        );
        assert_eq!(
            value.get("signal").and_then(Value::as_str),
            Some("max_rounds")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/checkpoint_id")
                .and_then(Value::as_str),
            Some("ckpt-123")
        );
        assert_eq!(
            value
                .pointer("/repair_envelope/resume_entrypoint")
                .and_then(Value::as_str),
            Some("next_planner_round")
        );
    }
}
