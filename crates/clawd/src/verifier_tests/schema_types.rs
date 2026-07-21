use super::*;

fn verify_fs_args(args: serde_json::Value) -> super::super::VerifyResult {
    let state = test_state();
    let task = test_task();
    verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args,
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    )
}

#[test]
fn enforce_mode_rejects_wrong_registry_field_type_before_execution() {
    let result = verify_fs_args(json!({
        "action": "read_text_range",
        "path": {"unexpected": "object"}
    }));

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        matches!(issue.kind, VerifyIssueKind::InvalidArgumentValue)
            && issue.detail.contains("field=path")
            && issue.detail.contains("constraint=type")
            && issue.detail.contains("expected=string")
    }));
}

#[test]
fn enforce_mode_accepts_each_declared_union_type() {
    for ext_filter in [json!("rs"), json!(["rs", "toml"])] {
        let result = verify_fs_args(json!({
            "action": "count_entries",
            "path": ".",
            "ext_filter": ext_filter
        }));

        assert!(result.approved, "issues: {:?}", result.issues);
        assert!(result.issues.iter().all(|issue| {
            !matches!(issue.kind, VerifyIssueKind::InvalidArgumentValue)
                || !issue.detail.contains("field=ext_filter")
        }));
    }
}
