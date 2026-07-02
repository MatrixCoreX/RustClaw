use super::{
    ActionPolicyDecision, ActionRef, ArgPolicyDecision, ContractMatrix, EvidenceExpression,
    FinalAnswerShape, FinalAnswerShapeClass, IntentOutputContract, MatchedContract,
    ObservationExtractor, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteResult,
    BUNDLED_CONTRACT_MATRIX,
};
#[cfg(test)]
use claw_core::skill_registry::SkillKind;
use claw_core::skill_registry::SkillsRegistry;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContractArgPolicy {
    pub(crate) decision: ArgPolicyDecision,
    pub(crate) action_key: String,
    pub(crate) contract_match: String,
    pub(crate) required_evidence: Vec<String>,
    pub(crate) missing_target_args: Vec<String>,
    pub(crate) deferred_target_args: Vec<String>,
    pub(crate) expected_target_args: Vec<String>,
    pub(crate) final_answer_shape: String,
    pub(crate) policy_mode: String,
    pub(crate) evidence_scope: String,
    pub(crate) freshness: String,
    pub(crate) artifact_kind: String,
    pub(crate) channel_visibility: String,
    pub(crate) evidence_profile: String,
}

impl ContractArgPolicy {
    pub(crate) fn is_allowed(&self) -> bool {
        self.decision == ArgPolicyDecision::Allowed
    }
}

impl ContractActionPolicy {
    pub(crate) fn is_allowed(&self) -> bool {
        self.decision == ActionPolicyDecision::Allowed
    }

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
    let output_contract = route.effective_output_contract();
    compact_prompt_line_for_output_contract(&output_contract)
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
    let allowed_actions = normalized_tokens(matched.allowed_actions());
    let allowed_actions = if allowed_actions.is_empty() {
        "none".to_string()
    } else {
        allowed_actions.join(",")
    };
    let forbidden_actions = normalized_tokens(matched.forbidden_actions());
    let forbidden_actions = if forbidden_actions.is_empty() {
        "none".to_string()
    } else {
        forbidden_actions.join(",")
    };

