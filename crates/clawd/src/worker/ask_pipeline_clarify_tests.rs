use std::path::PathBuf;

use super::ordinary_clarify_should_enter_agent_loop;

fn temp_root(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "rustclaw_ask_pipeline_clarify_{label}_{}_{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(path.join("configs")).expect("create config dir");
    path
}

fn state_with_semantic_route_authority(authority: &str) -> crate::AppState {
    let root = temp_root(authority);
    std::fs::write(
        root.join("configs/agent_guard.toml"),
        format!("[agent.loop_guard]\nsemantic_route_authority = \"{authority}\"\n"),
    )
    .expect("write agent guard");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = root;
    state
}

#[test]
fn ordinary_clarify_enters_agent_loop_under_agent_authority() {
    let state = state_with_semantic_route_authority("agent_loop_default");
    assert!(ordinary_clarify_should_enter_agent_loop(
        &state,
        crate::post_route_policy::ClarifyReasonKind::RouteReasonText
    ));
}

#[test]
fn boundary_clarify_stays_on_boundary_renderer_under_agent_authority() {
    let state = state_with_semantic_route_authority("agent_loop_default");
    assert!(!ordinary_clarify_should_enter_agent_loop(
        &state,
        crate::post_route_policy::ClarifyReasonKind::MissingPathScopedLocator
    ));
    assert!(!ordinary_clarify_should_enter_agent_loop(
        &state,
        crate::post_route_policy::ClarifyReasonKind::FuzzyLocatorCandidates
    ));
}

#[test]
fn ordinary_clarify_enters_agent_loop_even_under_legacy_rollback_token() {
    let state = state_with_semantic_route_authority("legacy");
    assert!(ordinary_clarify_should_enter_agent_loop(
        &state,
        crate::post_route_policy::ClarifyReasonKind::RouteReasonText
    ));
}
