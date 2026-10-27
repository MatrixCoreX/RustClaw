use std::collections::HashSet;
use std::sync::{Arc, RwLock};

use claw_core::config::ToolsConfig;
use claw_core::skill_registry::SkillsRegistry;

use serde_json::json;

use super::templates::{value_contains_unresolved_template, TemplatePlaceholderScope};
use super::{verify_plan, VerifyInput, VerifyIssueKind, VerifyMode};
use crate::{evidence_policy::FailureAttribution, PlanStep, SkillViewsSnapshot, ToolsPolicy};

#[path = "verifier_tests/fixtures.rs"]
mod fixtures;

pub(super) use fixtures::{
    plan_result, route_result, route_result_with_contract, test_state, test_task,
};

#[path = "verifier_tests/current_request_evidence.rs"]
mod current_request_evidence;

#[test]
fn checkpoint_action_plan_preserves_planner_output_contract() {
    let state = test_state();
    let task = test_task();
    let mut stored_contract = crate::IntentOutputContract::default();
    stored_contract.selection.structured_field_selector =
        Some("command,created_path,stdout,status".to_string());
    let plan = crate::agent_engine::checkpoint_action_plan(
        "system.run_command",
        "system.run_command",
        json!({"command": "printf checkpoint_ok"}),
        1,
        Some(stored_contract),
        Vec::new(),
    );

    let output_contract = plan
        .output_contract
        .as_ref()
        .expect("checkpoint output contract");
    assert_eq!(
        output_contract
            .selection
            .structured_field_selector
            .as_deref(),
        Some("command,created_path,stdout,status")
    );
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(output_contract),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    let action = result.approved_steps[0]
        .to_agent_action()
        .expect("resolved replay action");
    assert!(matches!(
        action,
        crate::AgentAction::CallSkill { ref skill, .. } if skill == "run_cmd"
    ));
}

fn redacted_workspace_child_boundary_context() -> String {
    let observation = json!({
        "kind": "agent_loop_boundary_observations",
        "schema_version": 1,
        "current_request_locator": {
            "source": "current_request",
            "has_concrete_surface": false,
            "explicit_locator_hints": [],
            "resolved_workspace_child": "",
            "resolved_workspace_child_redacted": true
        },
        "pre_loop_clarify_candidates": ["unbound_targeted_evidence"]
    });
    format!(
        "### AGENT_LOOP_BOUNDARY_OBSERVATIONS\n{}\n### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS",
        serde_json::to_string(&observation).expect("observation json")
    )
}

