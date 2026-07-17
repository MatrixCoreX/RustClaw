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
