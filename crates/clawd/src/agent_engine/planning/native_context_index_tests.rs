use super::*;
use serde_json::Value;
use std::collections::BTreeMap;

fn group(name: &str, capabilities: &[&str]) -> PlannerNativeCapabilityGroup {
    PlannerNativeCapabilityGroup {
        skill_name: name.into(),
        tool_name: format!("call_{name}"),
        description: "contract-detail-sentinel".into(),
        capability_names: capabilities.iter().map(|v| v.to_string()).collect(),
        capability_argument_schemas: BTreeMap::from([(
            "sample".into(),
            json!({"private_default":"schema-sentinel"}),
        )]),
        capability_descriptions: BTreeMap::new(),
    }
}

#[test]
fn complete_index_keeps_authorized_unloaded_groups_without_duplicate_contracts() {
    let text = build_index(
        vec![group("z", &["z.b", "z.a"]), group("a", &["a.read"])],
        &BTreeSet::from(["a".into()]),
        vec!["mcp.sample".into()],
    );
    assert!(!text.contains("contract-detail-sentinel") && !text.contains("schema-sentinel"));
    let value: Value = serde_json::from_str(text.split_once('\n').unwrap().1).unwrap();
    assert_eq!(value["catalog_complete"], true);
    assert_eq!(value["groups"][0]["group"], "a");
    assert_eq!(value["groups"][0]["disclosed"], true);
    assert_eq!(value["groups"][1]["capabilities"], json!(["z.a", "z.b"]));
    assert_eq!(value["groups"][1]["disclosed"], false);
    assert_eq!(value["mcp_capabilities"], json!(["mcp.sample"]));
    assert_eq!(value["dispatch"], "resolver_verifier");
}

#[test]
fn index_is_stable_and_never_introduces_unavailable_tools() {
    let a = build_index(
        vec![group("z", &["z.b", "z.a"]), group("a", &["a.read"])],
        &BTreeSet::new(),
        vec![],
    );
    let b = build_index(
        vec![group("a", &["a.read"]), group("z", &["z.a", "z.b"])],
        &BTreeSet::new(),
        vec![],
    );
    assert_eq!(a, b);
    let empty = build_index(vec![], &BTreeSet::new(), vec![]);
    assert!(!empty.contains("z.a") && !empty.contains("a.read"));
}
