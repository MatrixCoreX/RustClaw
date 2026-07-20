use super::MatchedContract;
use super::{
    ActionPolicyDecision, ActionRef, ContractMatrix, EvidenceExpression, FinalAnswerShape,
    FinalAnswerShapeClass, IntentOutputContract, ObservationExtractor, BUNDLED_CONTRACT_MATRIX,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

#[path = "contract_matrix_runtime_validation.rs"]
mod validation;
#[cfg(test)]
pub(crate) use validation::available_action_refs_from_registry;
pub(crate) use validation::fnv1a_hex;
pub(super) use validation::{
    action_matches_any, contains_token, default_extractor_kind_for_observation_source,
    evidence_expression_tokens, normalize_action_token, normalized_contract_field,
    normalized_extractor_kind, normalized_tokens, observation_extractors_for_sources,
    observation_extractors_stable_key, validate_artifact_shape_contract,
    validate_contract_runtime_fields, validate_observation_extractors,
};
use validation::{
    bundled_prompt_layer_manifest_hash, bundled_registry_hash, observation_extractors_trace_json,
};
#[cfg(test)]
pub(super) use validation::{collect_action_tokens, collect_external_observation_admission_errors};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractActionPolicy {
    pub(crate) decision: ActionPolicyDecision,
    pub(crate) action_key: String,
    pub(crate) original_action_ref: String,
    pub(crate) replacement_action_ref: Option<String>,
    pub(crate) contract_repair_source: String,
    pub(crate) preferred_replacement_reason_code: Option<String>,
    pub(crate) contract_match: String,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) preferred_actions: Vec<String>,
    pub(crate) final_answer_shape_kind: FinalAnswerShape,
    pub(crate) final_answer_shape: String,
    pub(crate) evidence_expression: EvidenceExpression,
    pub(crate) policy_mode: String,
    pub(crate) evidence_scope: String,
    pub(crate) freshness: String,
    pub(crate) artifact_kind: String,
    pub(crate) channel_visibility: String,
    pub(crate) evidence_profile: String,
}

impl ContractActionPolicy {
    #[cfg(test)]
    pub(crate) fn is_allowed(&self) -> bool {
        self.decision == ActionPolicyDecision::Allowed
    }

    #[cfg(test)]
    pub(crate) fn action_matches_preferred(&self) -> bool {
        action_matches_policy_tokens(&self.action_key, &self.preferred_actions)
    }
}

pub(crate) fn parse_contract_matrix_source(source: &str) -> Result<ContractMatrix, String> {
    let matrix: ContractMatrix =
        toml::from_str(source).map_err(|err| format!("contract matrix parse failed: {err}"))?;
    let shape_errors = matrix.validate_shape();
    if !shape_errors.is_empty() {
        return Err(format!(
            "contract matrix shape invalid: {}",
            shape_errors.join("; ")
        ));
    }
    Ok(matrix)
}

pub(crate) fn bundled_contract_matrix_result() -> Result<&'static ContractMatrix, &'static str> {
    match BUNDLED_CONTRACT_MATRIX.get_or_init(|| {
        parse_contract_matrix_source(include_str!("../../../configs/task_contract_matrix.toml"))
    }) {
        Ok(matrix) => Ok(matrix),
        Err(err) => Err(err.as_str()),
    }
}

pub(crate) fn bundled_contract_matrix() -> Option<&'static ContractMatrix> {
    bundled_contract_matrix_result().ok()
}