#[test]
fn output_contract_does_not_reintroduce_pre_planner_clarify_state() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: Some("need more info"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "read_file".to_string(),
                args: json!({ "path": "README.md" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.approved);
    assert!(result.blocked_reason.is_none());
    assert!(result
        .issues
        .iter()
        .all(|issue| !matches!(issue.kind, VerifyIssueKind::BoundaryClarifyRequired)));
    assert!(result.shadow_blocked_reason.is_none());
}

#[test]
fn redacted_workspace_child_boundary_blocks_path_content_read_plan() {
    let state = test_state();
    let task = test_task();
    let context = redacted_workspace_child_boundary_context();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("read that README"),
            context_bundle_summary: Some(&context),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({ "action": "read_text_range", "path": "/etc/agent-runtime-verifier-outside/README.md" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result.issues.iter().any(|issue| matches!(
        issue.kind,
        VerifyIssueKind::BoundaryClarifyRequired
    ) && issue
        .detail
        .contains("resolved_workspace_child_redacted")));
    assert!(result.shadow_blocked_reason.is_some());
}

#[test]
fn redacted_workspace_child_boundary_allows_workspace_relative_content_read_plan() {
    let state = test_state();
    let task = test_task();
    let context = redacted_workspace_child_boundary_context();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("summarize workspace README"),
            context_bundle_summary: Some(&context),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({ "action": "read_text_range", "path": "README.md" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result
        .issues
        .iter()
        .all(|issue| !matches!(issue.kind, VerifyIssueKind::BoundaryClarifyRequired)));
    assert!(result.shadow_blocked_reason.is_none());
}

#[test]
fn redacted_workspace_child_boundary_allows_workspace_absolute_content_read_plan() {
    let state = test_state();
    let task = test_task();
    let context = redacted_workspace_child_boundary_context();
    let path = state.skill_rt.workspace_root.join("README.md");
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("summarize workspace README"),
            context_bundle_summary: Some(&context),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({ "action": "read_text_range", "path": path.display().to_string() }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result
        .issues
        .iter()
        .all(|issue| !matches!(issue.kind, VerifyIssueKind::BoundaryClarifyRequired)));
    assert!(result.shadow_blocked_reason.is_none());
}

#[test]
fn redacted_workspace_child_boundary_in_plan_goal_blocks_path_content_read_plan() {
    let state = test_state();
    let task = test_task();
    let context = redacted_workspace_child_boundary_context();
    let mut plan = plan_result(vec![PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_tool".to_string(),
        skill: "fs_basic".to_string(),
        args: json!({ "action": "read_text_range", "path": "/etc/agent-runtime-verifier-outside/README.md" }),
        depends_on: Vec::new(),
        why: String::new(),
    }]);
    plan.goal = format!("test\n\n{context}");
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("read that README"),
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result.issues.iter().any(|issue| matches!(
        issue.kind,
        VerifyIssueKind::BoundaryClarifyRequired
    ) && issue
        .detail
        .contains("resolved_workspace_child_redacted")));
    assert!(result.shadow_blocked_reason.is_some());
}

#[test]
fn locatorless_runtime_status_plan_does_not_trip_route_clarify_block() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result();
    route.requires_content_evidence = true;
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.locator_kind = crate::OutputLocatorKind::None;

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: Some("runtime status scalar"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "system_basic".to_string(),
                args: json!({ "action": "runtime_status", "kind": "current_user" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result
        .issues
        .iter()
        .all(|issue| !matches!(issue.kind, VerifyIssueKind::BoundaryClarifyRequired)));
}

#[test]
fn observe_mode_rewrites_unresolved_template_args_to_response() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("帮我转成表格"),
            context_bundle_summary: Some("needs concrete JSON array"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "read_file".to_string(),
                args: json!({ "path": "{{last_output}}" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.approved);
    assert!(result.shadow_blocked_reason.is_some());
    assert!(result
        .issues
        .iter()
        .all(|issue| !matches!(issue.kind, VerifyIssueKind::BoundaryClarifyRequired)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg)));
    assert_eq!(result.rewritten_steps.len(), 1);
    assert_eq!(result.rewritten_steps[0].action_type, "respond");
    let content = result.rewritten_steps[0]
        .args
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(content).expect("machine payload");
    assert_eq!(
        payload
            .get("reason_code")
            .and_then(serde_json::Value::as_str),
        Some("verify_unresolved_template_arg")
    );
    assert_eq!(
        payload
            .get("message_key")
            .and_then(serde_json::Value::as_str),
        Some("clawd.msg.verify.unresolved_template_arg")
    );
}

#[test]
fn serialized_json_with_adjacent_closing_braces_is_not_a_template() {
    let args = json!({
        "intent_json": "{\"kind\":\"create\",\"task\":{\"kind\":\"run_skill\",\"payload\":{\"args\":{\"scheduled_run\":true}}}}"
    });

    assert!(!value_contains_unresolved_template(
        &args,
        &TemplatePlaceholderScope::default()
    ));
}

#[test]
fn observe_mode_rewrites_unresolved_call_capability_to_response() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some("帮我查一下"),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_capability".to_string(),
                skill: "unknown.example".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(result.shadow_blocked_reason.is_some());
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::CapabilityUnavailable)));
    assert_eq!(result.rewritten_steps.len(), 1);
    assert_eq!(result.rewritten_steps[0].action_type, "respond");
    let content = result.rewritten_steps[0]
        .args
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let payload: serde_json::Value = serde_json::from_str(content).expect("machine payload");
    assert_eq!(
        payload
            .get("reason_code")
            .and_then(serde_json::Value::as_str),
        Some("verify_capability_unavailable")
    );
    assert_eq!(
        payload
            .get("capability")
            .and_then(serde_json::Value::as_str),
        Some("unknown.example")
    );
}

