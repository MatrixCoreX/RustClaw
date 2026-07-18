use claw_core::skill_registry::{OutputKind, SkillManifest, SkillRiskLevel};
use serde_json::Value;
use std::collections::BTreeSet;

const QUICK_INDEX_MAX_PLANNER_CAPABILITIES: usize = 6;
const QUICK_INDEX_MAX_SCHEMA_FIELDS: usize = 8;
const QUICK_INDEX_MAX_ENUM_FIELDS: usize = 3;
const QUICK_INDEX_MAX_ENUM_VALUES: usize = 8;

fn skill_risk_level_token(risk_level: SkillRiskLevel) -> &'static str {
    match risk_level {
        SkillRiskLevel::Unknown => "unknown",
        SkillRiskLevel::Low => "low",
        SkillRiskLevel::Medium => "medium",
        SkillRiskLevel::High => "high",
    }
}

fn output_kind_token(kind: OutputKind) -> &'static str {
    match kind {
        OutputKind::Text => "text",
        OutputKind::File => "file",
        OutputKind::Image => "image",
        OutputKind::Mixed => "mixed",
    }
}

fn compact_token_list(values: Vec<String>, limit: usize) -> String {
    let mut unique = BTreeSet::new();
    for value in values {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            unique.insert(trimmed.to_string());
        }
    }
    let total = unique.len();
    let mut kept = unique.into_iter().take(limit).collect::<Vec<_>>();
    if total > kept.len() {
        kept.push(format!("+{}more", total - kept.len()));
    }
    kept.join("|")
}