    Some(format!(
        "- contract_matrix version={} hash={} match={} evidence_profile={} required_evidence={} final_answer_shape={} allowed_actions={} forbidden_actions={}",
        matrix.matrix_version,
        matrix.matrix_version_hash(),
        matched.match_name(),
        matched.evidence_profile(),
        required_evidence,
        matched
            .final_answer_shape_kind()
            .map(FinalAnswerShape::as_str)
            .unwrap_or_else(|| matched.final_answer_shape()),
        allowed_actions,
        forbidden_actions,
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
        || route_reason_has_machine_marker(
            route_reason,
            OutputSemanticKind::QuantityComparison.as_str(),
        )
        || route_reason_has_machine_marker(route_reason, "quantity_compare")
}

fn route_reason_has_machine_marker(route_reason: Option<&str>, marker: &str) -> bool {
    route_reason
        .unwrap_or_default()
        .split(';')
        .map(str::trim)
        .any(|part| {
            part == marker
                || part
                    .rsplit_once(':')
                    .is_some_and(|(_, suffix)| suffix.trim() == marker)
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
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["archive"],
        &["list"],
    ) {
        return Some(FinalAnswerShape::ArchiveMemberList);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["archive"],
        &["read"],
    ) {
        return Some(FinalAnswerShape::ArchiveMemberExcerpt);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["archive"],
        &["pack"],
    ) {
        return Some(FinalAnswerShape::CreatedArchivePath);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["archive"],
        &["unpack"],
    ) {
        return Some(FinalAnswerShape::UnpackDestinationSummary);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["database", "db", "sqlite"],
        &["list_tables", "list"],
    ) {
        return Some(FinalAnswerShape::TableListing);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["database", "db", "sqlite"],
        &["schema_version"],
    ) {
        return Some(FinalAnswerShape::SchemaVersion);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["service", "service_control"],
        &["status"],
    ) || crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["system", "system_basic"],
        &["health_check"],
    ) {
        return Some(FinalAnswerShape::StatusWithSource);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["service", "service_control"],
        &["restart", "start", "stop"],
    ) {
        return Some(FinalAnswerShape::LifecycleResult);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["docker", "docker_basic"],
        &["list_containers", "ps"],
    ) {
        return Some(FinalAnswerShape::ContainerList);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["docker", "docker_basic"],
        &["images", "list_images"],
    ) {
        return Some(FinalAnswerShape::ImageList);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["docker", "docker_basic"],
        &["logs", "read_logs"],
    ) {
        return Some(FinalAnswerShape::LogExcerptOrSummary);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["docker", "docker_basic"],
        &[
            "restart",
            "restart_container",
            "start",
            "start_container",
            "stop",
            "stop_container",
        ],
    ) {
        return Some(FinalAnswerShape::LifecycleResult);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["filesystem", "fs", "fs_basic"],
        &["count_entries"],
    ) {
        return Some(FinalAnswerShape::Scalar);
    }
    if route.effective_output_contract().response_shape == OutputResponseShape::Scalar
        && crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["system", "system_basic"],
            &["runtime_status"],
        )
    {
        return Some(FinalAnswerShape::Scalar);
    }
    if route.effective_output_contract().response_shape == OutputResponseShape::Scalar
        && crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["config", "config_basic"],
            &["read_field"],
        )
    {
        return Some(FinalAnswerShape::Scalar);
    }
    if route.effective_output_contract().response_shape == OutputResponseShape::Scalar
        && crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["system_basic"],
            &["extract_field"],
        )
    {
        return Some(FinalAnswerShape::Scalar);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["filesystem", "fs", "fs_basic"],
        &["find_entries"],
    ) {
        return Some(FinalAnswerShape::PathList);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["config"],
        &["list_keys"],
    ) {
        return Some(FinalAnswerShape::KeyListOrKeySummary);
    }
    if crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["config", "config_basic", "config_edit", "config_guard"],
        &[
            "guard_after_change",
            "guard_config",
            "guard_rustclaw_config",
            "validate",
            "validate_after_change",
            "validate_config",
        ],
    ) {
        return Some(FinalAnswerShape::ValidationVerdict);
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
        "contract_matrix_version": matrix.matrix_version,
        "contract_matrix_hash": matrix.matrix_version_hash(),
        "schema_version": matrix.schema_version,
        "trace_policy": matrix.trace_policy.to_trace_json(),
        "semantic_kind": output_contract.semantic_kind.as_str(),
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
        final_answer_shape_for_route(route).unwrap_or(FinalAnswerShape::Free);
    let required_evidence = crate::task_contract::required_evidence_fields_for_route(route);
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

pub(crate) fn preferred_action_refs_for_route(route: &RouteResult) -> Vec<ActionRef> {
    let output_contract = route.effective_output_contract();
    preferred_action_refs_for_output_contract(&output_contract)
}

fn allowed_action_refs_for_output_contract(
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

pub(crate) fn allowed_action_refs_for_route(route: &RouteResult) -> Vec<ActionRef> {
    let output_contract = route.effective_output_contract();
    allowed_action_refs_for_output_contract(&output_contract)
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

pub(crate) fn action_policy_for_route(
    route: Option<&RouteResult>,
    normalized_skill: &str,
    args: &Value,
) -> Option<ContractActionPolicy> {
    let route = route?;
    let output_contract = route.effective_output_contract();
    let mut policy =
        match action_policy_for_output_contract(Some(&output_contract), normalized_skill, args) {
            Some(policy) => policy,
            None => return route_capability_ref_action_policy(route, normalized_skill, args),
        };
    let capability_ref_allows_action =
        route_capability_ref_allows_action(route, normalized_skill, args);
    if capability_ref_allows_action {
        if let Some(action) = route_capability_policy_action_ref(normalized_skill, args) {
            policy.action_key = action.effective.as_key();
            policy.original_action_ref = action.original.as_key();
            policy.replacement_action_ref = action.replacement_action_ref();
            policy.preferred_replacement_reason_code =
                action.replacement_reason_code.map(str::to_string);
        }
        policy.decision = ActionPolicyDecision::Allowed;
        policy.contract_match = "capability_ref".to_string();
        policy.contract_repair_source = "capability_ref_route_policy".to_string();
        policy.required_evidence = crate::task_contract::required_evidence_fields_for_route(route);
        policy.preferred_actions = vec![policy.action_key.clone()];
    }
    Some(policy)
}

fn route_capability_ref_action_policy(
    route: &RouteResult,
    normalized_skill: &str,
    args: &Value,
) -> Option<ContractActionPolicy> {
    if !route_capability_ref_allows_action(route, normalized_skill, args) {
        return None;
    }
    let action = route_capability_policy_action_ref(normalized_skill, args)?;
    let final_answer_shape_kind =
        final_answer_shape_for_route(route).unwrap_or(FinalAnswerShape::Free);
    Some(ContractActionPolicy {
        decision: ActionPolicyDecision::Allowed,
        action_key: action.effective.as_key(),
        original_action_ref: action.original.as_key(),
        replacement_action_ref: action.replacement_action_ref(),
        contract_repair_source: "capability_ref_route_policy".to_string(),
        preferred_replacement_reason_code: action.replacement_reason_code.map(str::to_string),
        contract_match: "capability_ref".to_string(),
        required_evidence: crate::task_contract::required_evidence_fields_for_route(route),
        preferred_actions: vec![action.effective.as_key()],
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

fn route_capability_ref_allows_action(
    route: &RouteResult,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    let Some(action) = ActionRef::from_skill_args(normalized_skill, args) else {
        return false;
    };
    route_capability_ref_allows_action_ref(route, &action)
}

fn route_capability_ref_allows_action_ref(route: &RouteResult, action: &ActionRef) -> bool {
    if route_registry_capability_ref_allows_action_ref(route, action) {
        return true;
    }
    let action_name = action.action.as_deref().unwrap_or_default();
    match (action.skill.as_str(), action_name) {
        ("config_basic", "validate")
        | ("config_edit", "validate_config")
        | ("system_basic", "validate_structured") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["config"],
                &["validate", "validate_config", "validate_after_change"],
            )
        }
        ("config_basic", "guard_rustclaw_config") | ("config_edit", "guard_config") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["config"],
                &[
                    "guard",
                    "guard_config",
                    "guard_after_change",
                    "guard_rustclaw_config",
                ],
            )
        }
        ("config_guard", "") => crate::machine_capability_ref::route_has_capability_action(
            route,
            &["config"],
            &["guard", "risk"],
        ),
        ("config_basic", "read_field") | ("system_basic", "extract_field") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["config", "config_basic", "system_basic"],
                &["read_field", "extract_field"],
            )
        }
        ("config_basic", "read_fields") | ("system_basic", "extract_fields") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["config", "config_basic", "system_basic"],
                &["read_fields", "extract_fields"],
            )
        }
        ("config_basic", "list_keys") | ("system_basic", "structured_keys") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["config", "config_basic", "system_basic"],
                &["list_keys", "structured_keys"],
            )
        }
        ("fs_basic", "stat_paths") | ("system_basic", "path_batch_facts") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic", "system_basic"],
                &["stat_paths", "stat_path", "path_batch_facts"],
            )
        }
        ("fs_basic", "list_dir") | ("system_basic", "inventory_dir") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic", "system", "system_basic"],
                &["list_dir", "list_entries", "inventory_dir"],
            )
        }
        ("fs_basic", "count_entries") | ("system_basic", "count_inventory") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic", "system_basic"],
                &["count_entries", "count_inventory"],
            )
        }
        ("fs_basic", "read_text_range") | ("system_basic", "read_range") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic", "system", "system_basic"],
                &["read_text_range", "read_text", "read_file", "read_range"],
            )
        }
        ("fs_basic", "find_entries") | ("system_basic", "find_path") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic", "system_basic"],
                &["find_entries", "find_files", "find_paths", "find_path"],
            )
        }
        ("fs_basic", "grep_text") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic"],
                &["grep_text", "search_text"],
            )
        }
        ("fs_basic", "compare_paths") | ("system_basic", "compare_paths") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic", "system", "system_basic"],
                &["compare_paths"],
            )
        }
        ("fs_basic", "write_text") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic"],
                &["write_text", "write_file"],
            )
        }
        ("fs_basic", "append_text") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic"],
                &["append_text", "append_file"],
            )
        }
        ("fs_basic", "make_dir") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic"],
                &["make_dir", "create_dir"],
            )
        }
        ("fs_basic", "remove_path") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["filesystem", "fs", "fs_basic"],
                &["remove_path", "delete_path"],
            )
        }
        ("service_control", "status") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["service", "service_control"],
                &["status"],
            )
        }
        ("service_control", "verify") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["service", "service_control"],
                &["verify"],
            )
        }
        ("service_control", "logs") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["service", "service_control"],
                &["logs"],
            )
        }
        ("service_control", "start" | "stop" | "restart") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["service", "service_control"],
                &[action_name],
            )
        }
        ("docker_basic", "ps") => crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["docker", "docker_basic"],
            &["list_containers", "ps"],
        ),
        ("docker_basic", "images") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["docker", "docker_basic"],
                &["list_images", "images"],
            )
        }
        ("docker_basic", "version") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["docker", "docker_basic"],
                &["version"],
            )
        }
        ("docker_basic", "inspect") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["docker", "docker_basic"],
                &["inspect_container", "inspect"],
            )
        }
        ("docker_basic", "logs") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["docker", "docker_basic"],
                &["read_logs", "logs"],
            )
        }
        ("docker_basic", "restart") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["docker", "docker_basic"],
                &["restart", "restart_container"],
            )
        }
        ("docker_basic", "start") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["docker", "docker_basic"],
                &["start", "start_container"],
            )
        }
        ("docker_basic", "stop") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["docker", "docker_basic"],
                &["stop", "stop_container"],
            )
        }
        ("system_basic", "info") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["system", "system_basic"],
                &["info"],
            )
        }
        ("system_basic", "runtime_status") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["system", "system_basic"],
                &["runtime_status"],
            )
        }
        ("system_basic", "tree_summary") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["system", "system_basic"],
                &["tree_summary"],
            )
        }
        ("process_basic", "ps") => crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["process", "process_basic"],
            &["ps"],
        ),
        ("process_basic", "port_list") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["process", "process_basic"],
                &["port_list"],
            )
        }
        ("process_basic", "kill") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["process", "process_basic"],
                &["kill"],
            )
        }
        ("process_basic", "tail_log") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["process", "process_basic"],
                &["tail_log"],
            )
        }
        ("task_control", "list")
        | ("task_control", "list_with_first_detail")
        | ("task_control", "get")
        | ("task_control", "cancel_all")
        | ("task_control", "cancel_one")
        | ("task_control", "resume")
        | ("task_control", "pause") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["task_control"],
                &[action_name],
            )
        }
        ("config_edit", "plan_config_change") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["config"],
                &["plan_change", "plan_config_change"],
            )
        }
        ("config_edit", "apply_config_change") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["config"],
                &["apply_change", "apply_config_change"],
            )
        }
        ("archive_basic", "list") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["archive"],
                &["list"],
            )
        }
        ("archive_basic", "read") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["archive"],
                &["read"],
            )
        }
        ("archive_basic", "pack") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["archive"],
                &["pack"],
            )
        }
        ("archive_basic", "unpack") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["archive"],
                &["unpack"],
            )
        }
        ("db_basic", "list_tables") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["database", "db", "sqlite"],
                &["list_tables", "list"],
            )
        }
        ("db_basic", "schema_version") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["database", "db", "sqlite"],
                &["schema_version"],
            )
        }
        ("db_basic", "sqlite_query") => {
            crate::machine_capability_ref::route_has_capability_action_name(
                route,
                &["database", "db", "sqlite"],
                &["query", "sqlite_query"],
            )
        }
        ("git_basic", "status") => crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["git"],
            &["status", "repository_state"],
        ),
        _ => false,
    }
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