#[test]
fn enforce_mode_blocks_unresolved_call_capability() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_capability".to_string(),
                skill: "unknown.example".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(
        !result.approved,
        "issues: {:?}; permission_decision: {}",
        result.issues, result.permission_decision
    );
    assert!(result.blocked_reason.is_some());
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::CapabilityUnavailable)));
}

#[test]
fn observe_mode_allows_prior_output_template_in_later_args() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: Some(
                "查看 logs 目录，把里面的日志文件名整理到 logs_inventory.txt，然后把文件发给我。",
            ),
            context_bundle_summary: Some("auto_locator_path=/home/guagua/agent-runtime/logs"),
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "step_1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "list_dir".to_string(),
                    args: json!({ "path": "/home/guagua/agent-runtime/logs" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "step_2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "write_file".to_string(),
                    args: json!({
                        "path": "/home/guagua/agent-runtime/logs_inventory.txt",
                        "content": "{{last_output}}"
                    }),
                    depends_on: vec!["step_1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::UnresolvedTemplateArg)));
    assert!(result.rewritten_steps.is_empty());
}

#[test]
fn enforce_mode_blocks_missing_required_arg() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "read_file".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(
        !result.approved,
        "issues: {:?}; permission_decision: {}",
        result.issues, result.permission_decision
    );
    assert!(matches!(
        result.issues.first().map(|issue| issue.kind),
        Some(VerifyIssueKind::MissingRequiredArg)
    ));
    assert!(result
        .blocked_reason
        .as_deref()
        .unwrap_or_default()
        .contains("missing required arg"));
}

#[test]
fn enforce_mode_accepts_required_null_when_the_schema_allows_null() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "nullable_skill".to_string(),
                args: json!({"expected_sha": null}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| issue.kind == VerifyIssueKind::MissingRequiredArg));
}

#[test]
fn enforce_mode_blocks_action_scoped_required_arg() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
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
                args: json!({"action": "read_text_range"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| matches!(
        issue.kind,
        VerifyIssueKind::MissingRequiredArg
    ) && issue.detail.contains("`path`")));
}

#[test]
fn enforce_mode_accepts_action_scoped_alternative_arg() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
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
                args: json!({"action": "stat_paths", "path": "README.md"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::MissingRequiredArg)));
}

#[test]
fn enforce_mode_requires_complete_conjunctive_alternative() {
    let state = test_state();
    let task = test_task();
    let verify = |args| {
        verify_plan(
            &state,
            &task,
            VerifyInput {
                output_contract: Some(&route_result()),
                request_text: None,
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "weather".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "weather".to_string(),
                    args,
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
            },
            VerifyMode::Enforce,
        )
    };

    let latitude_only = verify(json!({"action": "query", "latitude": 31.2}));
    assert!(!latitude_only.approved);
    assert!(latitude_only
        .issues
        .iter()
        .any(|issue| issue.kind == VerifyIssueKind::MissingRequiredArg));

    let coordinates = verify(json!({
        "action": "query",
        "latitude": 31.2,
        "longitude": 121.5
    }));
    assert!(coordinates.approved, "issues: {:?}", coordinates.issues);

    let city = verify(json!({"action": "query", "city": "Shanghai"}));
    assert!(city.approved, "issues: {:?}", city.issues);
}

#[test]
fn workspace_creation_uses_registry_policy_without_route_risk_ceiling() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "write_file".to_string(),
                args: json!({"path": "out.txt", "content": "hello"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result
        .issues
        .iter()
        .all(|issue| !matches!(issue.kind, VerifyIssueKind::RiskBudgetExceeded)));
    assert_eq!(
        result
            .permission_decision
            .pointer("/owner_layer")
            .and_then(serde_json::Value::as_str),
        Some("plan_verifier")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/decision")
            .and_then(serde_json::Value::as_str),
        Some("allow")
    );
    assert_ne!(
        result
            .permission_decision
            .pointer("/status_code")
            .and_then(serde_json::Value::as_str),
        Some("risk_budget_exceeded")
    );
    assert_eq!(
        result
            .permission_decision
            .pointer("/steps/0/risk_level")
            .and_then(serde_json::Value::as_str),
        Some("high")
    );
}

