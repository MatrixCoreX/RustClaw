use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

use crate::{AppState, ClaimedTask};

const CAPABILITY_REF_PREFIX: &str = "capability:";

#[derive(Debug, Clone, Serialize)]
pub(super) struct CapabilityCatalogEntry {
    pub(super) capability_ref: String,
    pub(super) skill_id: String,
    pub(super) capability_id: String,
    pub(super) contract_sha256: String,
    pub(super) purpose: String,
    pub(super) semantic_tags: Vec<String>,
    pub(super) action: Option<String>,
    pub(super) contract: Value,
    pub(super) playbook_artifact: Option<Value>,
}

pub(super) fn catalog_entries_for_task(
    state: &AppState,
    task: &ClaimedTask,
) -> Vec<CapabilityCatalogEntry> {
    let mut entries = Vec::new();
    let mut skills = state.planner_available_skills_for_task(task);
    skills.sort();
    for skill in skills {
        let Some(manifest) = state.skill_manifest(&skill) else {
            continue;
        };
        let playbook_artifact = playbook_artifact(state, &skill);
        for capability in &manifest.planner_capabilities {
            let capability_id = capability.name.trim();
            if capability_id.is_empty() {
                continue;
            }
            let argument_schema = claw_core::skill_registry::planner_capability_argument_schema(
                manifest.input_schema.as_ref(),
                capability,
            )
            .unwrap_or_else(|error| json!({"schema_error": error}));
            let contract = json!({
                "schema_version": 1,
                "skill_id": skill,
                "capability_id": capability_id,
                "action": capability.action,
                "purpose": capability.description,
                "semantic_tags": capability.semantic_tags,
                "effect": capability.effect.map(|effect| effect.as_token()),
                "required": capability.required,
                "optional": capability.optional,
                "risk": capability.risk_level.or(manifest.risk_level).map(risk_token),
                "argument_schema": argument_schema,
                "output_schema": manifest.output_schema,
                "execution": {
                    "preferred": capability.preferred,
                    "once_per_task": capability.once_per_task,
                    "idempotent": capability.idempotent,
                    "execution_mode": capability.execution_mode.map(|mode| mode.as_token()),
                    "isolation_profile": capability.isolation_profile.map(|profile| profile.as_token()),
                    "network_access": capability.network_access,
                    "filesystem_write": capability.filesystem_write,
                    "external_publish": capability.external_publish,
                    "credential_access": capability.credential_access,
                    "subprocess": capability.subprocess,
                    "package_install": capability.package_install,
                    "privilege_escalation": capability.privilege_escalation,
                },
            });
            let canonical = canonical_json(&contract);
            entries.push(CapabilityCatalogEntry {
                capability_ref: capability_ref(&skill, capability_id),
                skill_id: skill.clone(),
                capability_id: capability_id.to_string(),
                contract_sha256: sha256_hex(canonical.as_bytes()),
                purpose: capability
                    .description
                    .as_deref()
                    .unwrap_or_default()
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" "),
                semantic_tags: capability.semantic_tags.clone(),
                action: capability.action.clone(),
                contract,
                playbook_artifact: playbook_artifact.clone(),
            });
        }
    }
    entries.sort_by(|left, right| left.capability_ref.cmp(&right.capability_ref));
    entries
}

pub(super) fn search_catalog(state: &AppState, task: &ClaimedTask, query: &str) -> Value {
    let terms = query_terms(query);
    let mut matches = catalog_entries_for_task(state, task)
        .into_iter()
        .filter_map(|entry| {
            let score = search_score(&entry, &terms);
            (score > 0).then(|| {
                json!({
                    "capability_ref": entry.capability_ref,
                    "skill_id": entry.skill_id,
                    "capability_id": entry.capability_id,
                    "contract_sha256": entry.contract_sha256,
                    "purpose": entry.purpose,
                    "semantic_tags": entry.semantic_tags,
                    "action": entry.action,
                    "score": score,
                    "expand_action": "expand",
                })
            })
        })
        .collect::<Vec<_>>();
    matches.sort_by(|left, right| {
        right["score"]
            .as_u64()
            .cmp(&left["score"].as_u64())
            .then_with(|| {
                left["capability_ref"]
                    .as_str()
                    .cmp(&right["capability_ref"].as_str())
            })
    });
    json!({
        "schema_version": 1,
        "catalog_kind": "authorized_capability_search",
        "complete": true,
        "query": query,
        "returned_count": matches.len(),
        "matches": matches,
    })
}