pub(crate) fn arg_policy_decision(
    output_contract: Option<&IntentOutputContract>,
    normalized_skill: &str,
    resolved_args: &Value,
) -> Option<ContractArgPolicy> {
    let output_contract = output_contract?;
    if output_contract.semantic_kind_is_unclassified()
        && !output_contract.requires_content_evidence
        && !output_contract.delivery_required
    {
        return None;
    }
    let matrix = bundled_contract_matrix()?;
    let matched = matrix.match_output_contract(output_contract)?;
    let action = policy_action_ref_for_match(&matched, normalized_skill, resolved_args)?;
    let final_answer_shape_kind = matched.final_answer_shape_kind()?;
    let (expected_target_args, missing_target_args, deferred_target_args, decision) =
        arg_target_policy_decision(output_contract, &action.effective, resolved_args);
    Some(ContractArgPolicy {
        decision,
        action_key: action.effective.as_key(),
        contract_match: matched.match_name().to_string(),
        required_evidence: matched.required_evidence(),
        missing_target_args,
        deferred_target_args,
        expected_target_args,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        policy_mode: matched.policy_mode(),
        evidence_scope: matched.evidence_scope(),
        freshness: matched.freshness(),
        artifact_kind: matched.artifact_kind(),
        channel_visibility: matched.channel_visibility(),
        evidence_profile: matched.evidence_profile(),
    })
}

