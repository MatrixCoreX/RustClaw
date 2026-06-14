use super::super::{TargetTaskPolicy, TurnType};
use crate::{
    ActFinalizeStyle, FirstLayerDecision, IntentOutputContract, OutputResponseShape,
    OutputSemanticKind, ScheduleKind,
};
use serde_json::Value;

#[test]
fn unobserved_runtime_status_answer_candidate_promotes_to_evidence_query() {
    let Some(current_user) = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let mut answer_candidate = current_user.trim().to_string();
    let mut state_patch = None;
    let mut decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = ActFinalizeStyle::Plain;
    let mut turn_type = None;
    let mut target_policy = None;

    let reason = super::apply_unobserved_runtime_status_answer_candidate_repair(
        &mut contract,
        &mut answer_candidate,
        &mut state_patch,
        false,
        false,
        ScheduleKind::None,
        Some(crate::execution_recipe::ExecutionRecipeSpec::default()),
        &mut decision,
        &mut finalize_style,
        &mut turn_type,
        &mut target_policy,
    );

    assert_eq!(
        reason,
        Some("unobserved_runtime_status_answer_candidate_requires_evidence")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, ActFinalizeStyle::Plain);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert!(contract.requires_content_evidence);
    assert_eq!(turn_type, Some(TurnType::StatusQuery));
    assert_eq!(target_policy, Some(TargetTaskPolicy::Standalone));
    assert!(answer_candidate.is_empty());
    assert_eq!(
        state_patch
            .as_ref()
            .and_then(|value| value.pointer("/runtime_status_query/kind"))
            .and_then(Value::as_str),
        Some("current_user")
    );
}

#[test]
fn planner_runtime_status_answer_candidate_promotes_to_state_patch() {
    let Some(current_user) = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let mut answer_candidate = current_user.trim().to_string();
    let mut state_patch = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = ActFinalizeStyle::Plain;
    let mut turn_type = Some(TurnType::StatusQuery);
    let mut target_policy = None;

    let reason = super::apply_unobserved_runtime_status_answer_candidate_repair(
        &mut contract,
        &mut answer_candidate,
        &mut state_patch,
        false,
        false,
        ScheduleKind::None,
        Some(crate::execution_recipe::ExecutionRecipeSpec::default()),
        &mut decision,
        &mut finalize_style,
        &mut turn_type,
        &mut target_policy,
    );

    assert_eq!(
        reason,
        Some("unobserved_runtime_status_answer_candidate_requires_evidence")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert!(contract.requires_content_evidence);
    assert_eq!(turn_type, Some(TurnType::StatusQuery));
    assert_eq!(target_policy, Some(TargetTaskPolicy::Standalone));
    assert!(answer_candidate.is_empty());
    assert_eq!(
        state_patch
            .as_ref()
            .and_then(|value| value.pointer("/runtime_status_query/kind"))
            .and_then(Value::as_str),
        Some("current_user")
    );
}

#[test]
fn planner_runtime_status_unobserved_answer_candidate_requires_evidence() {
    let Some(current_user) = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty())
    else {
        return;
    };
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: false,
        ..IntentOutputContract::default()
    };
    let mut answer_candidate = current_user.trim().to_string();
    let mut state_patch = None;
    let mut decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = ActFinalizeStyle::Plain;
    let mut turn_type = None;
    let mut target_policy = None;

    let reason = super::apply_unobserved_runtime_status_answer_candidate_repair(
        &mut contract,
        &mut answer_candidate,
        &mut state_patch,
        false,
        false,
        ScheduleKind::None,
        Some(crate::execution_recipe::ExecutionRecipeSpec::default()),
        &mut decision,
        &mut finalize_style,
        &mut turn_type,
        &mut target_policy,
    );

    assert_eq!(
        reason,
        Some("unobserved_runtime_status_answer_candidate_requires_evidence")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert!(contract.requires_content_evidence);
    assert_eq!(turn_type, Some(TurnType::StatusQuery));
    assert_eq!(target_policy, Some(TargetTaskPolicy::Standalone));
    assert!(answer_candidate.is_empty());
    assert_eq!(
        state_patch
            .as_ref()
            .and_then(|value| value.pointer("/runtime_status_query/kind"))
            .and_then(Value::as_str),
        Some("current_user")
    );
}