pub(crate) fn compact_prompt_line_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<String> {
    let matrix = bundled_contract_matrix()?;
    if let Some(exact_list_evidence) = exact_list_evidence_fields(output_contract) {
        return Some(format!(
            "- evidence_policy source=validated_output_selector version={} hash={} match=exact_list_selector planner_authority=agent_loop_registry evidence_profile=selected_list required_evidence={} final_answer_shape=exact_list",
            matrix.matrix_version,
            matrix.matrix_version_hash(),
            exact_list_evidence.join(","),
        ));
    }
    let matched = matrix.match_output_contract(output_contract)?;
    let required_evidence = matched.required_evidence();
    let required_evidence = if required_evidence.is_empty() {
        "none".to_string()
    } else {
        required_evidence.join(",")
    };
    Some(format!(
        "- evidence_policy source=bundled_evidence_policy version={} hash={} match={} planner_authority=agent_loop_registry evidence_profile={} required_evidence={} final_answer_shape={}",
        matrix.matrix_version,
        matrix.matrix_version_hash(),
        matched.match_name(),
        matched.evidence_profile(),
        required_evidence,
        matched
            .final_answer_shape_kind()
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
    ))
}

pub(crate) fn required_evidence_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<Vec<String>> {
    if let Some(fields) = exact_list_evidence_fields(output_contract) {
        return Some(fields);
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let mut fields = matched
        .required_evidence()
        .into_iter()
        .collect::<BTreeSet<_>>();
    if output_contract.delivery_required
        || matches!(
            output_contract.delivery_intent,
            crate::OutputDeliveryIntent::FileSingle
                | crate::OutputDeliveryIntent::DirectoryLookup
                | crate::OutputDeliveryIntent::DirectoryBatchFiles
        )
    {
        fields.insert("path".to_string());
    }
    Some(fields.into_iter().collect())
}

fn evidence_expression_for_output_contract(
    output_contract: &IntentOutputContract,
    matched: &MatchedContract<'_>,
) -> EvidenceExpression {
    if let Some(fields) = exact_list_evidence_fields(output_contract) {
        return EvidenceExpression {
            all_of: fields,
            ..Default::default()
        };
    }
    matched.evidence_expression()
}

fn exact_list_evidence_fields(output_contract: &IntentOutputContract) -> Option<Vec<String>> {
    if output_contract.requests_exact_name_list() {
        return Some(vec!["candidates".to_string()]);
    }
    output_contract
        .requests_exact_path_list()
        .then_some(())
        .and_then(|()| {
            output_contract
                .selection
                .structured_field_selector
                .as_deref()
                .and_then(crate::machine_kv_projection::exact_machine_field_selector)
        })
}

pub(crate) fn final_answer_shape_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<FinalAnswerShape> {
    if output_contract.requests_exact_list() {
        return Some(FinalAnswerShape::ExactList);
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    matched.final_answer_shape_kind()
}

pub(crate) fn runtime_contract_snapshot_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let contract_snapshot = trace_snapshot_for_output_contract(output_contract)?;
    let compact_line = compact_prompt_line_for_output_contract(output_contract);
    Some(runtime_contract_snapshot_value(
        matrix,
        contract_snapshot,
        compact_line,
    ))
}

fn runtime_contract_snapshot_value(
    matrix: &ContractMatrix,
    contract_snapshot: Value,
    compact_line: Option<String>,
) -> Value {
    json!({
        "schema_version": 1,
        "matrix": {
            "version": matrix.matrix_version,
            "hash": matrix.matrix_version_hash(),
            "source": "bundled:configs/task_contract_matrix.toml",
        },
        "registry": {
            "hash": bundled_registry_hash(),
            "source": "bundled:configs/skills_registry.toml",
        },
        "prompt_layer": {
            "hash": bundled_prompt_layer_manifest_hash(),
            "source": "bundled:prompts/layers/manifest.toml",
        },
        "compact_contract_block": compact_line.as_ref().map(|line| {
            json!({
                "hash": fnv1a_hex(line),
                "bytes": line.len(),
                "present": true,
            })
        }),
        "contract": contract_snapshot,
    })
}

pub(crate) fn trace_snapshot_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let final_answer_shape_kind = final_answer_shape_for_output_contract(output_contract);
    let observation_extractors = matched.observation_extractors();
    Some(json!({
        "evidence_policy_version": matrix.matrix_version,
        "evidence_policy_hash": matrix.matrix_version_hash(),
        "schema_version": matrix.schema_version,
        "trace_policy": matrix.trace_policy.to_trace_json(),
        "contract_marker": output_contract.semantic_kind.as_str(),
        "response_shape": output_contract.response_shape.as_str(),
        "locator_kind": output_contract.locator_kind.as_str(),
        "delivery_intent": output_contract.delivery_intent.as_str(),
        "requires_content_evidence": output_contract.requires_content_evidence,
        "delivery_required": output_contract.delivery_required,
        "structured_field_selector": output_contract
            .selection
            .structured_field_selector
            .as_deref(),
        "contract_match": matched.match_name(),
        "policy_mode": matched.policy_mode(),
        "evidence_scope": matched.evidence_scope(),
        "freshness": matched.freshness(),
        "artifact_kind": matched.artifact_kind(),
        "channel_visibility": matched.channel_visibility(),
        "evidence_profile": matched.evidence_profile(),
        "required_evidence": required_evidence_for_output_contract(output_contract)
            .unwrap_or_else(|| matched.required_evidence()),
        "evidence_expression": evidence_expression_for_output_contract(output_contract, &matched)
            .to_trace_json(&matched.required_evidence()),
        "observation_sources": matched.observation_sources(),
        "observation_extractors": observation_extractors_trace_json(&observation_extractors),
        "final_answer_shape": final_answer_shape_kind
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
        "final_answer_shape_class": final_answer_shape_kind.map(|shape| shape.class().as_str()),
        "coarse_response_shape": final_answer_shape_kind
            .map(|shape| shape.coarse_response_shape().as_str()),
        "allows_model_language": final_answer_shape_kind.map(FinalAnswerShape::allows_model_language),
        "preferred_actions": normalized_tokens(matched.preferred_actions()),
        "allowed_actions": normalized_tokens(matched.allowed_actions()),
        "forbidden_actions": normalized_tokens(matched.forbidden_actions()),
    }))
}

