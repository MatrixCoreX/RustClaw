use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
use claw_core::skill_registry::{SkillKind, SkillsRegistry};

use super::*;

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
pub(in crate::contract_matrix) fn collect_action_tokens(
    out: &mut BTreeSet<String>,
    values: &[String],
) {
    for value in values {
        if let Some(action_ref) = ActionRef::parse(value) {
            out.insert(action_ref.as_key());
        }
    }
}

#[cfg(test)]
pub(in crate::contract_matrix) fn collect_external_observation_admission_errors(
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

pub(in crate::contract_matrix) fn action_matches_any(
    action: &ActionRef,
    policies: &[String],
) -> bool {
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

pub(in crate::contract_matrix) fn contains_token(values: &[String], needle: &str) -> bool {
    values
        .iter()
        .any(|value| normalize_action_token(value) == normalize_action_token(needle))
}

pub(in crate::contract_matrix) fn normalized_tokens(values: &[String]) -> Vec<String> {
    values
        .iter()
        .map(|value| normalize_action_token(value))
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(in crate::contract_matrix) fn observation_extractors_for_sources(
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

pub(super) fn observation_extractors_trace_json(extractors: &[ObservationExtractor]) -> Value {
    json!(extractors
        .iter()
        .map(ObservationExtractor::to_trace_json)
        .collect::<Vec<_>>())
}

pub(in crate::contract_matrix) fn observation_extractors_stable_key(
    extractors: &[ObservationExtractor],
) -> String {
    extractors
        .iter()
        .map(ObservationExtractor::stable_key)
        .collect::<Vec<_>>()
        .join(",")
}

pub(in crate::contract_matrix) fn default_extractor_kind_for_observation_source(
    source: &str,
) -> &'static str {
    match normalize_action_token(source).as_str() {
        "run_cmd" | "http_basic" => "text_legacy",
        _ => "structured_json",
    }
}

pub(in crate::contract_matrix) fn normalized_extractor_kind(value: &str) -> String {
    normalized_contract_field(value, "structured_json")
}

fn extractor_kind_is_valid(value: &str) -> bool {
    matches!(value, "structured_json" | "text_legacy")
}

pub(in crate::contract_matrix) fn normalized_contract_field(value: &str, default: &str) -> String {
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
        errors.push(format!(
            "contract_validation.invalid_field context={context} field={field} raw={raw}"
        ));
    }
}

pub(in crate::contract_matrix) fn validate_contract_runtime_fields(
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
            "contract_validation.invalid_evidence_profile context={context} evidence_profile={evidence_profile}"
        ));
    }
}

pub(in crate::contract_matrix) fn validate_artifact_shape_contract(
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
                "contract_validation.delivery_artifact_text_artifact context={context} artifact_kind={normalized_artifact}"
            ));
        }
        if normalized_visibility != "user_visible" {
            errors.push(format!(
                "contract_validation.delivery_artifact_visibility context={context} channel_visibility={normalized_visibility}"
            ));
        }
    }
    if delivery_shape.is_some_and(|value| normalize_action_token(value) == "file")
        && normalized_artifact != "file"
    {
        errors.push(format!(
            "contract_validation.delivery_shape_artifact_mismatch context={context} delivery_shape=file artifact_kind={normalized_artifact}"
        ));
    }
}

pub(in crate::contract_matrix) fn validate_observation_extractors(
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
                "contract_validation.observation_extractor_missing_source context={context}"
            ));
            continue;
        }
        if !known_sources.contains(&source) {
            errors.push(format!(
                "contract_validation.observation_extractor_unknown_source context={context} source={}",
                extractor.source
            ));
        }
        let extractor_kind = normalized_extractor_kind(&extractor.extractor_kind);
        if !extractor_kind_is_valid(&extractor_kind) {
            errors.push(format!(
                "contract_validation.observation_extractor_invalid_kind context={context} source={} extractor_kind={}",
                extractor.source, extractor.extractor_kind
            ));
            continue;
        }
        let extractor_key = format!("{source}={extractor_kind}");
        if !seen_extractors.insert(extractor_key) {
            errors.push(format!(
                "contract_validation.observation_extractor_duplicate context={context} source={} extractor_kind={}",
                extractor.source, extractor.extractor_kind
            ));
        }
        if !crate::task_journal::evidence_extractor_registry_contains(&source, &extractor_kind) {
            errors.push(format!(
                "contract_validation.observation_extractor_registry_missing context={context} source={} extractor_kind={}",
                extractor.source, extractor.extractor_kind
            ));
        }
    }
}

pub(in crate::contract_matrix) fn evidence_expression_tokens(
    expression: &EvidenceExpression,
) -> Vec<String> {
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

pub(in crate::contract_matrix) fn normalize_action_token(value: &str) -> String {
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

pub(super) fn bundled_registry_hash() -> String {
    fnv1a_hex(include_str!("../../../configs/skills_registry.toml"))
}

pub(super) fn bundled_prompt_layer_manifest_hash() -> String {
    fnv1a_hex(include_str!("../../../prompts/layers/manifest.toml"))
}
