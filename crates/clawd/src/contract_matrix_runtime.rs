#[cfg(test)]
use super::MatchedContract;
use super::{
    ActionPolicyDecision, ActionRef, ContractMatrix, EvidenceExpression, FinalAnswerShape,
    FinalAnswerShapeClass, IntentOutputContract, ObservationExtractor, OutputResponseShape,
    OutputSemanticKind, RouteResult, BUNDLED_CONTRACT_MATRIX,
};
use claw_core::skill_registry::SkillsRegistry;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::OnceLock;

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

static BUNDLED_SKILLS_REGISTRY: OnceLock<Result<SkillsRegistry, String>> = OnceLock::new();

fn bundled_skills_registry() -> Option<&'static SkillsRegistry> {
    BUNDLED_SKILLS_REGISTRY
        .get_or_init(|| {
            SkillsRegistry::load_from_str(include_str!("../../../configs/skills_registry.toml"))
        })
        .as_ref()
        .ok()
}

pub(crate) fn compact_prompt_line_for_route(route: &RouteResult) -> Option<String> {
    if let Some(line) = compact_prompt_line_for_route_capability_ref(route) {
        return Some(line);
    }
    let output_contract = route.effective_output_contract();
    compact_prompt_line_for_output_contract(&output_contract)
}

fn compact_prompt_line_for_route_capability_ref(route: &RouteResult) -> Option<String> {
    let capability_refs = crate::machine_capability_ref::route_capability_ref_tokens(route);
    if capability_refs.is_empty() {
        return None;
    }
    let capability_refs = capability_refs.join(",");
    let required_evidence = crate::evidence_policy::required_evidence_fields_for_route(route);
    let required_evidence = if required_evidence.is_empty() {
        "none".to_string()
    } else {
        required_evidence.join(",")
    };
    let available_action_refs = route_capability_ref_action_refs(route, false)
        .into_iter()
        .map(|action| action.as_key())
        .collect::<Vec<_>>();
    let available_action_refs = if available_action_refs.is_empty() {
        "none".to_string()
    } else {
        available_action_refs.join(",")
    };
    let preferred_action_refs = route_capability_ref_action_refs(route, true)
        .into_iter()
        .map(|action| action.as_key())
        .collect::<Vec<_>>();
    let preferred_action_refs = if preferred_action_refs.is_empty() {
        available_action_refs.clone()
    } else {
        preferred_action_refs.join(",")
    };
    let final_answer_shape = final_answer_shape_for_route_capability_ref(route)
        .unwrap_or(FinalAnswerShape::Free)
        .as_str();

    Some(format!(
        "- capability_policy source=registry match=capability_ref capability_refs={} evidence_profile=capability_ref required_evidence={} final_answer_shape={} available_action_refs={} preferred_action_refs={}",
        capability_refs,
        required_evidence,
        final_answer_shape,
        available_action_refs,
        preferred_action_refs,
    ))
}

pub(crate) fn compact_prompt_line_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<String> {
    let matrix = bundled_contract_matrix()?;
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
    required_evidence_for_output_contract_with_route_reason(output_contract, None)
}

fn required_evidence_for_output_contract_with_route_reason(
    output_contract: &IntentOutputContract,
    route_reason: Option<&str>,
) -> Option<Vec<String>> {
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
    if output_contract_requires_quantity_path_metadata(output_contract, route_reason)
        && matches!(
            output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::CurrentWorkspace
        )
    {
        fields.insert("exists".to_string());
        fields.insert("kind".to_string());
    }
    Some(fields.into_iter().collect())
}

fn output_contract_requires_quantity_path_metadata(
    output_contract: &IntentOutputContract,
    route_reason: Option<&str>,
) -> bool {
    output_contract.semantic_kind_is(OutputSemanticKind::QuantityComparison)
        || route_reason.is_some_and(|route_reason| {
            let markers = crate::RouteReasonMarkers::new(route_reason);
            markers.has_machine_marker(OutputSemanticKind::QuantityComparison.as_str())
                || markers.has_machine_marker("quantity_compare")
        })
}