#[test]
fn external_fs_basic_mutation_requires_registry_confirmation() {
    let state = test_state();
    let task = test_task();
    let route = route_result();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({
                    "action": "write_text",
                    "path": "/etc/agent-runtime-verifier-outside.txt",
                    "content": "outside"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(
        result.approved && result.needs_confirmation,
        "issues: {:?}; permission_decision: {}",
        result.issues,
        result.permission_decision
    );
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn observe_mode_does_not_reject_actions_from_semantic_matrix_only() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_contract(false);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({"command": "ls"}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(result.shadow_blocked_reason.is_none());
}

#[test]
fn observe_mode_allows_user_named_output_path_marker_without_contract_rejection() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_contract(true);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::None;
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "write_file".to_string(),
                args: json!({
                    "path": "pwd_line_abs.txt",
                    "content": "/home/guagua/agent-runtime\n",
                    "_clawd_user_named_output_path": true
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
}

#[test]
fn summary_contract_allows_registry_observe_config_preview_without_confirmation() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_contract(false);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "config_edit".to_string(),
                args: json!({
                    "action": "plan_config_change",
                    "path": "configs/config.toml",
                    "field_path": "llm.selected_vendor",
                    "value": "minimax"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.needs_confirmation, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn summary_contract_does_not_reject_registry_config_apply_by_semantic_matrix() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_contract(false);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "config_edit".to_string(),
                args: json!({
                    "action": "apply_config_change",
                    "path": "configs/config.toml",
                    "field_path": "llm.selected_vendor",
                    "value": "minimax"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn verifier_issue_failure_attribution_groups_contract_policy_kinds() {
    assert_eq!(
        VerifyIssueKind::ContractActionRejected.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::ContractMissing.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::ContractPolicyViolation.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::ContractPreferredActionAvailable.failure_attribution(),
        FailureAttribution::ContractGap
    );
    assert_eq!(
        VerifyIssueKind::MissingRequiredArg.failure_attribution(),
        FailureAttribution::ModelError
    );
    assert_eq!(
        VerifyIssueKind::CapabilityUnavailable.failure_attribution(),
        FailureAttribution::ToolGap
    );
    assert_eq!(
        VerifyIssueKind::RiskBudgetExceeded.failure_attribution(),
        FailureAttribution::PermissionDenied
    );
}

#[test]
fn verifier_issue_kinds_expose_stable_machine_fields() {
    let kinds = [
        VerifyIssueKind::SkillNotVisible,
        VerifyIssueKind::CapabilityUnavailable,
        VerifyIssueKind::MissingRequiredArg,
        VerifyIssueKind::DefaultCreationTargetApplied,
        VerifyIssueKind::UnresolvedTemplateArg,
        VerifyIssueKind::InvalidDependsOn,
        VerifyIssueKind::ConfirmationRequired,
        VerifyIssueKind::SandboxPolicyDenied,
        VerifyIssueKind::RiskBudgetExceeded,
        VerifyIssueKind::BoundaryClarifyRequired,
        VerifyIssueKind::RecipeInspectBeforeMutateRequired,
        VerifyIssueKind::RecipeValidationAfterMutateRequired,
        VerifyIssueKind::RecipeTargetScopeRequired,
        VerifyIssueKind::ContractActionRejected,
        VerifyIssueKind::ContractMissing,
        VerifyIssueKind::ContractPolicyViolation,
        VerifyIssueKind::ContractPreferredActionAvailable,
    ];

    for kind in kinds {
        assert!(!kind.reason_code().is_empty(), "{kind:?} reason_code");
        assert!(
            kind.reason_code().starts_with("verify_"),
            "{kind:?} reason_code prefix"
        );
        assert!(!kind.status_code().is_empty(), "{kind:?} status_code");
        assert!(
            kind.message_key().starts_with("clawd.verify."),
            "{kind:?} message_key namespace"
        );
        assert!(
            !kind.failure_attribution().as_str().is_empty(),
            "{kind:?} failure attribution"
        );
    }
}

#[test]
fn observe_mode_no_longer_records_semantic_matrix_preferred_action_hint() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_contract(false);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "stat_paths", "path": "."}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result.issues.iter().any(|issue| matches!(
        issue.kind,
        VerifyIssueKind::ContractPreferredActionAvailable
    )));
    assert!(result.blocked_reason.is_none());
}

#[test]
fn exact_scalar_path_does_not_insert_domain_write_repair() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_contract(false);
    route.response_shape = crate::OutputResponseShape::Scalar;
    route.selection.structured_field_selector = Some("path".to_string());
    route.delivery_required = false;
    route.locator_hint = "pwd_line_abs.txt".to_string();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({"command": "pwd"}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "synthesize_answer".to_string(),
                    skill: "synthesize_answer".to_string(),
                    args: json!({"evidence_refs": ["last_output"]}),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert_eq!(result.approved_steps.len(), 2);
    assert!(
        !result.approved_steps.iter().any(|step| {
            step.skill == "fs_basic"
                && step.args.get("action").and_then(|value| value.as_str()) == Some("write_text")
        }),
        "runtime must not invent a domain write step for a generic machine contract"
    );
}

#[test]
fn enforce_mode_blocks_skill_switch_disabled_even_when_contract_allows_action() {
    let mut state = test_state();
    let registry = state
        .get_skills_registry()
        .expect("test registry should be installed");
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(SkillViewsSnapshot {
        binding: Default::default(),
        registry: Some(registry),
        skills_list: Arc::new(
            ["read_file", "run_cmd", "list_dir"]
                .into_iter()
                .map(str::to_string)
                .collect::<HashSet<_>>(),
        ),
    })));
    let task = test_task();
    let route = route_result_with_contract(false);
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "fs_basic".to_string(),
                args: json!({"action": "list_dir", "path": "."}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result.approved, "issues: {:?}", result.issues);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::SkillNotVisible)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(result
        .blocked_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("not in planner visible skills")));
}

#[test]
fn enforce_mode_allows_low_risk_action_under_low_risk_ceiling() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
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
                args: json!({"action": "stat_paths", "paths": ["README.md"]}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::RiskBudgetExceeded)));
}