fn arg_target_policy_decision(
    output_contract: &IntentOutputContract,
    action: &ActionRef,
    resolved_args: &Value,
) -> (Vec<String>, Vec<String>, Vec<String>, ArgPolicyDecision) {
    let target_groups = contract_target_arg_groups(output_contract, action);
    let expected_target_args = target_groups
        .iter()
        .flat_map(|group| group.iter().copied())
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut missing_target_args = Vec::new();
    let mut deferred_target_args = Vec::new();
    for group in &target_groups {
        if arg_group_has_concrete_value(resolved_args, group) {
            continue;
        }
        let group_label = group.join("|");
        if arg_group_has_unresolved_template(resolved_args, group) {
            deferred_target_args.push(group_label);
        } else {
            missing_target_args.push(group_label);
        }
    }
    let decision = if !deferred_target_args.is_empty() {
        ArgPolicyDecision::DeferredTemplateArg
    } else if !missing_target_args.is_empty() {
        ArgPolicyDecision::MissingTargetBinding
    } else {
        ArgPolicyDecision::Allowed
    };
    (
        expected_target_args,
        missing_target_args,
        deferred_target_args,
        decision,
    )
}

pub(crate) fn arg_policy_decision_for_route(
    route: Option<&RouteResult>,
    normalized_skill: &str,
    resolved_args: &Value,
) -> Option<ContractArgPolicy> {
    let route = route?;
    let output_contract = route.effective_output_contract();
    let mut policy =
        match arg_policy_decision(Some(&output_contract), normalized_skill, resolved_args) {
            Some(policy) => policy,
            None => return route_capability_ref_arg_policy(route, normalized_skill, resolved_args),
        };
    if route_capability_ref_allows_action(route, normalized_skill, resolved_args) {
        if let Some(action) = route_capability_policy_action_ref(normalized_skill, resolved_args) {
            let (expected, missing, deferred, decision) =
                arg_target_policy_decision(&output_contract, &action.effective, resolved_args);
            policy.action_key = action.effective.as_key();
            policy.expected_target_args = expected;
            policy.missing_target_args = missing;
            policy.deferred_target_args = deferred;
            policy.decision = decision;
        }
        policy.contract_match = "capability_ref".to_string();
        policy.required_evidence = crate::task_contract::required_evidence_fields_for_route(route);
    }
    Some(policy)
}

