use serde_json::{json, Value};

use super::{TaskJournalAnswerVerifierSummary, TaskJournalRolloutAttribution};

impl TaskJournalRolloutAttribution {
    fn deterministic_boundary_context(
        decision_source: &'static str,
        rewrite_reason_code: impl Into<String>,
        input_contract_ref: impl Into<String>,
        output_contract_ref: impl Into<String>,
    ) -> Value {
        json!({
            "schema_version": 1,
            "decision_source": decision_source,
            "rewrite_reason_code": rewrite_reason_code.into(),
            "semantic_control_state": "none",
            "input_contract_ref": input_contract_ref.into(),
            "output_contract_ref": output_contract_ref.into(),
        })
    }

    pub(crate) fn answer_verifier_required_evidence_block(
        summary: Option<&TaskJournalAnswerVerifierSummary>,
    ) -> Self {
        Self {
            switch_name: "answer_verifier_enforce_required_scope".to_string(),
            event: "answer_verifier_required_evidence_block".to_string(),
            outcome: "blocked".to_string(),
            reason_code: Some("answer_verifier_required_evidence_block".to_string()),
            owner_layer: Some("answer_verifier".to_string()),
            decision: Some("blocked".to_string()),
            failure_attribution: Some(
                crate::evidence_policy::FailureAttribution::ContractGap
                    .as_str()
                    .to_string(),
            ),
            missing_evidence_fields: summary
                .map(|summary| summary.missing_evidence_fields.clone())
                .unwrap_or_default(),
            confidence: summary.map(|summary| summary.confidence),
            boundary_context: Some(Self::deterministic_boundary_context(
                "contract_boundary",
                "answer_verifier_required_evidence_block",
                "answer_verifier_summary",
                "required_evidence_contract",
            )),
            ..Self::default()
        }
    }

    pub(crate) fn registry_idempotency_guard_block(
        reason_code: impl Into<String>,
        skill: impl Into<String>,
        action: Option<String>,
        dedup_scope: impl Into<String>,
        fingerprint: impl Into<String>,
        repeat_count: Option<usize>,
        limit: Option<usize>,
    ) -> Self {
        let reason_code = reason_code.into();
        Self {
            switch_name: "registry_idempotency_guard_scope".to_string(),
            event: "registry_idempotency_guard_block".to_string(),
            outcome: "blocked".to_string(),
            reason_code: Some(reason_code.clone()),
            owner_layer: Some("execution_guard".to_string()),
            decision: Some("blocked".to_string()),
            skill: Some(skill.into()),
            action,
            dedup_scope: Some(dedup_scope.into()),
            fingerprint: Some(fingerprint.into()),
            repeat_count,
            limit,
            boundary_context: Some(Self::deterministic_boundary_context(
                "safety_policy",
                reason_code,
                "registry_idempotency_policy",
                "side_effect_fingerprint_guard",
            )),
            ..Self::default()
        }
    }

    pub(crate) fn document_heading_answer_verifier_recovery(
        summary: Option<&TaskJournalAnswerVerifierSummary>,
    ) -> Self {
        Self {
            switch_name: "deterministic_answer_recovery".to_string(),
            event: "document_heading_answer_verifier_recovery".to_string(),
            outcome: "recovered".to_string(),
            reason_code: Some(
                "document_heading_recovered_from_observed_markdown_heading".to_string(),
            ),
            owner_layer: Some("answer_verifier_recovery".to_string()),
            decision: Some("recovered".to_string()),
            missing_evidence_fields: summary
                .map(|summary| summary.missing_evidence_fields.clone())
                .unwrap_or_default(),
            confidence: summary.map(|summary| summary.confidence),
            final_outcome: Some("success".to_string()),
            boundary_context: Some(Self::deterministic_boundary_context(
                "recovery_boundary",
                "document_heading_recovered_from_observed_markdown_heading",
                "answer_verifier_summary",
                "observed_markdown_heading",
            )),
            ..Self::default()
        }
    }
}
