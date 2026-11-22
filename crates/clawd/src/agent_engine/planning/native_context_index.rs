use crate::capability_map::PlannerNativeCapabilityGroup;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

// Keep every authorized group discoverable. Unloaded leaf names live in the
// canonical catalog and loader; their schemas become callable only after loading.
pub(super) fn build_index(
    mut groups: Vec<PlannerNativeCapabilityGroup>,
    disclosed: &BTreeSet<String>,
    mcp_capabilities: Vec<String>,
) -> String {
    groups.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
    for group in &mut groups {
        group.capability_names.sort();
    }
    let canonical = groups
        .iter()
        .map(|g| {
            json!({
                "group":g.skill_name,"capabilities":g.capability_names
            })
        })
        .collect::<Vec<_>>();
    let canonical_sha256 = format!(
        "sha256:{:x}",
        Sha256::digest(serde_json::to_vec(&canonical).expect("catalog contains only JSON values"))
    );
    let capability_count: usize = groups.iter().map(|g| g.capability_names.len()).sum();
    let mut omitted = 0usize;
    let groups = groups
        .into_iter()
        .map(|group| {
            let visible = disclosed.contains(&group.skill_name);
            let count = group.capability_names.len();
            let capabilities = if visible {
                group.capability_names
            } else {
                omitted += count;
                Vec::new()
            };
            json!({"group":group.skill_name,"disclosed":visible,
            "capability_count":count,
            "capabilities":capabilities})
        })
        .collect::<Vec<_>>();
    let mcp = mcp_capabilities.into_iter().collect::<BTreeSet<_>>();
    let index = json!({
        "schema_version":2,"catalog_complete":omitted == 0,
        "group_catalog_complete":true,"capability_names_scope":"disclosed_groups_only","canonical_capability_count":capability_count,
        "canonical_catalog_sha256":canonical_sha256,"omitted_capability_count":omitted,
        "contract_source":"native_model_tools","discovery_source":"skill_quick_index",
        "contract_expansion":"load_capability_groups","dispatch":"resolver_verifier",
        "undisclosed_group_recovery":{"tool":"load_capability_groups",
            "op":"load_groups","selector":"groups","values":"exact_group_tokens"},
        "groups":groups,"mcp_capabilities":mcp,
    });
    format!("runtime_capability_map_v2\n{index}")
}

#[cfg(test)]
#[path = "native_context_index_tests.rs"]
mod tests;