fn route_capability_ref_arg_policy(
    route: &RouteResult,
    normalized_skill: &str,
    resolved_args: &Value,
) -> Option<ContractArgPolicy> {
    if !route_capability_ref_allows_action(route, normalized_skill, resolved_args) {
        return None;
    }
    let output_contract = route.effective_output_contract();
    let action = route_capability_policy_action_ref(normalized_skill, resolved_args)?;
    let final_answer_shape_kind =
        final_answer_shape_for_route(route).unwrap_or(FinalAnswerShape::Free);
    let (expected, missing, deferred, decision) =
        arg_target_policy_decision(&output_contract, &action.effective, resolved_args);
    Some(ContractArgPolicy {
        decision,
        action_key: action.effective.as_key(),
        contract_match: "capability_ref".to_string(),
        required_evidence: crate::task_contract::required_evidence_fields_for_route(route),
        missing_target_args: missing,
        deferred_target_args: deferred,
        expected_target_args: expected,
        final_answer_shape: final_answer_shape_kind.as_str().to_string(),
        policy_mode: "observe".to_string(),
        evidence_scope: "conversation".to_string(),
        freshness: "conversation".to_string(),
        artifact_kind: "text".to_string(),
        channel_visibility: "user_visible".to_string(),
        evidence_profile: "capability_ref".to_string(),
    })
}

pub(crate) fn action_matches_policy_tokens(action_key: &str, policies: &[String]) -> bool {
    let Some(action) = ActionRef::parse(action_key) else {
        return false;
    };
    action_matches_any(&action, policies)
}

fn contract_target_arg_groups(
    output_contract: &IntentOutputContract,
    action: &ActionRef,
) -> Vec<Vec<&'static str>> {
    if !output_contract.requires_content_evidence && !output_contract.delivery_required {
        return Vec::new();
    }
    if !matches!(
        output_contract.locator_kind,
        OutputLocatorKind::Path | OutputLocatorKind::Filename
    ) && !output_contract.delivery_required
    {
        return Vec::new();
    }
    match (action.skill.as_str(), action.action.as_deref()) {
        ("fs_basic", Some("compare_paths")) => vec![vec!["left_path"], vec!["right_path"]],
        ("fs_basic", Some("stat_paths")) => vec![vec!["path", "paths"]],
        ("fs_basic", Some("count_entries")) => vec![vec!["path"]],
        ("fs_basic", Some("list_dir" | "read_text_range")) => vec![vec!["path"]],
        ("fs_basic", Some("grep_text")) => vec![vec!["root", "path"]],
        ("fs_basic", Some("write_text" | "append_text" | "make_dir" | "remove_path")) => {
            vec![vec!["path"]]
        }
        ("doc_parse", _) => vec![vec!["path", "file_path", "requested_path"]],
        ("archive_basic", Some("list" | "read")) => vec![vec!["archive", "archive_path", "path"]],
        ("archive_basic", Some("pack")) => vec![vec!["source", "source_path", "path"]],
        ("archive_basic", Some("unpack")) => {
            vec![
                vec!["archive", "archive_path", "path"],
                vec!["dest", "dest_path"],
            ]
        }
        (
            "config_basic",
            Some("read_field" | "read_fields" | "list_keys" | "validate" | "guard_rustclaw_config"),
        ) => vec![vec!["path"]],
        (
            "config_edit",
            Some(
                "plan_config_change"
                | "apply_config_change"
                | "validate_config"
                | "guard_config"
                | "read_back"
                | "restart_if_requested",
            ),
        ) => vec![vec!["path"]],
        ("db_basic", _) => vec![vec!["db_path", "path"]],
        _ => Vec::new(),
    }
}