pub(super) fn expand_catalog(
    state: &AppState,
    task: &ClaimedTask,
    references: &[String],
) -> Result<(Value, Vec<String>), String> {
    let entries = catalog_entries_for_task(state, task);
    let requested = references.iter().cloned().collect::<BTreeSet<_>>();
    if requested.is_empty() {
        return Err("capability_catalog_expand_refs_missing".to_string());
    }
    let selected = entries
        .into_iter()
        .filter(|entry| requested.contains(&entry.capability_ref))
        .collect::<Vec<_>>();
    let found = selected
        .iter()
        .map(|entry| entry.capability_ref.clone())
        .collect::<BTreeSet<_>>();
    let unavailable = requested.difference(&found).cloned().collect::<Vec<_>>();
    if !unavailable.is_empty() {
        return Err(json!({
            "error_code": "capability_contract_not_authorized",
            "unavailable_refs": unavailable,
        })
        .to_string());
    }
    let groups = selected
        .iter()
        .map(|entry| entry.skill_id.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    Ok((
        json!({
            "schema_version": 1,
            "catalog_kind": "authorized_capability_expansion",
            "complete": true,
            "returned_count": selected.len(),
            "contracts": selected,
        }),
        groups,
    ))
}

pub(super) fn compact_catalog_line(entry: &CapabilityCatalogEntry) -> String {
    format!(
        "{}(contract_sha256={},expand_ref={})",
        entry.capability_id, entry.contract_sha256, entry.capability_ref
    )
}

fn capability_ref(skill: &str, capability: &str) -> String {
    format!("{CAPABILITY_REF_PREFIX}{skill}/{capability}")
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|character: char| {
            !character.is_alphanumeric() && character != '_' && character != '-'
        })
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_lowercase)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn search_score(entry: &CapabilityCatalogEntry, terms: &[String]) -> u64 {
    if terms.is_empty() {
        return 1;
    }
    let capability = entry.capability_id.to_lowercase();
    let skill = entry.skill_id.to_lowercase();
    let action = entry.action.as_deref().unwrap_or_default().to_lowercase();
    let purpose = entry.purpose.to_lowercase();
    let tags = entry.semantic_tags.join(" ").to_lowercase();
    terms
        .iter()
        .map(|term| {
            u64::from(capability == *term) * 16
                + u64::from(capability.contains(term)) * 8
                + u64::from(skill.contains(term)) * 4
                + u64::from(action.contains(term)) * 4
                + u64::from(tags.contains(term)) * 3
                + u64::from(purpose.contains(term)) * 2
        })
        .sum()
}

fn playbook_artifact(state: &AppState, skill: &str) -> Option<Value> {
    let logical_path = state.skill_registry_prompt_rel_path(skill)?;
    let body = crate::load_prompt_template_for_state(state, &logical_path, "").0;
    let digest = sha256_hex(body.as_bytes());
    Some(json!({
        "artifact_id": format!("prompt:{digest}"),
        "logical_path": logical_path,
        "sha256": digest,
        "size_bytes": body.len(),
        "load_action": "load_capability_groups",
    }))
}

fn risk_token(risk: claw_core::skill_registry::SkillRiskLevel) -> &'static str {
    match risk {
        claw_core::skill_registry::SkillRiskLevel::Unknown => "unknown",
        claw_core::skill_registry::SkillRiskLevel::Low => "low",
        claw_core::skill_registry::SkillRiskLevel::Medium => "medium",
        claw_core::skill_registry::SkillRiskLevel::High => "high",
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Object(object) => {
            let sorted = object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect::<Map<_, _>>();
            serde_json::to_string(&Value::Object(sorted)).unwrap_or_default()
        }
        _ => serde_json::to_string(&canonicalize(value)).unwrap_or_default(),
    }
}

fn canonicalize(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), canonicalize(value)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(canonicalize).collect()),
        _ => value.clone(),
    }
}

#[cfg(test)]
#[path = "capability_catalog_tests.rs"]
mod tests;
