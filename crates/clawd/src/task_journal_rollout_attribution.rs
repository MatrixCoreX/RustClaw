use serde_json::{json, Value};

use super::decision_envelope::{
    agent_action_capability_delta, agent_loop_decision_envelope_json,
    first_non_think_action_capability_ref, first_non_think_action_decision,
    output_contract_ref_for_route, route_gate_agent_decision_delta,
};
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

    pub(crate) fn dispatch_boundary_attribution(
        route: &crate::RouteResult,
        event: impl Into<String>,
        old_owner: impl Into<String>,
        new_owner: impl Into<String>,
        chosen_path: impl Into<String>,
        rollback_token: impl Into<String>,
    ) -> Self {
        let event = event.into();
        let old_owner = old_owner.into();
        let new_owner = new_owner.into();
        let chosen_path = chosen_path.into();
        let rollback_token = rollback_token.into();
        Self {
            switch_name: "semantic_route_authority".to_string(),
            event,
            outcome: "observed".to_string(),
            reason_code: Some("dispatch_boundary_attribution_recorded".to_string()),
            owner_layer: Some(new_owner.clone()),
            decision: Some(chosen_path.clone()),
            initial_gate_ref: Some(route.gate_kind().as_str().to_string()),
            initial_hint_ref: Some(
                route
                    .legacy_first_layer_decision_for_trace()
                    .as_str()
                    .to_string(),
            ),
            route_gate_kind: Some(route.gate_kind().as_str().to_string()),
            old_first_layer_decision: Some(
                route
                    .legacy_first_layer_decision_for_trace()
                    .as_str()
                    .to_string(),
            ),
            agent_decision: Some(chosen_path.clone()),
            decision_delta: Some("boundary_shortcut_retained".to_string()),
            route_layer_that_disagreed: Some(old_owner.clone()),
            risk_level: Some(route.risk_ceiling.as_str().to_string()),
            output_contract_ref: Some(output_contract_ref_for_route(route)),
            boundary_context: Some(json!({
                "schema_version": 1,
                "decision_source": "compat_trace",
                "rewrite_reason_code": "dispatch_boundary_attribution_recorded",
                "semantic_control_state": "none",
                "old_owner": old_owner,
                "new_owner": new_owner,
                "chosen_path": chosen_path,
                "rollback_token": rollback_token,
                "input_contract_ref": "semantic_route_authority",
                "output_contract_ref": "dispatch_boundary_trace",
            })),
            ..Self::default()
        }
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

    pub(crate) fn selected_contract_structured_evidence_block(
        summary: Option<&TaskJournalAnswerVerifierSummary>,
        selected_class: impl Into<String>,
    ) -> Self {
        Self {
            switch_name: "structured_evidence_required_for_selected_contracts".to_string(),
            event: "selected_contract_structured_evidence_block".to_string(),
            outcome: "blocked".to_string(),
            reason_code: Some("selected_contract_missing_structured_evidence".to_string()),
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
            output_contract_ref: Some(selected_class.into()),
            boundary_context: Some(Self::deterministic_boundary_context(
                "contract_boundary",
                "selected_contract_missing_structured_evidence",
                "answer_verifier_summary",
                "selected_contract_structured_evidence",
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

    pub(crate) fn agent_decides_shadow_snapshot(
        route: &crate::RouteResult,
        budget_profile: impl Into<String>,
        boundary_context: Option<Value>,
    ) -> Self {
        let contract = crate::TaskContract::from_route_result(route);
        let required_evidence = contract.required_evidence_fields.clone();
        Self {
            switch_name: "semantic_route_authority".to_string(),
            event: "agent_decides_shadow_snapshot".to_string(),
            outcome: "shadow_only".to_string(),
            reason_code: Some("agent_decides_shadow_not_evaluated".to_string()),
            owner_layer: Some("agent_loop_shadow".to_string()),
            decision: Some(route.gate_kind().as_str().to_string()),
            initial_gate_ref: Some(route.gate_kind().as_str().to_string()),
            initial_hint_ref: Some(
                route
                    .legacy_first_layer_decision_for_trace()
                    .as_str()
                    .to_string(),
            ),
            route_gate_kind: Some(route.gate_kind().as_str().to_string()),
            old_first_layer_decision: Some(
                route
                    .legacy_first_layer_decision_for_trace()
                    .as_str()
                    .to_string(),
            ),
            agent_decision: Some("not_evaluated".to_string()),
            decision_delta: Some("not_evaluated".to_string()),
            missing_slots: contract.missing_parameters,
            required_evidence: required_evidence.clone(),
            risk_level: Some(route.risk_ceiling.as_str().to_string()),
            output_contract_ref: Some(output_contract_ref_for_route(route)),
            old_required_evidence: required_evidence,
            agent_required_evidence: Vec::new(),
            capability_delta: Some("not_evaluated".to_string()),
            risk_delta: Some("not_evaluated".to_string()),
            output_contract_delta: Some("not_evaluated".to_string()),
            budget_profile: Some(budget_profile.into()),
            boundary_context,
            ..Self::default()
        }
    }

    pub(crate) fn agent_decides_shadow_first_action(
        route: &crate::RouteResult,
        budget_profile: impl Into<String>,
        actions: &[crate::AgentAction],
        boundary_context: Option<Value>,
    ) -> Self {
        let contract = crate::TaskContract::from_route_result(route);
        let agent_decision = first_non_think_action_decision(actions);
        let decision_delta = route_gate_agent_decision_delta(route.gate_kind(), agent_decision);
        let route_layer_that_disagreed =
            (decision_delta == "different_gate").then(|| "route_gate_vs_agent_loop".to_string());
        let required_evidence = contract.required_evidence_fields.clone();
        let output_contract_ref = output_contract_ref_for_route(route);
        Self {
            switch_name: "semantic_route_authority".to_string(),
            event: "agent_decides_shadow_first_action".to_string(),
            outcome: "shadow_only".to_string(),
            reason_code: Some("agent_decides_shadow_delta_observed".to_string()),
            owner_layer: Some("agent_loop_shadow".to_string()),
            decision: Some(agent_decision.to_string()),
            capability_ref: first_non_think_action_capability_ref(actions).map(str::to_string),
            initial_gate_ref: Some(route.gate_kind().as_str().to_string()),
            initial_hint_ref: Some(
                route
                    .legacy_first_layer_decision_for_trace()
                    .as_str()
                    .to_string(),
            ),
            route_gate_kind: Some(route.gate_kind().as_str().to_string()),
            old_first_layer_decision: Some(
                route
                    .legacy_first_layer_decision_for_trace()
                    .as_str()
                    .to_string(),
            ),
            agent_decision: Some(agent_decision.to_string()),
            decision_delta: Some(decision_delta.to_string()),
            route_layer_that_disagreed,
            missing_slots: contract.missing_parameters,
            required_evidence: required_evidence.clone(),
            risk_level: Some(route.risk_ceiling.as_str().to_string()),
            output_contract_ref: Some(output_contract_ref.clone()),
            old_required_evidence: required_evidence.clone(),
            agent_required_evidence: required_evidence,
            capability_delta: Some(agent_action_capability_delta(actions).to_string()),
            risk_delta: Some("not_evaluated".to_string()),
            output_contract_delta: Some("not_evaluated".to_string()),
            budget_profile: Some(budget_profile.into()),
            boundary_context,
            decision_envelope: Some(agent_loop_decision_envelope_json(
                route,
                actions,
                &output_contract_ref,
                "planner_first_action_shadow",
                "planner_loop_shadow",
            )),
            ..Self::default()
        }
    }
}
