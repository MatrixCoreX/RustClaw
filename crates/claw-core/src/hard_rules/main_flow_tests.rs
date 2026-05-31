use std::fs;
use std::path::PathBuf;

use uuid::Uuid;

use super::*;

fn temp_rules_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "rustclaw_main_flow_rules_{label}_{}.toml",
        Uuid::new_v4()
    ))
}

#[test]
fn missing_file_falls_back_to_defaults() {
    let path = temp_rules_path("missing");
    let defaults = MainFlowRules::defaults();
    let loaded = load_main_flow_rules(path.to_string_lossy().as_ref());
    assert_eq!(
        loaded.runtime_whatsapp_channel_aliases,
        defaults.runtime_whatsapp_channel_aliases
    );
    assert_eq!(
        loaded.duplicate_affirmation_scan_limit,
        defaults.duplicate_affirmation_scan_limit
    );
}

#[test]
fn invalid_toml_falls_back_to_defaults() {
    let path = temp_rules_path("invalid");
    fs::write(&path, "[duplicate_affirmation\nwindow_secs = 30").expect("write invalid toml");
    let defaults = MainFlowRules::defaults();
    let loaded = load_main_flow_rules(path.to_string_lossy().as_ref());
    let _ = fs::remove_file(&path);
    assert_eq!(loaded.whatsapp_web_adapters, defaults.whatsapp_web_adapters);
    assert_eq!(
        loaded.runtime_whatsapp_channel_aliases,
        defaults.runtime_whatsapp_channel_aliases
    );
}

#[test]
fn partially_invalid_values_keep_defaults_for_bad_fields() {
    let path = temp_rules_path("partial");
    fs::write(
        &path,
        r#"[whatsapp]
web_adapters = ["custom_web"]

[duplicate_affirmation]
window_secs = 0
scan_limit = 8
statuses = ["queued", "running"]
"#,
    )
    .expect("write partial toml");
    let defaults = MainFlowRules::defaults();
    let loaded = load_main_flow_rules(path.to_string_lossy().as_ref());
    let _ = fs::remove_file(&path);
    assert_eq!(loaded.whatsapp_web_adapters, vec!["custom_web".to_string()]);
    assert_eq!(
        loaded.duplicate_affirmation_window_secs,
        defaults.duplicate_affirmation_window_secs
    );
    assert_eq!(loaded.duplicate_affirmation_scan_limit, 8);
    assert_eq!(
        loaded.duplicate_affirmation_statuses,
        vec!["queued".to_string(), "running".to_string()]
    );
}