#[test]
fn enforce_mode_blocks_skill_not_visible() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "totally_fake_skill".to_string(),
                args: json!({}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(!result.approved);
    assert!(result
        .issues
        .iter()
        .any(|issue| { matches!(issue.kind, VerifyIssueKind::SkillNotVisible) }));
}

#[test]
fn enforce_mode_accepts_runtime_native_active_turn_control() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "control_active_turn".to_string(),
                args: json!({"action": "pause", "expected_instruction_revision": 2}),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::SkillNotVisible)));
}

#[test]
fn enforce_mode_allows_internal_subagent_tool_visibility() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_tool".to_string(),
                skill: "subagent".to_string(),
                args: json!({
                    "role": "review",
                    "objective": "runtime_boundary_alignment_audit",
                    "context_refs": ["AGENTS.md"],
                    "budget": {"max_rounds": 1, "max_tool_calls": 3}
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(!result
        .issues
        .iter()
        .any(|issue| { matches!(issue.kind, VerifyIssueKind::SkillNotVisible) }));
}

#[test]
fn deterministic_subagent_boundary_plan_bypasses_misclassified_contract_rejection() {
    let state = test_state();
    let task = test_task();
    let route = route_result_with_contract(false);
    let mut plan = plan_result(vec![
        PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "subagent".to_string(),
            args: json!({
                "role": "review",
                "objective": "runtime_boundary_alignment_audit",
                "context_refs": ["AGENTS.md", "plan/current.md"],
                "budget": {"max_rounds": 1, "max_tool_calls": 3}
            }),
            depends_on: Vec::new(),
            why: String::new(),
        },
        PlanStep {
            step_id: "s2".to_string(),
            action_type: "call_tool".to_string(),
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "AGENTS.md",
                "start_line": 1,
                "end_line": 260
            }),
            depends_on: Vec::new(),
            why: String::new(),
        },
        PlanStep {
            step_id: "s3".to_string(),
            action_type: "call_tool".to_string(),
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "plan/current.md",
                "start_line": 1,
                "end_line": 260
            }),
            depends_on: Vec::new(),
            why: String::new(),
        },
        PlanStep {
            step_id: "s4".to_string(),
            action_type: "respond".to_string(),
            skill: "respond".to_string(),
            args: json!({
                "content": "{\"output_format\":\"machine_json\",\"owner_layer\":\"subagent_boundary_review\",\"boundary\":{\"write_enabled\":false,\"external_publish_enabled\":false}}"
            }),
            depends_on: vec!["s1".to_string(), "s2".to_string(), "s3".to_string()],
            why: String::new(),
        },
    ]);
    plan.raw_plan_text = "deterministic:subagent_review_boundary_surface".to_string();

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractActionRejected)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ContractPolicyViolation)));
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::SkillNotVisible)));
}

