use serde_json::json;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, RwLock};

use super::*;

fn state() -> AppState {
    let registry_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/skills_registry.toml")
        .canonicalize()
        .expect("canonicalize registry path");
    let registry = claw_core::skill_registry::SkillsRegistry::load_from_path(&registry_path)
        .expect("load registry");
    let enabled: HashSet<String> = registry.enabled_names().into_iter().collect();
    let mut state = AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(crate::SkillViewsSnapshot {
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(enabled),
    })));
    let mut tools = claw_core::config::ToolsConfig::default();
    tools.allow = vec!["skill:fs_basic".to_string(), "skill:run_cmd".to_string()];
    state.skill_rt.tools_policy = Arc::new(crate::ToolsPolicy::from_config(&tools).unwrap());
    state
}

fn task() -> ClaimedTask {
    ClaimedTask {
        claim_attempt: 0,
        task_id: uuid::Uuid::new_v4().to_string(),
        user_id: 1,
        chat_id: 2,
        user_key: Some("user:direct-skill-permission".to_string()),
        channel: "cli".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "run_skill".to_string(),
        payload_json: "{}".to_string(),
    }
}

#[test]
fn direct_read_only_skill_uses_verifier_without_confirmation() {
    let state = state();
    let verification = verify_direct_run_skill(
        &state,
        &task(),
        "fs_basic",
        json!({"action": "list_dir", "path": "."}),
    );

    assert!(verification.allowed(), "{:?}", verification.verify.issues);
    assert_eq!(verification.verify.permission_decision["decision"], "allow");
    assert_eq!(
        verification.verify.permission_decision["steps"][0]["decision"],
        "allow"
    );
}

#[test]
fn direct_high_risk_skill_stops_at_one_shot_approval_boundary() {
    let state = state();
    let verification = verify_direct_run_skill(
        &state,
        &task(),
        "run_cmd",
        json!({"command": "rm -rf target/direct-skill-permission-fixture"}),
    );

    assert!(verification.needs_confirmation());
    assert_eq!(
        verification.verify.permission_decision["decision"],
        "require_confirmation"
    );
    assert_eq!(
        verification.verify.permission_decision["steps"][0]["decision"],
        "require_confirmation"
    );
    assert_eq!(
        verification.verify.permission_decision["steps"][0]["sandbox"]["external_publish"],
        true
    );
    assert_eq!(
        verification.verify.permission_decision["steps"][0]["sandbox"]["credential_access"],
        true
    );
    assert_eq!(
        verification.verify.permission_decision["steps"][0]["registry_policy"]["capability"],
        "system.run_command"
    );
}