fn arg_group_has_concrete_value(args: &Value, group: &[&str]) -> bool {
    group
        .iter()
        .any(|name| args.get(*name).is_some_and(arg_value_is_concrete))
}

fn arg_group_has_unresolved_template(args: &Value, group: &[&str]) -> bool {
    group.iter().any(|name| {
        args.get(*name)
            .is_some_and(arg_value_has_unresolved_template)
    })
}

fn arg_value_is_concrete(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            !trimmed.is_empty() && !string_has_unresolved_template(trimmed)
        }
        Value::Array(values) => values.iter().any(arg_value_is_concrete),
        Value::Object(map) => map.values().any(arg_value_is_concrete),
        Value::Null => false,
        Value::Bool(_) | Value::Number(_) => true,
    }
}

fn arg_value_has_unresolved_template(value: &Value) -> bool {
    match value {
        Value::String(text) => string_has_unresolved_template(text),
        Value::Array(values) => values.iter().any(arg_value_has_unresolved_template),
        Value::Object(map) => map.values().any(arg_value_has_unresolved_template),
        _ => false,
    }
}

fn string_has_unresolved_template(value: &str) -> bool {
    value.contains("{{") && value.contains("}}")
}

#[cfg(test)]
pub(crate) fn available_action_refs_from_registry(registry: &SkillsRegistry) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for name in registry.all_names() {
        out.insert(name.clone());
        if let Some(manifest) = registry.manifest(&name) {
            if let Some(action) = manifest.runtime_action.as_deref() {
                if let Some(action_ref) = ActionRef::parse(&format!("{name}.{action}")) {
                    out.insert(action_ref.as_key());
                }
            }
            for capability in manifest.planner_capabilities {
                if let Some(action) = capability.action.as_deref() {
                    if let Some(action_ref) = ActionRef::parse(&format!("{name}.{action}")) {
                        out.insert(action_ref.as_key());
                    }
                }
            }
            collect_input_schema_action_refs(&mut out, &name, manifest.input_schema.as_ref());
        }
    }
    out
}

#[cfg(test)]
fn collect_input_schema_action_refs(
    out: &mut BTreeSet<String>,
    skill: &str,
    schema: Option<&Value>,
) {
    let Some(schema) = schema else {
        return;
    };
    let Some(action_schema) = schema
        .get("properties")
        .and_then(Value::as_object)
        .and_then(|properties| properties.get("action"))
    else {
        return;
    };
    let Some(action_enum) = action_schema.get("enum").and_then(Value::as_array) else {
        return;
    };
    for action in action_enum.iter().filter_map(Value::as_str) {
        if let Some(action_ref) = ActionRef::parse(&format!("{skill}.{action}")) {
            out.insert(action_ref.as_key());
        }
    }
}

#[cfg(test)]
pub(super) fn collect_action_tokens(out: &mut BTreeSet<String>, values: &[String]) {
    for value in values {
        if let Some(action_ref) = ActionRef::parse(value) {
            out.insert(action_ref.as_key());
        }
    }
}

