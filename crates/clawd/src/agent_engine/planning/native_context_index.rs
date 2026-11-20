use crate::capability_map::PlannerNativeCapabilityGroup;
use serde_json::json;
use std::collections::BTreeSet;

// Exact contracts live in native tool schemas; a compact complete index keeps
// discovery available without a second copy of schemas and policy descriptions.
pub(super) fn build_index(
    mut groups: Vec<PlannerNativeCapabilityGroup>,
    disclosed: &BTreeSet<String>,
    mcp_capabilities: Vec<String>,
) -> String {
    groups.sort_by(|a, b| a.skill_name.cmp(&b.skill_name));
    let groups = groups.into_iter().map(|mut group| {
        group.capability_names.sort();
        json!({"group":group.skill_name,"disclosed":disclosed.contains(&group.skill_name),"capabilities":group.capability_names})
    }).collect::<Vec<_>>();
    let mcp = mcp_capabilities.into_iter().collect::<BTreeSet<_>>();
    let index = json!({
        "schema_version":1,"catalog_complete":true,
        "contract_source":"native_model_tools","discovery_source":"skill_quick_index",
        "contract_expansion":"load_capability_groups","dispatch":"resolver_verifier",
        "groups":groups,"mcp_capabilities":mcp,
    });
    format!("runtime_capability_map_v2\n{index}")
}

#[cfg(test)]
#[path = "native_context_index_tests.rs"]
mod tests;
