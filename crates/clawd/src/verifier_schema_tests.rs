use serde_json::json;

use super::{tests, verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
use crate::PlanStep;

#[test]
fn invalid_registry_enum_is_a_blocking_model_error() {
    let state = tests::test_state();
    let task = tests::test_task();
    let plan = tests::plan_result(vec![PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "fs_basic".to_string(),
        args: json!({
            "action": "remove_entries",
            "path": "tmp/example"
        }),
        depends_on: Vec::new(),
        why: String::new(),
    }]);

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    let issue = result
        .issues
        .iter()
        .find(|issue| issue.kind == VerifyIssueKind::InvalidArgumentValue)
        .expect("invalid enum issue");
    assert_eq!(
        issue.kind.failure_attribution(),
        crate::evidence_policy::FailureAttribution::ModelError
    );
    assert_eq!(
        issue.detail,
        "error_code=invalid_argument_value field=action constraint=enum"
    );
}

#[test]
fn undeclared_registry_argument_is_a_blocking_model_error() {
    let state = tests::test_state();
    let task = tests::test_task();
    let plan = tests::plan_result(vec![PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "fs_basic".to_string(),
        args: json!({
            "action": "stat_paths",
            "path": "fixture",
            "duration_seconds": 3
        }),
        depends_on: Vec::new(),
        why: String::new(),
    }]);

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    let issue = result
        .issues
        .iter()
        .find(|issue| {
            issue.kind == VerifyIssueKind::InvalidArgumentValue
                && issue.detail.contains("field=duration_seconds")
        })
        .expect("unknown argument issue");
    assert_eq!(
        issue.kind.failure_attribution(),
        crate::evidence_policy::FailureAttribution::ModelError
    );
    assert_eq!(
        issue.detail,
        "error_code=invalid_argument_value field=duration_seconds constraint=declared_property"
    );
}

#[test]
fn virtual_count_entries_runtime_arguments_are_declared() {
    let state = tests::test_state();
    let task = tests::test_task();
    let plan = tests::plan_result(vec![PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "fs_basic".to_string(),
        args: json!({
            "action": "count_entries",
            "path": "scripts/nl_tests/fixtures/device_local",
            "files_only": true,
            "dirs_only": false,
            "include_hidden": false,
            "ext_filter": ".json"
        }),
        depends_on: Vec::new(),
        why: String::new(),
    }]);

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    let invalid_fields = result
        .issues
        .iter()
        .filter(|issue| issue.kind == VerifyIssueKind::InvalidArgumentValue)
        .map(|issue| issue.detail.as_str())
        .collect::<Vec<_>>();
    assert!(
        invalid_fields.is_empty(),
        "runtime rewrite arguments must remain valid: {invalid_fields:?}"
    );
}

#[test]
fn runtime_status_requires_declared_machine_kind() {
    let state = tests::test_state();
    let task = tests::test_task();
    let plan = tests::plan_result(vec![PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "system_basic".to_string(),
        args: json!({ "action": "runtime_status" }),
        depends_on: Vec::new(),
        why: String::new(),
    }]);

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::MissingRequiredArg
            && issue.missing_fields == ["kind".to_string()]
    }));
}

#[test]
fn runtime_status_rejects_kind_outside_registry_enum() {
    let state = tests::test_state();
    let task = tests::test_task();
    let plan = tests::plan_result(vec![PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "system_basic".to_string(),
        args: json!({ "action": "runtime_status", "kind": "runtime_status" }),
        depends_on: Vec::new(),
        why: String::new(),
    }]);

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::InvalidArgumentValue
            && issue.detail == "error_code=invalid_argument_value field=kind constraint=enum"
    }));
}