pub(crate) fn action_trace_for_output_contract(
    output_contract: &IntentOutputContract,
    action_ref: &str,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let action = ActionRef::parse(action_ref)?;
    let action_key = action.as_key();
    let observation_extractor = matched.observation_extractor_for_source(&action_key);
    let final_answer_shape_kind = final_answer_shape_for_output_contract(output_contract);
    Some(json!({
        "schema_version": 1,
        "action_ref": action_key,
        "contract_match": matched.match_name(),
        "decision": matched.action_policy(&action).as_str(),
        "policy_mode": matched.policy_mode(),
        "evidence_profile": matched.evidence_profile(),
        "observation_extractor": observation_extractor.as_ref().map(ObservationExtractor::to_trace_json),
        "required_evidence": required_evidence_for_output_contract(output_contract)
            .unwrap_or_else(|| matched.required_evidence()),
        "evidence_expression": evidence_expression_for_output_contract(output_contract, &matched)
            .to_trace_json(&matched.required_evidence()),
        "final_answer_shape": final_answer_shape_kind
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
        "final_answer_shape_class": final_answer_shape_kind.map(|shape| shape.class().as_str()),
        "coarse_response_shape": final_answer_shape_kind
            .map(|shape| shape.coarse_response_shape().as_str()),
        "allows_model_language": final_answer_shape_kind.map(FinalAnswerShape::allows_model_language),
        "preferred_actions": normalized_tokens(matched.preferred_actions()),
        "allowed_actions": normalized_tokens(matched.allowed_actions()),
        "forbidden_actions": normalized_tokens(matched.forbidden_actions()),
    }))
}

