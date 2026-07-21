use claw_core::config::{ToolApprovalPolicy, ToolSandboxMode, ToolsConfig};

use super::policy::{SandboxRequirements, ToolsPolicy};

#[test]
fn default_coding_profile_is_an_explicit_local_capability_set() {
    let config = ToolsConfig::default();
    assert_eq!(config.access_profile, "coding");
    assert_eq!(config.sandbox_mode.as_token(), "workspace_write");
    assert_eq!(config.sandbox_backend.as_token(), "auto");
    assert_eq!(config.approval_policy.as_token(), "on_risk");
    assert!(!config.allow_sudo);
    assert!(!config.allow_path_outside_workspace);

    let policy = ToolsPolicy::from_config(&config).expect("default tools policy");
    for allowed in [
        "skill:run_cmd",
        "skill:code_index",
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
    assert!(policy.is_allowed("capability:image.preview_generate", None));
    assert!(policy.is_any_allowed(
        &["skill:image_generate", "capability:image.preview_generate"],
        None
    ));
    for (skill, capability) in [
        ("audio_synthesize", "audio.preview_synthesize"),
        ("video_generate", "video.preview_generate"),
        ("music_generate", "music.preview_generate"),
    ] {
        let skill_token = format!("skill:{skill}");
        let capability_token = format!("capability:{capability}");
        assert!(policy.is_allowed(&capability_token, None));
        assert!(policy.is_any_allowed(&[&skill_token, &capability_token], None));
        assert!(!policy.is_allowed(&skill_token, None));
    }
    for capability in ["schedule.preview", "schedule.list"] {
        let capability_token = format!("capability:{capability}");
        assert!(policy.is_allowed(&capability_token, None));
        assert!(policy.is_any_allowed(&["skill:schedule", &capability_token], None));
    }
    assert!(!policy.is_allowed("skill:schedule", None));
    assert!(policy.is_allowed("capability:service_control", None));
    assert!(policy.is_any_allowed(
        &["skill:service_control", "capability:service_control"],
        None
    ));
    for capability in [
        "service.logs",
        "service.start",
        "service.stop",
        "service.restart",
        "service_control.start",
        "service_control.stop",
        "service_control.restart",
    ] {
        assert!(!policy.is_allowed(&format!("capability:{capability}"), None));
    }
    for capability in [
        "schedule.create",
        "schedule.delete",
        "schedule.pause",
        "schedule.resume",
    ] {
        assert!(!policy.is_allowed(&format!("capability:{capability}"), None));
    }
}

#[test]
fn explicit_skill_deny_overrides_action_capability_allow() {
    let mut config = ToolsConfig::default();
    config.deny = vec!["skill:image_generate".to_string()];
    let policy = ToolsPolicy::from_config(&config).expect("tools policy");

    assert!(!policy.is_any_allowed(
        &["skill:image_generate", "capability:image.preview_generate"],
        None
    ));
}

#[test]
fn explicit_schedule_skill_deny_overrides_observe_capability_allow() {
    let mut config = ToolsConfig::default();
    config.deny = vec!["skill:schedule".to_string()];
    let policy = ToolsPolicy::from_config(&config).expect("tools policy");

    assert!(!policy.is_any_allowed(&["skill:schedule", "capability:schedule.preview"], None));
}

#[test]
fn full_profile_remains_an_explicit_operator_opt_in() {
    let mut config = ToolsConfig::default();
    config.access_profile = "full".to_string();
    let policy = ToolsPolicy::from_config(&config).expect("full tools policy");
    assert!(policy.is_allowed("skill:x", None));
    assert!(policy.is_allowed("skill:service_control", None));
}

#[test]
fn sandbox_mode_and_approval_policy_are_independent() {
    let mut config = ToolsConfig::default();
    config.sandbox_mode = ToolSandboxMode::ReadOnly;
    config.approval_policy = ToolApprovalPolicy::Always;
    let policy = ToolsPolicy::from_config(&config).expect("tools policy");

    assert_eq!(policy.sandbox_mode_token(), "read_only");
    assert_eq!(policy.sandbox_backend_token(), "auto");
    assert_eq!(policy.approval_policy_token(), "always");
    assert_eq!(
        policy.sandbox_denial(SandboxRequirements {
            mutates: true,
            filesystem_write: true,
            ..SandboxRequirements::default()
        }),
        Some("sandbox_read_only_write_denied")
    );
    assert!(policy.approval_required(false, false, true));
}

#[test]
fn workspace_sandbox_separates_brokered_network_from_subprocess_access() {
    let policy = ToolsPolicy::from_config(&ToolsConfig::default()).expect("tools policy");
    assert_eq!(
        policy.sandbox_denial(SandboxRequirements {
            network_access: true,
            ..SandboxRequirements::default()
        }),
        None
    );
    assert_eq!(
        policy.sandbox_denial(SandboxRequirements {
            network_access: true,
            subprocess: true,
            ..SandboxRequirements::default()
        }),
        Some("sandbox_workspace_external_denied")
    );
    assert_eq!(
        policy.sandbox_denial(SandboxRequirements {
            package_install: true,
            ..SandboxRequirements::default()
        }),
        Some("sandbox_workspace_privilege_denied")
    );
}

#[test]
fn subprocess_requires_declared_isolation_in_restrictive_modes() {
    let mut config = ToolsConfig {
        sandbox_mode: ToolSandboxMode::ReadOnly,
        ..ToolsConfig::default()
    };
    let read_only = ToolsPolicy::from_config(&config).expect("read-only policy");
    assert_eq!(
        read_only.sandbox_denial(SandboxRequirements {
            subprocess: true,
            ..SandboxRequirements::default()
        }),
        Some("sandbox_read_only_subprocess_denied")
    );
    assert_eq!(
        read_only.sandbox_denial(SandboxRequirements {
            subprocess: true,
            isolation_profile: Some("read_only"),
            ..SandboxRequirements::default()
        }),
        None
    );

    config.sandbox_mode = ToolSandboxMode::IsolatedWorktree;
    let worktree = ToolsPolicy::from_config(&config).expect("worktree policy");
    assert_eq!(
        worktree.sandbox_denial(SandboxRequirements {
            subprocess: true,
            isolation_profile: Some("local_current_workspace"),
            ..SandboxRequirements::default()
        }),
        Some("sandbox_worktree_subprocess_isolation_required")
    );
    assert_eq!(
        worktree.sandbox_denial(SandboxRequirements {
            subprocess: true,
            isolation_profile: Some("local_worktree"),
            ..SandboxRequirements::default()
        }),
        None
    );
}
