use super::*;
use claw_core::skill_registry::SkillsRegistry;

fn registry_entry_from(toml: &str, name: &str) -> SkillRegistryEntry {
    let path = std::env::temp_dir().join(format!("skill_availability_{name}.toml"));
    std::fs::write(&path, toml).unwrap();
    let registry = SkillsRegistry::load_from_path(&path).unwrap();
    let entry = registry.get(name).unwrap().clone();
    let _ = std::fs::remove_file(path);
    entry
}

#[test]
fn missing_required_bin_marks_skill_unavailable() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "needs_missing_bin"
enabled = true
supported_os = ["linux", "macos"]
required_bins = ["definitely_missing_rustclaw_test_bin_20260511"]
"#,
        "needs_missing_bin",
    );
    let availability = evaluate_entry_availability(&entry);
    assert!(!availability.is_available());
    assert_eq!(
        availability.missing_required_bins,
        vec!["definitely_missing_rustclaw_test_bin_20260511"]
    );
}

#[test]
fn unsupported_os_marks_skill_unavailable() {
    let entry = registry_entry_from(
        r#"
[[skills]]
name = "wrong_os"
enabled = true
supported_os = ["definitely-not-this-os"]
"#,
        "wrong_os",
    );
    let availability = evaluate_entry_availability(&entry);
    assert!(!availability.is_available());
    assert_eq!(
        availability.unsupported_os,
        Some(vec!["definitely-not-this-os".to_string()])
    );
}
