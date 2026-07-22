use std::collections::{BTreeMap, BTreeSet};

use super::{remove_skill_registry_block, render_skill_store_config};

#[test]
fn skill_store_config_keeps_switch_and_uninstall_state_distinct() {
    let raw = "[skills]\nskill_switches = { weather = true }\nskills_list = [\"weather\"]\n";
    let switches = BTreeMap::from([("weather".to_string(), false)]);
    let uninstalled = BTreeSet::from(["weather".to_string()]);

    let updated = render_skill_store_config(raw, &switches, &uninstalled);
    let parsed = toml::from_str::<toml::Value>(&updated).expect("valid config");

    assert_eq!(
        parsed["skills"]["skill_switches"]["weather"].as_bool(),
        Some(false)
    );
    assert_eq!(
        parsed["skills"]["uninstalled_skills"][0].as_str(),
        Some("weather")
    );
}

#[test]
fn reimport_removes_every_existing_registry_block_before_append() {
    let raw = "[[skills]]\nname = \"demo\"\nenabled = true\n\n[[skills]]\nname = \"keep\"\nenabled = true\n\n[[skills]]\nname = \"demo\"\nenabled = false\n";

    let (updated, removed) = remove_skill_registry_block(raw, "demo");

    assert!(removed);
    assert!(!updated.contains("name = \"demo\""));
    assert_eq!(updated.matches("name = \"keep\"").count(), 1);
}