pub(crate) fn final_answer_shape_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<FinalAnswerShape> {
    if let Some(shape) = final_answer_shape_override_for_output_contract(output_contract) {
        return Some(shape);
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    matched.final_answer_shape_kind()
}

pub(crate) fn final_answer_shape_for_route(route: &RouteResult) -> Option<FinalAnswerShape> {
    if let Some(shape) = final_answer_shape_for_route_capability_ref(route) {
        return Some(shape);
    }
    let output_contract = route.effective_output_contract();
    final_answer_shape_for_output_contract(&output_contract)
}

fn final_answer_shape_for_route_capability_ref(route: &RouteResult) -> Option<FinalAnswerShape> {
    registry_final_answer_shape_for_route_capability_ref(route, true)
        .or_else(|| registry_final_answer_shape_for_route_capability_ref(route, false))
}

fn registry_final_answer_shape_for_route_capability_ref(
    route: &RouteResult,
    preferred_only: bool,
) -> Option<FinalAnswerShape> {
    let route_refs = crate::machine_capability_ref::route_capability_ref_tokens(route)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if route_refs.is_empty() {
        return None;
    }
    let registry = bundled_skills_registry()?;
    for skill in registry.all_names() {
        for mapping in registry.planner_capabilities(&skill) {
            if !route_refs.contains(&mapping.name) || (preferred_only && !mapping.preferred) {
                continue;
            }
            let Some(shape) = mapping.final_answer_shape.as_deref() else {
                continue;
            };
            if let Some(shape) = FinalAnswerShape::parse(shape) {
                return Some(shape);
            }
        }
    }
    None
}

fn final_answer_shape_override_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<FinalAnswerShape> {
    if output_contract.semantic_kind_is(OutputSemanticKind::HiddenEntriesCheck)
        && output_contract.response_shape == OutputResponseShape::Scalar
    {
        return Some(FinalAnswerShape::Scalar);
    }
    if output_contract.semantic_kind_is(OutputSemanticKind::StructuredKeys)
        && output_contract.response_shape != OutputResponseShape::Strict
    {
        return Some(FinalAnswerShape::ValidationVerdict);
    }
    None
}

pub(crate) fn trace_snapshot_for_route(route: &RouteResult) -> Option<Value> {
    let output_contract = route.effective_output_contract();
    trace_snapshot_for_output_contract_with_route_reason(
        &output_contract,
        Some(route.route_reason.as_str()),
    )
}

pub(crate) fn runtime_contract_snapshot_for_route(route: &RouteResult) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let contract_snapshot = trace_snapshot_for_route(route)?;
    let compact_line = compact_prompt_line_for_route(route);
    Some(runtime_contract_snapshot_value(
        matrix,
        contract_snapshot,
        compact_line,
    ))
}

#[cfg(test)]
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

#[cfg(test)]
pub(crate) fn trace_snapshot_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Option<Value> {
    trace_snapshot_for_output_contract_with_route_reason(output_contract, None)
}

