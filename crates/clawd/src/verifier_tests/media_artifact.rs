use super::*;

#[test]
fn generated_file_path_report_does_not_repair_media_artifact_output_with_text_write() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_semantic(crate::OutputSemanticKind::GeneratedFilePathReport);
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_hint = "document/skill_audio_smoke.mp3".to_string();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "audio_synthesize".to_string(),
                    args: json!({
                        "text": "RustClaw skill test passed",
                        "output_path": "document/skill_audio_smoke.mp3"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: json!({"content": "document/skill_audio_smoke.mp3"}),
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
    assert!(!result.approved_steps.iter().any(|step| {
        step.skill == "fs_basic"
            && step.args.get("action").and_then(|value| value.as_str()) == Some("write_text")
    }));
}

#[test]
fn media_generate_dry_run_does_not_exceed_medium_risk_ceiling() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_semantic(crate::OutputSemanticKind::GeneratedFilePathReport);
    route.risk_ceiling = crate::RiskCeiling::Medium;
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "image_generate".to_string(),
                args: json!({
                    "action": "generate",
                    "prompt": "status card",
                    "output_path": "document/media_dry_run/image_status_card.png",
                    "dry_run": true
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(
        !result
            .issues
            .iter()
            .any(|issue| matches!(issue.kind, VerifyIssueKind::RiskBudgetExceeded)),
        "issues: {:?}",
        result.issues
    );
}

#[test]
fn generated_file_path_report_does_not_write_stat_json_over_media_path() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result_with_semantic(crate::OutputSemanticKind::GeneratedFilePathReport);
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.delivery_required = false;
    route.output_contract.locator_hint = "document/rust_icon_pixel_smoke.png".to_string();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "python3 -c 'create image'",
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_tool".to_string(),
                    skill: "fs_basic".to_string(),
                    args: json!({
                        "action": "stat_paths",
                        "paths": ["document/rust_icon_pixel_smoke.png"],
                    }),
                    depends_on: vec!["s1".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s3".to_string(),
                    action_type: "respond".to_string(),
                    skill: "respond".to_string(),
                    args: json!({"content": "document/rust_icon_pixel_smoke.png"}),
                    depends_on: vec!["s2".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::ObserveOnly,
    );

    assert!(result.approved, "issues: {:?}", result.issues);
    assert_eq!(result.approved_steps.len(), 3);
    assert!(!result.approved_steps.iter().any(|step| {
        step.skill == "fs_basic"
            && step.args.get("action").and_then(|value| value.as_str()) == Some("write_text")
    }));
}