fn schema_string_array(schema: &Value, key: &str) -> Vec<String> {
    schema
        .get(key)
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn schema_property_names(schema: &Value) -> Vec<String> {
    schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default()
}

fn capability_enum_constraints(
    schema: Option<&Value>,
    required: &[String],
    optional: &[String],
) -> Vec<String> {
    let Some(properties) = schema
        .and_then(|value| value.get("properties"))
        .and_then(Value::as_object)
    else {
        return Vec::new();
    };

    required
        .iter()
        .chain(optional)
        .flat_map(|field| field.split('|'))
        .map(str::trim)
        .filter(|field| !field.is_empty())
        .filter_map(|field| {
            let values = properties
                .get(field)
                .and_then(|property| property.get("enum"))
                .and_then(Value::as_array)?
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let values = compact_token_list(values, QUICK_INDEX_MAX_ENUM_VALUES);
            (!values.is_empty()).then(|| format!("allowed_{field}={values}"))
        })
        .take(QUICK_INDEX_MAX_ENUM_FIELDS)
        .collect()
}

pub(super) fn output_contract_metadata(manifest: &SkillManifest) -> String {
    let mut attrs = vec![format!("kind={}", output_kind_token(manifest.output_kind))];
    if let Some(schema) = manifest.output_schema.as_ref() {
        let required = compact_token_list(
            schema_string_array(schema, "required"),
            QUICK_INDEX_MAX_SCHEMA_FIELDS,
        );
        if !required.is_empty() {
            attrs.push(format!("required={required}"));
        }
        let fields =
            compact_token_list(schema_property_names(schema), QUICK_INDEX_MAX_SCHEMA_FIELDS);
        if !fields.is_empty() {
            attrs.push(format!("fields={fields}"));
        }
    }
    format!("output_contract: {}", attrs.join(","))
}

pub(super) fn output_contract(manifest: &SkillManifest) -> String {
    format!("; {}", output_contract_metadata(manifest))
}

fn planner_capability_tokens(manifest: &SkillManifest) -> Vec<String> {
    manifest
        .planner_capabilities
        .iter()
        .take(QUICK_INDEX_MAX_PLANNER_CAPABILITIES)
        .map(|capability| {
            let name = capability.name.trim();
            let mut attrs = Vec::new();
            if let Some(action) = capability.action.as_deref() {
                if !action.trim().is_empty() {
                    attrs.push(format!("action={}", action.trim()));
                }
            }
            if let Some(effect) = capability.effect {
                attrs.push(format!("effect={}", effect.as_token()));
            }
            if !capability.required.is_empty() {
                attrs.push(format!("required={}", capability.required.join("|")));
            }
            if !capability.optional.is_empty() {
                attrs.push(format!("optional={}", capability.optional.join("|")));
            }
            if let Some(risk_level) = capability.risk_level.or(manifest.risk_level) {
                attrs.push(format!("risk={}", skill_risk_level_token(risk_level)));
            }
            if capability.preferred {
                attrs.push("preferred=true".to_string());
            }
            if let Some(once_per_task) = capability.once_per_task {
                attrs.push(format!("once_per_task={once_per_task}"));
            }
            if let Some(dedup_scope) = capability.dedup_scope {
                attrs.push(format!("dedup_scope={}", dedup_scope.as_token()));
            }
            if let Some(idempotent) = capability.idempotent {
                attrs.push(format!("idempotent={idempotent}"));
            }
            if let Some(execution_mode) = capability.execution_mode {
                attrs.push(format!("execution_mode={}", execution_mode.as_token()));
            }
            if let Some(async_adapter_kind) = capability.async_adapter_kind.as_deref() {
                if !async_adapter_kind.trim().is_empty() {
                    attrs.push(format!("async_adapter_kind={}", async_adapter_kind.trim()));
                }
            }
            if let Some(isolation_profile) = capability.isolation_profile {
                attrs.push(format!(
                    "isolation_profile={}",
                    isolation_profile.as_token()
                ));
            }
            if let Some(network_access) = capability.network_access {
                attrs.push(format!("network_access={network_access}"));
            }
            if let Some(filesystem_write) = capability.filesystem_write {
                attrs.push(format!("filesystem_write={filesystem_write}"));
            }
            if let Some(external_publish) = capability.external_publish {
                attrs.push(format!("external_publish={external_publish}"));
            }
            if let Some(credential_access) = capability.credential_access {
                attrs.push(format!("credential_access={credential_access}"));
            }
            if let Some(subprocess) = capability.subprocess {
                attrs.push(format!("subprocess={subprocess}"));
            }
            if let Some(package_install) = capability.package_install {
                attrs.push(format!("package_install={package_install}"));
            }
            if let Some(privilege_escalation) = capability.privilege_escalation {
                attrs.push(format!("privilege_escalation={privilege_escalation}"));
            }
            if let Some(output_semantic_kind) = capability.output_semantic_kind.as_deref() {
                if !output_semantic_kind.trim().is_empty() {
                    attrs.push(format!(
                        "output_semantic_kind={}",
                        output_semantic_kind.trim()
                    ));
                }
            }
            if let Some(final_answer_shape) = capability.final_answer_shape.as_deref() {
                if !final_answer_shape.trim().is_empty() {
                    attrs.push(format!("final_answer_shape={}", final_answer_shape.trim()));
                }
            }
            if attrs.is_empty() {
                name.to_string()
            } else {
                format!("{name}({})", attrs.join(","))
            }
        })
        .collect()
}

pub(super) fn planner_capabilities_metadata(manifest: &SkillManifest) -> Option<String> {
    let tokens = planner_capability_tokens(manifest);
    (!tokens.is_empty()).then(|| format!("planner_capabilities: {}", tokens.join("; ")))
}

pub(super) fn planner_capabilities(manifest: &SkillManifest) -> String {
    planner_capabilities_metadata(manifest)
        .map(|metadata| format!("; {metadata}"))
        .unwrap_or_default()
}

pub(super) fn planner_capability_candidates(manifest: &SkillManifest) -> String {
    let total = manifest.planner_capabilities.len();
    let mut candidates = manifest
        .planner_capabilities
        .iter()
        .take(QUICK_INDEX_MAX_PLANNER_CAPABILITIES)
        .map(|capability| {
            let mut attrs = Vec::new();
            if let Some(action) = capability.action.as_deref().map(str::trim) {
                if !action.is_empty() {
                    attrs.push(format!("action={action}"));
                }
            }
            if !capability.required.is_empty() {
                attrs.push(format!("required={}", capability.required.join("|")));
            }
            attrs.extend(capability_enum_constraints(
                manifest.input_schema.as_ref(),
                &capability.required,
                &capability.optional,
            ));
            if let Some(effect) = capability.effect {
                attrs.push(format!("effect={}", effect.as_token()));
            }
            if let Some(risk_level) = capability.risk_level.or(manifest.risk_level) {
                attrs.push(format!("risk={}", skill_risk_level_token(risk_level)));
            }
            if capability.preferred {
                attrs.push("preferred=true".to_string());
            }
            if let Some(output_semantic_kind) = capability.output_semantic_kind.as_deref() {
                if !output_semantic_kind.trim().is_empty() {
                    attrs.push(format!(
                        "output_semantic_kind={}",
                        output_semantic_kind.trim()
                    ));
                }
            }
            if let Some(final_answer_shape) = capability.final_answer_shape.as_deref() {
                if !final_answer_shape.trim().is_empty() {
                    attrs.push(format!("final_answer_shape={}", final_answer_shape.trim()));
                }
            }
            if attrs.is_empty() {
                capability.name.clone()
            } else {
                format!("{}({})", capability.name, attrs.join(","))
            }
        })
        .collect::<Vec<_>>();
    if total > candidates.len() {
        candidates.push(format!("+{}more", total - candidates.len()));
    }
    if candidates.is_empty() {
        String::new()
    } else {
        format!("; capability_candidates={}", candidates.join(";"))
    }
}