fn trace_snapshot_for_output_contract_with_route_reason(
    output_contract: &IntentOutputContract,
    route_reason: Option<&str>,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let final_answer_shape_kind = final_answer_shape_override_for_output_contract(output_contract)
        .or_else(|| matched.final_answer_shape_kind());
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
            .self_extension
            .structured_field_selector
            .as_deref(),
        "contract_match": matched.match_name(),
        "policy_mode": matched.policy_mode(),
        "evidence_scope": matched.evidence_scope(),
        "freshness": matched.freshness(),
        "artifact_kind": matched.artifact_kind(),
        "channel_visibility": matched.channel_visibility(),
        "evidence_profile": matched.evidence_profile(),
        "required_evidence": required_evidence_for_output_contract_with_route_reason(
            output_contract,
            route_reason,
        )
            .unwrap_or_else(|| matched.required_evidence()),
        "evidence_expression": matched
            .evidence_expression()
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

#[cfg(test)]
pub(crate) fn action_trace_for_output_contract(
    output_contract: &IntentOutputContract,
    action_ref: &str,
) -> Option<Value> {
    action_trace_for_output_contract_with_route_reason(output_contract, None, action_ref)
}

pub(crate) fn action_trace_for_route(route: &RouteResult, action_ref: &str) -> Option<Value> {
    let action = ActionRef::parse(action_ref)?;
    if route_capability_ref_allows_action_ref(route, &action) {
        return Some(capability_ref_action_trace(route, &action));
    }
    let output_contract = route.effective_output_contract();
    action_trace_for_output_contract_with_route_reason(
        &output_contract,
        Some(route.route_reason.as_str()),
        action_ref,
    )
}

fn action_trace_for_output_contract_with_route_reason(
    output_contract: &IntentOutputContract,
    route_reason: Option<&str>,
    action_ref: &str,
) -> Option<Value> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let action = ActionRef::parse(action_ref)?;
    let action_key = action.as_key();
    let observation_extractor = matched.observation_extractor_for_source(&action_key);
    let final_answer_shape_kind = final_answer_shape_override_for_output_contract(output_contract)
        .or_else(|| matched.final_answer_shape_kind());
    Some(json!({
        "schema_version": 1,
        "action_ref": action_key,
        "contract_match": matched.match_name(),
        "decision": matched.action_policy(&action).as_str(),
        "policy_mode": matched.policy_mode(),
        "evidence_profile": matched.evidence_profile(),
        "observation_extractor": observation_extractor.as_ref().map(ObservationExtractor::to_trace_json),
        "required_evidence": required_evidence_for_output_contract_with_route_reason(
            output_contract,
            route_reason,
        )
            .unwrap_or_else(|| matched.required_evidence()),
        "evidence_expression": matched
            .evidence_expression()
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

fn capability_ref_action_trace(route: &RouteResult, action: &ActionRef) -> Value {
    let action_key = action.as_key();
    let final_answer_shape_kind =
        final_answer_shape_for_route_capability_ref(route).unwrap_or(FinalAnswerShape::Free);
    let required_evidence = crate::evidence_policy::required_evidence_fields_for_route(route);
    let evidence_expression = EvidenceExpression::default().to_trace_json(&required_evidence);
    let observation_extractor = ObservationExtractor::from_source(&action_key);
    json!({
        "schema_version": 1,
        "action_ref": action_key,
        "contract_match": "capability_ref",
        "decision": ActionPolicyDecision::Allowed.as_str(),
        "policy_mode": "observe",
        "evidence_profile": "capability_ref",
        "observation_extractor": observation_extractor.as_ref().map(ObservationExtractor::to_trace_json),
        "required_evidence": required_evidence,
        "evidence_expression": evidence_expression,
        "final_answer_shape": final_answer_shape_kind.as_str(),
        "final_answer_shape_class": final_answer_shape_kind.class().as_str(),
        "coarse_response_shape": final_answer_shape_kind.coarse_response_shape().as_str(),
        "allows_model_language": final_answer_shape_kind.allows_model_language(),
        "preferred_actions": [action_key.clone()],
        "allowed_actions": [action_key],
        "forbidden_actions": Vec::<String>::new(),
    })
}

pub(crate) fn contract_trace_action_key_for_route(
    route: &RouteResult,
    action_ref: &str,
) -> Option<String> {
    let output_contract = route.effective_output_contract();
    contract_trace_action_key_for_contract(&output_contract, action_ref)
}

fn contract_trace_action_key_for_contract(
    output_contract: &IntentOutputContract,
    action_ref: &str,
) -> Option<String> {
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let action = ActionRef::parse(action_ref)?;
    if matched.action_policy(&action) != ActionPolicyDecision::Allowed {
        return Some(action.as_key());
    }
    for raw in matched.allowed_actions() {
        let Some(policy_ref) = ActionRef::parse(raw) else {
            continue;
        };
        if action_matches_any(&action, std::slice::from_ref(raw)) {
            return Some(policy_ref.as_key());
        }
    }
    Some(action.as_key())
}

#[cfg(test)]
fn preferred_action_refs_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Vec<ActionRef> {
    bundled_contract_matrix()
        .and_then(|matrix| matrix.match_output_contract(output_contract))
        .map(|matched| {
            matched
                .preferred_actions()
                .iter()
                .filter_map(|action| ActionRef::parse(action))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn preferred_action_refs_for_route(route: &RouteResult) -> Vec<ActionRef> {
    let preferred_capability_refs = route_capability_ref_action_refs(route, true);
    if !preferred_capability_refs.is_empty() {
        return preferred_capability_refs;
    }
    let capability_refs = route_capability_ref_action_refs(route, false);
    if !capability_refs.is_empty() || route_has_capability_refs(route) {
        return capability_refs;
    }
    let output_contract = route.effective_output_contract();
    preferred_action_refs_for_output_contract(&output_contract)
}

#[cfg(test)]
pub(crate) fn allowed_action_refs_for_output_contract(
    output_contract: &IntentOutputContract,
) -> Vec<ActionRef> {
    bundled_contract_matrix()
        .and_then(|matrix| matrix.match_output_contract(output_contract))
        .map(|matched| {
            matched
                .allowed_actions()
                .iter()
                .filter_map(|action| ActionRef::parse(action))
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
pub(crate) fn allowed_action_refs_for_route(route: &RouteResult) -> Vec<ActionRef> {
    let capability_refs = route_capability_ref_action_refs(route, false);
    if !capability_refs.is_empty() || route_has_capability_refs(route) {
        return capability_refs;
    }
    let output_contract = route.effective_output_contract();
    allowed_action_refs_for_output_contract(&output_contract)
}

fn route_has_capability_refs(route: &RouteResult) -> bool {
    !crate::machine_capability_ref::route_capability_ref_tokens(route).is_empty()
}

fn route_capability_ref_action_refs(route: &RouteResult, preferred_only: bool) -> Vec<ActionRef> {
    let route_refs = crate::machine_capability_ref::route_capability_ref_tokens(route)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if route_refs.is_empty() {
        return Vec::new();
    }
    let Some(registry) = bundled_skills_registry() else {
        return Vec::new();
    };
    let mut action_refs = BTreeSet::new();
    for skill in registry.all_names() {
        for mapping in registry.planner_capabilities(&skill) {
            if !route_refs.contains(&mapping.name) || (preferred_only && !mapping.preferred) {
                continue;
            }
            let Some(action) = mapping.action.as_deref() else {
                continue;
            };
            if let Some(action_ref) = ActionRef::parse(&format!("{skill}.{action}")) {
                action_refs.insert(action_ref);
            }
        }
    }
    if action_refs.is_empty() && !preferred_only {
        for route_ref in &route_refs {
            if let Some(action_ref) = fallback_action_ref_for_capability_ref(route_ref) {
                action_refs.insert(action_ref);
            }
        }
    }
    action_refs.into_iter().collect()
}

fn fallback_action_ref_for_capability_ref(route_ref: &str) -> Option<ActionRef> {
    let (namespace, action) = route_ref.split_once('.')?;
    let skill = fallback_skill_for_capability_namespace(namespace)?;
    let action = fallback_action_for_capability_action(skill, action);
    ActionRef::parse(&format!("{skill}.{action}"))
}

fn fallback_skill_for_capability_namespace(namespace: &str) -> Option<&'static str> {
    match normalize_action_token(namespace).replace('-', "_").as_str() {
        "archive" => Some("archive_basic"),
        "config" | "config_basic" => Some("config_basic"),
        "config_edit" => Some("config_edit"),
        "database" | "db" | "sqlite" => Some("db_basic"),
        "docker" => Some("docker_basic"),
        "document" | "doc" => Some("doc_parse"),
        "file" | "filesystem" | "fs" | "fs_basic" => Some("fs_basic"),
        "kb" => Some("kb"),
        "package" | "package_manager" => Some("package_manager"),
        "process" => Some("process_basic"),
        "service" | "service_control" => Some("service_control"),
        "system" | "system_basic" => Some("system_basic"),
        "task" | "task_control" => Some("task_control"),
        "weather" => Some("weather"),
        "web" | "web_search" => Some("web_search_extract"),
        _ => None,
    }
}

fn fallback_action_for_capability_action(skill: &str, action: &str) -> String {
    let action = normalize_action_token(action).replace('-', "_");
    match (skill, action.as_str()) {
        ("db_basic", action) if machine_action_has_any_segment(action, &["query"]) => {
            "sqlite_query".to_string()
        }
        ("doc_parse", action) if machine_action_has_any_segment(action, &["parse"]) => {
            "parse_doc".to_string()
        }
        ("fs_basic", action) if machine_action_has_any_segment(action, &["list"]) => {
            "list_dir".to_string()
        }
        ("fs_basic", action) if machine_action_has_any_segment(action, &["read"]) => {
            "read_text_range".to_string()
        }
        ("package_manager", action) if machine_action_has_any_segment(action, &["detect"]) => {
            "detect_manager".to_string()
        }
        _ => action,
    }
}

#[cfg(test)]
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

fn route_capability_policy_action_ref(
    normalized_skill: &str,
    args: &Value,
) -> Option<PolicyActionRef> {
    let action = ActionRef::from_skill_args(normalized_skill, args)?;
    runtime_equivalent_virtual_action_ref(normalized_skill, args)
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

#[cfg(test)]
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
    let final_answer_shape_kind = matched.final_answer_shape_kind()?;
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
        required_evidence: matched.required_evidence(),
        preferred_actions: normalized_tokens(matched.preferred_actions()),
        final_answer_shape_kind,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        evidence_expression: matched.evidence_expression(),
        policy_mode: matched.policy_mode(),
        evidence_scope: matched.evidence_scope(),
        freshness: matched.freshness(),
        artifact_kind: matched.artifact_kind(),
        channel_visibility: matched.channel_visibility(),
        evidence_profile: matched.evidence_profile(),
    })
}

pub(crate) fn capability_ref_action_policy_for_route(
    route: Option<&RouteResult>,
    normalized_skill: &str,
    args: &Value,
) -> Option<ContractActionPolicy> {
    let route = route?;
    if route_has_capability_refs(route) {
        return route_capability_ref_action_policy(route, normalized_skill, args);
    }
    None
}

#[cfg(test)]
pub(crate) fn action_policy_for_route(
    route: Option<&RouteResult>,
    normalized_skill: &str,
    args: &Value,
) -> Option<ContractActionPolicy> {
    capability_ref_action_policy_for_route(route, normalized_skill, args)
}

fn route_capability_ref_action_policy(
    route: &RouteResult,
    normalized_skill: &str,
    args: &Value,
) -> Option<ContractActionPolicy> {
    let action = route_capability_policy_action_ref(normalized_skill, args)?;
    let action_allowed = route_capability_ref_allows_action(route, normalized_skill, args);
    if !action_allowed && !route_capability_ref_repairable_evidence_action(route, &action.effective)
    {
        return None;
    }
    let decision = if action_allowed {
        ActionPolicyDecision::Allowed
    } else {
        ActionPolicyDecision::RejectedNotAllowed
    };
    let mut preferred_actions = route_capability_ref_action_refs(route, false)
        .into_iter()
        .map(|action_ref| action_ref.as_key())
        .collect::<Vec<_>>();
    let action_key = action.effective.as_key();
    if !preferred_actions
        .iter()
        .any(|preferred| preferred == &action_key)
    {
        preferred_actions.push(action_key.clone());
    }
    let final_answer_shape_kind =
        final_answer_shape_for_route_capability_ref(route).unwrap_or(FinalAnswerShape::Free);
    Some(ContractActionPolicy {
        decision,
        action_key: action.effective.as_key(),
        original_action_ref: action.original.as_key(),
        replacement_action_ref: action.replacement_action_ref(),
        contract_repair_source: "capability_ref_route_policy".to_string(),
        preferred_replacement_reason_code: action.replacement_reason_code.map(str::to_string),
        contract_match: "capability_ref".to_string(),
        required_evidence: crate::evidence_policy::required_evidence_fields_for_route(route),
        preferred_actions,
        final_answer_shape_kind,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        evidence_expression: EvidenceExpression::default(),
        policy_mode: "observe".to_string(),
        evidence_scope: "conversation".to_string(),
        freshness: "conversation".to_string(),
        artifact_kind: "text".to_string(),
        channel_visibility: "user_visible".to_string(),
        evidence_profile: "capability_ref".to_string(),
    })
}

fn route_capability_ref_repairable_evidence_action(
    route: &RouteResult,
    action: &ActionRef,
) -> bool {
    let Some(action_name) = action.action.as_deref() else {
        return false;
    };
    if action.skill != "fs_basic" || action_name != "read_text_range" {
        return false;
    }
    route_capability_ref_action_refs(route, false)
        .iter()
        .any(|preferred| preferred.skill == "doc_parse")
}

fn route_capability_ref_allows_action(
    route: &RouteResult,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    let Some(action) = ActionRef::from_skill_args(normalized_skill, args) else {
        return false;
    };
    if route_capability_ref_allows_action_ref(route, &action) {
        return true;
    }
    route_capability_policy_action_ref(normalized_skill, args)
        .filter(|policy_action| policy_action.replacement_action_ref().is_some())
        .is_some_and(|policy_action| {
            route_registry_capability_ref_allows_action_ref(route, &policy_action.effective)
        })
}

fn route_capability_ref_allows_action_ref(route: &RouteResult, action: &ActionRef) -> bool {
    route_registry_capability_ref_allows_action_ref(route, action)
        || route_generic_capability_ref_allows_action_ref(route, action)
}

fn route_registry_capability_ref_allows_action_ref(
    route: &RouteResult,
    action: &ActionRef,
) -> bool {
    let Some(action_name) = action.action.as_deref() else {
        return false;
    };
    let Some(registry) = bundled_skills_registry() else {
        return false;
    };
    let Some(manifest) = registry.manifest(&action.skill) else {
        return false;
    };
    let route_refs = crate::machine_capability_ref::route_capability_ref_tokens(route);
    if route_refs.is_empty() {
        return false;
    }
    manifest.planner_capabilities.iter().any(|mapping| {
        mapping.action.as_deref() == Some(action_name)
            && route_refs
                .iter()
                .any(|capability| capability == &mapping.name)
    })
}

fn route_generic_capability_ref_allows_action_ref(route: &RouteResult, action: &ActionRef) -> bool {
    crate::machine_capability_ref::route_capability_ref_tokens(route)
        .iter()
        .any(|route_ref| generic_capability_ref_allows_action_ref(route_ref, action))
}

fn generic_capability_ref_allows_action_ref(route_ref: &str, action: &ActionRef) -> bool {
    let Some((namespace, capability_action)) = route_ref.split_once('.') else {
        return false;
    };
    capability_namespace_matches_action_skill(namespace, &action.skill)
        && action.action.as_deref().is_some_and(|action_name| {
            capability_action_matches_skill_action(capability_action, action_name)
        })
}

fn capability_namespace_matches_action_skill(namespace: &str, skill: &str) -> bool {
    let namespace = normalize_action_token(namespace).replace('-', "_");
    let skill = normalize_action_token(skill).replace('-', "_");
    match skill.as_str() {
        "archive_basic" => matches!(namespace.as_str(), "archive"),
        "config_basic" | "config_edit" => {
            matches!(
                namespace.as_str(),
                "config" | "config_basic" | "config_edit"
            )
        }
        "db_basic" => matches!(namespace.as_str(), "database" | "db" | "sqlite"),
        "docker_basic" => matches!(namespace.as_str(), "docker"),
        "doc_parse" => matches!(namespace.as_str(), "document" | "doc" | "doc_parse"),
        "fs_basic" => matches!(
            namespace.as_str(),
            "file" | "filesystem" | "fs" | "fs_basic"
        ),
        "kb" => matches!(namespace.as_str(), "kb"),
        "package_manager" => matches!(namespace.as_str(), "package" | "package_manager"),
        "process_basic" => matches!(namespace.as_str(), "process"),
        "service_control" => matches!(namespace.as_str(), "service" | "service_control"),
        "system_basic" => matches!(namespace.as_str(), "system" | "system_basic"),
        "task_control" => matches!(namespace.as_str(), "task" | "task_control"),
        "weather" => matches!(namespace.as_str(), "weather"),
        "web_search_extract" => matches!(namespace.as_str(), "web" | "web_search"),
        _ => namespace == skill,
    }
}

fn capability_action_matches_skill_action(capability_action: &str, skill_action: &str) -> bool {
    let capability_action = normalize_action_token(capability_action).replace('-', "_");
    let skill_action = normalize_action_token(skill_action).replace('-', "_");
    if capability_action == skill_action {
        return true;
    }
    let capability_segments = action_segments(&capability_action);
    let skill_segments = action_segments(&skill_action);
    capability_segments.iter().any(|capability_segment| {
        skill_segments
            .iter()
            .any(|skill_segment| capability_segment == skill_segment)
    })
}

fn action_segments(action: &str) -> Vec<&str> {
    action
        .split(|ch| matches!(ch, '.' | '_' | '-'))
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect()
}

fn machine_action_has_any_segment(action: &str, needles: &[&str]) -> bool {
    let segments = action_segments(action);
    segments
        .iter()
        .any(|segment| needles.iter().any(|needle| segment == needle))
}

pub(crate) fn action_matches_policy_tokens(action_key: &str, policies: &[String]) -> bool {
    let Some(action) = ActionRef::parse(action_key) else {
        return false;
    };
    action_matches_any(&action, policies)
}