#[test]
fn deterministic_subagent_boundary_plan_defers_clarify_when_locator_is_structured() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_contract(false);
    route.requires_content_evidence = true;
    route.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;

    let mut plan = plan_result(vec![
        PlanStep {
            step_id: "s1".to_string(),
            action_type: "call_tool".to_string(),
            skill: "subagent".to_string(),
            args: json!({
                "role": "review",
                "objective": "runtime_boundary_alignment_audit",
                "context_refs": ["AGENTS.md", "plan/current.md"],
                "budget": {"max_rounds": 1, "max_tool_calls": 3}
            }),
            depends_on: Vec::new(),
            why: String::new(),
        },
        PlanStep {
            step_id: "s2".to_string(),
            action_type: "call_tool".to_string(),
            skill: "fs_basic".to_string(),
            args: json!({
                "action": "read_text_range",
                "path": "AGENTS.md",
                "start_line": 1,
                "end_line": 260
            }),
            depends_on: Vec::new(),
            why: String::new(),
        },
    ]);
    plan.raw_plan_text = "deterministic:subagent_review_boundary_surface".to_string();

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route),
            request_text: None,
            context_bundle_summary: Some("structured_current_plan_locator"),
            plan_result: &plan,
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::BoundaryClarifyRequired)));
}

#[test]
fn verifier_allows_independent_steps_from_same_registry_group() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "primary_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "fallback_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
}

#[test]
fn verifier_allows_repeated_steps_from_same_registry_group_skill() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "primary_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "primary_reader".to_string(),
                    args: json!({}),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
}

#[test]
fn legacy_resume_route_does_not_bypass_confirmation_policy() {
    let state = test_state();
    let task = test_task();
    let resumed_route = route_result();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&resumed_route),
            request_text: None,
            context_bundle_summary: Some("resume"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({ "command": "pwd" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved);
    assert!(result.needs_confirmation);
    assert!(result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[test]
fn confirmation_exempt_invocation_skips_confirmation_requirement() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: Some(&route_result()),
            request_text: None,
            context_bundle_summary: Some("photo preview"),
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "photo_organize".to_string(),
                args: json!({ "action": "organize", "mode": "plan" }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved);
    assert!(!result.needs_confirmation);
    assert!(!result
        .issues
        .iter()
        .any(|issue| matches!(issue.kind, VerifyIssueKind::ConfirmationRequired)));
}

#[path = "verifier_tests/workspace_primitives.rs"]
mod workspace_primitives;

#[path = "verifier_tests/dangerous_defaults.rs"]
mod dangerous_defaults;

#[path = "verifier_tests/code_change_validation.rs"]
mod code_change_validation;

#[path = "verifier_tests/confirmation_policy.rs"]
mod confirmation_policy;

#[path = "verifier_tests/scope_boundaries.rs"]
mod scope_boundaries;

#[path = "verifier_tests/schema_types.rs"]
mod schema_types;

#[path = "verifier_tests/scope_persistence.rs"]
mod scope_persistence;

#[path = "verifier_tests/ops_recipe_repair.rs"]
mod ops_recipe_repair;

#[path = "verifier_tests/media_artifact.rs"]
mod media_artifact;

#[path = "verifier_tests/registry_confirmation.rs"]
mod registry_confirmation;

#[path = "verifier_tests/agent_loop_execution_contract.rs"]
mod agent_loop_execution_contract;

#[path = "verifier_tests/direct_capability.rs"]
mod direct_capability;

#[path = "verifier_tests/config_read_contract.rs"]
mod config_read_contract;

#[path = "verifier_tests/admin_authority.rs"]
mod admin_authority;

#[path = "verifier_tests/session_alias.rs"]
mod session_alias;