fn policy_action_ref_for_match(
    matched: &MatchedContract<'_>,
    normalized_skill: &str,
    args: &Value,
) -> Option<PolicyActionRef> {
    let action = ActionRef::from_skill_args(normalized_skill, args)?;
    if matched.action_policy(&action) == ActionPolicyDecision::Allowed {
        return Some(PolicyActionRef::original(action));
    }
    runtime_equivalent_virtual_action_ref(normalized_skill, args)
        .filter(|canonical| matched.action_policy(canonical) == ActionPolicyDecision::Allowed)
        .map(|canonical| {
            PolicyActionRef::replacement(
                action.clone(),
                canonical,
                "runtime_equivalent_virtual_action",
                "runtime_virtual_action_allowed",
            )
        })
        .or_else(|| {
            crate::virtual_tools::canonicalize_legacy_tool_call(normalized_skill, args.clone())
                .and_then(|canonical| ActionRef::from_skill_args(&canonical.tool, &canonical.args))
                .filter(|canonical| {
                    matched.action_policy(canonical) == ActionPolicyDecision::Allowed
                })
                .map(|canonical| {
                    PolicyActionRef::replacement(
                        action.clone(),
                        canonical,
                        "legacy_tool_canonicalization",
                        "legacy_tool_canonical_action_allowed",
                    )
                })
        })
        .or_else(|| Some(PolicyActionRef::original(action)))
}

fn runtime_equivalent_virtual_action_ref(
    normalized_skill: &str,
    args: &Value,
) -> Option<ActionRef> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_action_token)?;
    match (
        normalize_action_token(normalized_skill).as_str(),
        action.as_str(),
    ) {
        ("config_edit", "guard_config") => ActionRef::parse("config_basic.guard_rustclaw_config"),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PolicyActionRef {
    original: ActionRef,
    effective: ActionRef,
    repair_source: &'static str,
    replacement_reason_code: Option<&'static str>,
}

impl PolicyActionRef {
    fn original(action: ActionRef) -> Self {
        Self {
            original: action.clone(),
            effective: action,
            repair_source: "none",
            replacement_reason_code: None,
        }
    }

    fn replacement(
        original: ActionRef,
        effective: ActionRef,
        repair_source: &'static str,
        replacement_reason_code: &'static str,
    ) -> Self {
        Self {
            original,
            effective,
            repair_source,
            replacement_reason_code: Some(replacement_reason_code),
        }
    }

    fn replacement_action_ref(&self) -> Option<String> {
        (self.original.as_key() != self.effective.as_key()).then(|| self.effective.as_key())
    }
}

pub(crate) fn action_policy_for_output_contract(
    output_contract: Option<&IntentOutputContract>,
    normalized_skill: &str,
    args: &Value,
) -> Option<ContractActionPolicy> {
    let output_contract = output_contract?;
    if output_contract.semantic_kind_is_unclassified()
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
    {
        return None;
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let policy_action = policy_action_ref_for_match(&matched, normalized_skill, args)?;
    let final_answer_shape_kind = final_answer_shape_for_output_contract(output_contract)?;
    let decision = matched.action_policy(&policy_action.effective);
    Some(ContractActionPolicy {
        decision,
        action_key: policy_action.effective.as_key(),
        original_action_ref: policy_action.original.as_key(),
        replacement_action_ref: policy_action.replacement_action_ref(),
        contract_repair_source: policy_action.repair_source.to_string(),
        preferred_replacement_reason_code: policy_action
            .replacement_reason_code
            .map(str::to_string),
        contract_match: matched.match_name().to_string(),
        required_evidence: required_evidence_for_output_contract(output_contract)
            .unwrap_or_else(|| matched.required_evidence()),
        preferred_actions: normalized_tokens(matched.preferred_actions()),
        final_answer_shape_kind,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        evidence_expression: evidence_expression_for_output_contract(output_contract, &matched),
        policy_mode: matched.policy_mode(),
        evidence_scope: matched.evidence_scope(),
        freshness: matched.freshness(),
        artifact_kind: matched.artifact_kind(),
        channel_visibility: matched.channel_visibility(),
        evidence_profile: matched.evidence_profile(),
    })
}

#[cfg(test)]
pub(crate) fn action_matches_policy_tokens(action_key: &str, policies: &[String]) -> bool {
    let Some(action) = ActionRef::parse(action_key) else {
        return false;
    };
    action_matches_any(&action, policies)
}
