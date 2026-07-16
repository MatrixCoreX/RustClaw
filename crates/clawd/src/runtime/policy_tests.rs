use claw_core::config::ToolsConfig;

use super::policy::ToolsPolicy;

#[test]
fn default_coding_profile_is_an_explicit_local_capability_set() {
    let config = ToolsConfig::default();
    assert_eq!(config.profile, "coding");
    assert!(!config.allow_sudo);
    assert!(!config.allow_path_outside_workspace);

    let policy = ToolsPolicy::from_config(&config).expect("default tools policy");
    for allowed in [
        "skill:run_cmd",
        "skill:fs_basic",
        "skill:git_basic",
        "skill:task_control",
    ] {
        assert!(policy.is_allowed(allowed, None), "{allowed}");
    }
    for denied in [
        "skill:x",
        "skill:http_basic",
        "skill:crypto",
        "skill:image_generate",
        "skill:service_control",
        "skill:install_module",
    ] {
        assert!(!policy.is_allowed(denied, None), "{denied}");
    }
}

#[test]
fn full_profile_remains_an_explicit_operator_opt_in() {
    let mut config = ToolsConfig::default();
    config.profile = "full".to_string();
    let policy = ToolsPolicy::from_config(&config).expect("full tools policy");
    assert!(policy.is_allowed("skill:x", None));
    assert!(policy.is_allowed("skill:service_control", None));
}