#[cfg(test)]
pub(super) fn collect_external_observation_admission_errors(
    errors: &mut BTreeSet<String>,
    context: &str,
    observation_sources: &[String],
    observation_extractors: &[ObservationExtractor],
    registry: &SkillsRegistry,
) {
    for token in observation_sources {
        let Some(action_ref) = ActionRef::parse(token) else {
            continue;
        };
        let Some(entry) = registry.get(&action_ref.skill) else {
            continue;
        };
        let requires_admission = entry.matrix_admission.is_some()
            || entry.kind == SkillKind::External
            || entry
                .external_bundle_dir
                .as_deref()
                .is_some_and(|value| !value.trim().is_empty());
        if !requires_admission {
            continue;
        }
        if !registry.matrix_admission_eligible(&action_ref.skill, action_ref.action.as_deref()) {
            errors.insert(format!(
                "{context} observation_source `{}` requires matrix_admission.eligible=true for strict evidence use",
                action_ref.as_key()
            ));
        }
        let uses_text_legacy = observation_extractors.iter().any(|extractor| {
            extractor.extractor_kind == "text_legacy"
                && (extractor.source == action_ref.as_key() || extractor.source == action_ref.skill)
        });
        if uses_text_legacy {
            let admission_allows_text_legacy = entry
                .matrix_admission
                .as_ref()
                .and_then(|admission| admission.extractor_kind.as_deref())
                .is_some_and(|kind| normalize_action_token(kind) == "text_legacy");
            if !admission_allows_text_legacy {
                errors.insert(format!(
                    "{context} observation_source `{}` uses text_legacy extractor without matrix_admission.extractor_kind=text_legacy",
                    action_ref.as_key()
                ));
            }
        }
    }
}

pub(super) fn action_matches_any(action: &ActionRef, policies: &[String]) -> bool {
    policies.iter().any(|policy| {
        let Some(policy_ref) = ActionRef::parse(policy) else {
            return false;
        };
        if action.skill != policy_ref.skill {
            return false;
        }
        match &policy_ref.action {
            Some(policy_action) => action
                .action
                .as_deref()
                .is_some_and(|action| action == policy_action),
            None => true,
        }
    })
}

pub(super) fn contains_token(values: &[String], needle: &str) -> bool {
    values
        .iter()
        .any(|value| normalize_action_token(value) == normalize_action_token(needle))
}

