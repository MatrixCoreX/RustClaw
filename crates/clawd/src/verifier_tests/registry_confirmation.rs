use super::*;

fn workspace_registry_state() -> AppState {
    let state = AppState::test_default_with_fixture_provider();
    let registry_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load workspace registry");
    let enabled = registry
        .enabled_names()
        .into_iter()
        .collect::<std::collections::HashSet<_>>();
    *state
        .core
        .skill_views_snapshot
        .write()
        .expect("skill snapshot lock") = Arc::new(SkillViewsSnapshot {
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(enabled),
    });
    state
}

#[test]
fn archive_and_db_readonly_actions_are_confirmation_exempt_from_registry() {
    let state = workspace_registry_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "archive-list".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "archive_basic".to_string(),
                    args: json!({
                        "action": "list",
                        "archive": "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "db-list".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "db_basic".to_string(),
                    args: json!({
                        "action": "list_tables",
                        "path": "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn archive_and_db_mutating_actions_still_require_confirmation() {
    let state = workspace_registry_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result(false).output_contract),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "archive-pack".to_string(),
                action_type: "call_tool".to_string(),
                skill: "archive_basic".to_string(),
                args: json!({
                    "action": "pack",
                    "source": "scripts/nl_tests/fixtures/device_local/docs",
                    "archive": "tmp/out.zip"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));

    assert!(state.skill_invocation_requires_confirmation_policy(
        "db_basic",
        Some(&json!({
            "action": "sqlite_execute",
            "path": "scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite",
            "sql": "delete from demo",
            "confirm": true
        }))
    ));
}