pub(super) fn normalized_tokens(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| normalize_action_token(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(super) fn observation_extractors_for_sources(
    sources: Vec<String>,
    configured_extractors: &[ObservationExtractor],
) -> Vec<ObservationExtractor> {
    let mut extractors = BTreeMap::new();
    for source in sources {
        if let Some(extractor) = ObservationExtractor::from_source(&source) {
            extractors.insert(extractor.stable_key(), extractor);
        }
    }
    for configured in configured_extractors {
        if let Some(extractor) =
            ObservationExtractor::normalized(&configured.source, &configured.extractor_kind)
        {
            extractors.insert(extractor.stable_key(), extractor);
        }
    }
    extractors.into_values().collect()
}

fn observation_extractors_trace_json(extractors: &[ObservationExtractor]) -> Value {
    json!(extractors
        .iter()
        .map(ObservationExtractor::to_trace_json)
        .collect::<Vec<_>>())
}

pub(super) fn observation_extractors_stable_key(extractors: &[ObservationExtractor]) -> String {
    extractors
        .iter()
        .map(ObservationExtractor::stable_key)
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn default_extractor_kind_for_observation_source(source: &str) -> &'static str {
    match normalize_action_token(source).as_str() {
        "run_cmd" | "http_basic" => "text_legacy",
        _ => "structured_json",
    }
}

pub(super) fn normalized_extractor_kind(value: &str) -> String {
    normalized_contract_field(value, "structured_json")
}

fn extractor_kind_is_valid(value: &str) -> bool {
    matches!(value, "structured_json" | "text_legacy")
}

pub(super) fn normalized_contract_field(value: &str, default: &str) -> String {
    let normalized = normalize_action_token(value);
    if normalized.is_empty() {
        default.to_string()
    } else {
        normalized
    }
}

fn validate_contract_field(
    errors: &mut Vec<String>,
    context: &str,
    field: &str,
    raw: &str,
    default: &str,
    allowed: &[&str],
) {
    let value = normalized_contract_field(raw, default);
    if !allowed.contains(&value.as_str()) {
        errors.push(format!("{context} has invalid {field} `{raw}`"));
    }
}

pub(super) fn validate_contract_runtime_fields(
    errors: &mut Vec<String>,
    context: &str,
    policy_mode: &str,
    evidence_scope: &str,
    freshness: &str,
    artifact_kind: &str,
    channel_visibility: &str,
    evidence_profile: &str,
) {
    validate_contract_field(
        errors,
        context,
        "policy_mode",
        policy_mode,
        "enforce",
        &["observe", "enforce"],
    );
    validate_contract_field(
        errors,
        context,
        "evidence_scope",
        evidence_scope,
        "current_task",
        &[
            "current_step",
            "current_task",
            "active_task",
            "conversation",
            "long_term_memory",
        ],
    );
    validate_contract_field(
        errors,
        context,
        "freshness",
        freshness,
        "current_task",
        &[
            "realtime",
            "current_task",
            "active_task",
            "conversation",
            "long_term_memory",
        ],
    );
    validate_contract_field(
        errors,
        context,
        "artifact_kind",
        artifact_kind,
        "text",
        &["text", "file", "image", "audio", "url"],
    );
    validate_contract_field(
        errors,
        context,
        "channel_visibility",
        channel_visibility,
        "user_visible",
        &["user_visible", "trace_only"],
    );
    let evidence_profile = normalize_action_token(evidence_profile);
    if !evidence_profile.is_empty()
        && evidence_profile != "generic"
        && evidence_profile != "workspace_user_docs_first"
    {
        errors.push(format!(
            "{context} has invalid evidence_profile `{evidence_profile}`"
        ));
    }
}

pub(super) fn validate_artifact_shape_contract(
    errors: &mut Vec<String>,
    context: &str,
    delivery_shape: Option<&str>,
    final_answer_shape: &str,
    artifact_kind: &str,
    channel_visibility: &str,
) {
    let normalized_artifact = normalized_contract_field(artifact_kind, "text");
    let normalized_visibility = normalized_contract_field(channel_visibility, "user_visible");
    let shape = FinalAnswerShape::parse(final_answer_shape);
    if shape.is_some_and(|shape| shape.class() == FinalAnswerShapeClass::DeliveryArtifact) {
        if normalized_artifact == "text" {
            errors.push(format!(
                "{context} delivery artifact final_answer_shape must declare non-text artifact_kind"
            ));
        }
        if normalized_visibility != "user_visible" {
            errors.push(format!(
                "{context} delivery artifact final_answer_shape must be user_visible"
            ));
        }
    }
    if delivery_shape.is_some_and(|value| normalize_action_token(value) == "file")
        && normalized_artifact != "file"
    {
        errors.push(format!(
            "{context} delivery_shape=file must declare artifact_kind=file"
        ));
    }
}

pub(super) fn validate_observation_extractors(
    errors: &mut Vec<String>,
    context: &str,
    observation_sources: &[String],
    configured_extractors: &[ObservationExtractor],
) {
    let known_sources = observation_sources
        .iter()
        .map(|source| normalize_action_token(source))
        .collect::<BTreeSet<_>>();
    let mut seen_extractors = BTreeSet::new();
    for extractor in configured_extractors {
        let source = normalize_action_token(&extractor.source);
        if source.is_empty() {
            errors.push(format!(
                "{context} has observation_extractor without source"
            ));
            continue;
        }
        if !known_sources.contains(&source) {
            errors.push(format!(
                "{context} observation_extractor source `{}` is not in observation_sources",
                extractor.source
            ));
        }
        let extractor_kind = normalized_extractor_kind(&extractor.extractor_kind);
        if !extractor_kind_is_valid(&extractor_kind) {
            errors.push(format!(
                "{context} observation_extractor source `{}` has invalid extractor_kind `{}`",
                extractor.source, extractor.extractor_kind
            ));
            continue;
        }
        let extractor_key = format!("{source}={extractor_kind}");
        if !seen_extractors.insert(extractor_key) {
            errors.push(format!(
                "{context} has duplicate observation_extractor source `{}` extractor_kind `{}`",
                extractor.source, extractor.extractor_kind
            ));
        }
        if !crate::task_journal::evidence_extractor_registry_contains(&source, &extractor_kind) {
            errors.push(format!(
                "{context} observation_extractor source `{}` with extractor_kind `{}` is not declared in the evidence extractor registry",
                extractor.source, extractor.extractor_kind
            ));
        }
    }
}

pub(super) fn evidence_expression_tokens(expression: &EvidenceExpression) -> Vec<String> {
    let mut tokens = BTreeSet::new();
    for value in expression
        .all_of
        .iter()
        .chain(expression.one_of.iter())
        .chain(expression.any_of.iter())
        .chain(expression.negative_evidence.iter())
    {
        let normalized = normalize_action_token(value);
        if !normalized.is_empty() {
            tokens.insert(normalized);
        }
    }
    tokens.into_iter().collect()
}

pub(super) fn normalize_action_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

pub(crate) fn fnv1a_hex(input: &str) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn bundled_registry_hash() -> String {
    fnv1a_hex(include_str!("../../../configs/skills_registry.toml"))
}

fn bundled_prompt_layer_manifest_hash() -> String {
    fnv1a_hex(include_str!("../../../prompts/layers/manifest.toml"))
}
