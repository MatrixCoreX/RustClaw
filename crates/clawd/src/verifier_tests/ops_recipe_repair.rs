use super::*;

#[test]
fn ops_recipe_rewrites_combined_run_cmd_into_apply_then_validate() {
    let state = test_state();
    let task = test_task();
    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route_result(false)),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "cd /tmp/demo && nohup python3 -m http.server 51179 --bind 127.0.0.1 > /dev/null 2>&1 & sleep 2 && curl -s http://127.0.0.1:51179/ | grep -q 'ops-demo-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    inspect_first: false,
                    validation_required: true,
                    max_repairs: 2,
                    ..Default::default()
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert_eq!(result.rewritten_steps.len(), 2);
    assert_eq!(result.rewritten_steps[0].step_id, "s1");
    assert_eq!(result.rewritten_steps[1].step_id, "s1__validate");
    assert_eq!(
            result.rewritten_steps[0].args.get("command").and_then(|v| v.as_str()),
            Some(
                "cd /tmp/demo && nohup python3 -m http.server 51179 --bind 127.0.0.1 > /dev/null 2>&1 &"
            )
        );
    assert_eq!(
            result.rewritten_steps[1].args.get("command").and_then(|v| v.as_str()),
            Some(
                "sleep 2 && curl -s http://127.0.0.1:51179/ | grep -q 'ops-demo-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
            )
        );
    assert_eq!(result.rewritten_steps[1].depends_on, vec!["s1".to_string()]);
    assert!(result.rewritten_steps[0]
        .args
        .get("timeout_seconds")
        .is_none());
    assert!(result.rewritten_steps[1]
        .args
        .get("timeout_seconds")
        .is_none());
}

#[test]
fn ops_recipe_split_does_not_infer_success_marker_from_request_text() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(false);
    route.resolved_intent =
        "start local http service and verify homepage contains ops-demo-ok".to_string();
    let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: Some(
                    "Start a static HTTP server in the background, then use curl to verify that the homepage contains ops-demo-ok; when validation passes, explicitly output VALIDATION_PASSED and finish immediately.",
                ),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "cd /tmp/demo && nohup python3 -m http.server 51179 --bind 127.0.0.1 > /dev/null 2>&1 & sleep 2 && curl -s http://127.0.0.1:51179/"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: false,
                        validation_required: true,
                        max_repairs: 2,
                    ..Default::default()
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
    assert_eq!(result.rewritten_steps.len(), 2);
    assert_eq!(
        result.rewritten_steps[1]
            .args
            .get("command")
            .and_then(|value| value.as_str()),
        Some("sleep 2 && curl -s http://127.0.0.1:51179/")
    );
}

#[test]
fn ops_recipe_does_not_infer_http_expect_contains_marker_from_route_text() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(false);
    route.resolved_intent =
        "verify local http service homepage contains ops-repair-ok and repair if needed"
            .to_string();
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
                skill: "http_basic".to_string(),
                args: json!({
                    "action": "get",
                    "url": "http://127.0.0.1:51179/"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                crate::execution_recipe::ExecutionRecipeSpec {
                    kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                    inspect_first: true,
                    validation_required: true,
                    max_repairs: 2,
                    ..Default::default()
                },
            ),
        },
        VerifyMode::ObserveOnly,
    );
    assert!(result.rewritten_steps.is_empty());
    assert_eq!(result.approved_steps.len(), 1);
    assert!(result.approved_steps[0]
        .args
        .get("expect_contains")
        .is_none());
}

#[test]
fn ops_recipe_does_not_infer_http_expect_contains_marker_from_request_text() {
    let state = test_state();
    let task = test_task();
    let route = route_result(false);
    let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: Some(
                    "First verify whether the local static HTTP service serves a homepage containing ops-repair-ok. If verification fails, repair it and verify again until it passes.",
                ),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![PlanStep {
                    step_id: "s1".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "http_basic".to_string(),
                    args: json!({
                        "action": "get",
                        "url": "http://127.0.0.1:51179/"
                    }),
                    depends_on: Vec::new(),
                    why: String::new(),
                }]),
                execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
                    crate::execution_recipe::ExecutionRecipeSpec {
                        kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
                        inspect_first: true,
                        validation_required: true,
                        max_repairs: 2,
                    ..Default::default()
                    },
                ),
            },
            VerifyMode::ObserveOnly,
        );
    assert!(result.rewritten_steps.is_empty());
    assert_eq!(result.approved_steps.len(), 1);
    assert!(result.approved_steps[0]
        .args
        .get("expect_contains")
        .is_none());
}

#[test]
fn ops_recipe_repair_round_plan_stays_valid_after_failed_http_preflight() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(false);
    route.resolved_intent =
        "verify local http service homepage contains ops-repair-ok and repair if needed"
            .to_string();
    route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
    let initial_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    let inspect_result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s1".to_string(),
                action_type: "call_skill".to_string(),
                skill: "http_basic".to_string(),
                args: json!({
                    "action": "get",
                    "url": "http://127.0.0.1:51179/",
                    "expect_contains": "ops-repair-ok"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: initial_recipe,
        },
        VerifyMode::ObserveOnly,
    );
    let inspect_step = &inspect_result.approved_steps[0];
    let raw_effect = crate::execution_recipe::classify_skill_action_effect(
        &state,
        &inspect_step.skill,
        &inspect_step.args,
    );
    let effective_effect =
        crate::execution_recipe::effective_action_effect_for_recipe(initial_recipe, raw_effect);
    let validation = crate::execution_recipe::assess_validation_output(
        &state,
        &inspect_step.skill,
        &inspect_step.args,
        "status=200\nops-repair-bad\n",
    );
    assert!(matches!(
        validation,
        crate::execution_recipe::ValidationObservation::Failed(_)
    ));
    assert!(effective_effect.observes);
    assert!(!effective_effect.validates);

    let mut repair_recipe = initial_recipe;
    crate::execution_recipe::apply_action_effect_failure(&mut repair_recipe, effective_effect);
    assert_eq!(
        crate::execution_recipe::stop_signal_for_validation_failure(&repair_recipe),
        "recoverable_failure_continue_round"
    );
    assert!(repair_recipe.saw_inspect);
    assert_eq!(
        repair_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Apply
    );

    let repair_result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "read_file".to_string(),
                    args: json!({ "path": "document/nl_ops_http_demo/index.html" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s3".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html"
                    }),
                    depends_on: vec!["s2".to_string()],
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s4".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({
                        "command": "curl -s http://127.0.0.1:51179/ | grep -q 'ops-repair-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
                    }),
                    depends_on: vec!["s3".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: repair_recipe,
        },
        VerifyMode::Enforce,
    );
    assert!(repair_result.approved, "issues: {:?}", repair_result.issues);
    assert!(repair_result.blocked_reason.is_none());
    assert_eq!(repair_result.approved_steps.len(), 3);
    assert!(repair_result.rewritten_steps.is_empty());
    assert_eq!(
            repair_result.approved_steps[2]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some(
                "curl -s http://127.0.0.1:51179/ | grep -q 'ops-repair-ok' && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
            )
        );
}

#[test]
fn ops_recipe_service_repair_round_plan_stays_valid_after_failed_status_preflight() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(false);
    route.resolved_intent = "repair sing-box and verify the service is running".to_string();
    route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
    let initial_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    let inspect_step = PlanStep {
        step_id: "s1".to_string(),
        action_type: "call_skill".to_string(),
        skill: "run_cmd".to_string(),
        args: json!({ "command": "systemctl status sing-box" }),
        depends_on: Vec::new(),
        why: String::new(),
    };
    let inspect_result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![inspect_step.clone()]),
            execution_recipe: initial_recipe,
        },
        VerifyMode::ObserveOnly,
    );
    assert!(inspect_result.approved);
    assert_eq!(inspect_result.approved_steps.len(), 1);
    assert_eq!(
        inspect_result.approved_steps[0].step_id,
        inspect_step.step_id
    );
    assert_eq!(inspect_result.approved_steps[0].skill, inspect_step.skill);
    assert_eq!(
        inspect_result.approved_steps[0]
            .args
            .get("command")
            .and_then(|value| value.as_str()),
        Some("systemctl status sing-box")
    );

    let raw_effect = crate::execution_recipe::classify_skill_action_effect(
        &state,
        "run_cmd",
        &json!({ "command": "systemctl status sing-box" }),
    );
    let effective_effect =
        crate::execution_recipe::effective_action_effect_for_recipe(initial_recipe, raw_effect);
    let validation = crate::execution_recipe::assess_validation_output(
        &state,
        "run_cmd",
        &json!({ "command": "systemctl status sing-box" }),
        "inactive (dead)\n",
    );
    assert!(matches!(
        validation,
        crate::execution_recipe::ValidationObservation::Failed(_)
    ));
    assert!(effective_effect.observes);
    assert!(!effective_effect.validates);

    let mut repair_recipe = initial_recipe;
    crate::execution_recipe::apply_action_effect_failure(&mut repair_recipe, effective_effect);
    assert_eq!(
        crate::execution_recipe::stop_signal_for_validation_failure(&repair_recipe),
        "recoverable_failure_continue_round"
    );
    assert!(repair_recipe.saw_inspect);
    assert_eq!(
        repair_recipe.phase,
        crate::execution_recipe::ExecutionRecipePhase::Apply
    );

    let repair_result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![
                PlanStep {
                    step_id: "s2".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "systemctl restart sing-box" }),
                    depends_on: Vec::new(),
                    why: String::new(),
                },
                PlanStep {
                    step_id: "s3".to_string(),
                    action_type: "call_skill".to_string(),
                    skill: "run_cmd".to_string(),
                    args: json!({ "command": "systemctl is-active sing-box" }),
                    depends_on: vec!["s2".to_string()],
                    why: String::new(),
                },
            ]),
            execution_recipe: repair_recipe,
        },
        VerifyMode::Enforce,
    );
    assert!(repair_result.approved, "issues: {:?}", repair_result.issues);
    assert!(repair_result.blocked_reason.is_none());
    assert_eq!(repair_result.approved_steps.len(), 2);
    assert!(repair_result.rewritten_steps.is_empty());
    assert_eq!(
        repair_result.approved_steps[1]
            .args
            .get("command")
            .and_then(|value| value.as_str()),
        Some("systemctl is-active sing-box")
    );
}

#[test]
fn ops_recipe_repair_round_rewrites_combined_run_cmd_plan() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(false);
    route.resolved_intent =
        "repair local demo file and verify it contains ops-repair-ok".to_string();
    route.resume_behavior = crate::ResumeBehavior::ResumeExecute;
    let mut repair_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    repair_recipe.saw_inspect = true;
    repair_recipe.phase = crate::execution_recipe::ExecutionRecipePhase::Apply;

    let result = verify_plan(
        &state,
        &task,
        VerifyInput {
            route_result: Some(&route),
            request_text: None,
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "s2".to_string(),
                action_type: "call_skill".to_string(),
                skill: "run_cmd".to_string(),
                args: json!({
                    "command": "printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html & sleep 1 && grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
                }),
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: repair_recipe,
        },
        VerifyMode::Enforce,
    );
    assert!(result.approved, "issues: {:?}", result.issues);
    assert!(result.blocked_reason.is_none());
    assert_eq!(result.rewritten_steps.len(), 2);
    assert_eq!(result.rewritten_steps[0].step_id, "s2");
    assert_eq!(result.rewritten_steps[1].step_id, "s2__validate");
    assert_eq!(
        result.rewritten_steps[0]
            .args
            .get("command")
            .and_then(|value| value.as_str()),
        Some("printf 'ops-repair-ok\\n' > document/nl_ops_http_demo/index.html &")
    );
    assert_eq!(
            result.rewritten_steps[1]
                .args
                .get("command")
                .and_then(|value| value.as_str()),
            Some(
                "sleep 1 && grep -q 'ops-repair-ok' document/nl_ops_http_demo/index.html && echo 'VALIDATION_PASSED' || echo 'VALIDATION_FAILED'"
            )
        );
    assert_eq!(result.rewritten_steps[1].depends_on, vec!["s2".to_string()]);
}

#[test]
fn ops_recipe_apply_phase_skips_leading_validation_before_mutation() {
    let state = test_state();
    let task = test_task();
    let mut route = route_result(false);
    route.resolved_intent =
        "验证首页包含 ops-repair-ok，失败就修复 document/nl_ops_http_demo/index.html 后重试"
            .to_string();
    let mut apply_recipe = crate::execution_recipe::ExecutionRecipeRuntimeState::from_spec(
        crate::execution_recipe::ExecutionRecipeSpec {
            kind: crate::execution_recipe::ExecutionRecipeKind::OpsClosedLoop,
            inspect_first: true,
            validation_required: true,
            max_repairs: 2,
            ..Default::default()
        },
    );
    apply_recipe.saw_inspect = true;
    apply_recipe.phase = crate::execution_recipe::ExecutionRecipePhase::Apply;
    let result = verify_plan(
            &state,
            &task,
            VerifyInput {
                route_result: Some(&route),
                request_text: Some(
                    "先验证首页是否包含 ops-repair-ok，如果失败就修复 document/nl_ops_http_demo/index.html，然后再次验证直到通过。",
                ),
                context_bundle_summary: None,
                plan_result: &plan_result(vec![
                    PlanStep {
                        step_id: "s1".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "http_basic".to_string(),
                        args: json!({ "action": "get", "url": "http://127.0.0.1:51179/" }),
                        depends_on: Vec::new(),
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s2".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "read_file".to_string(),
                        args: json!({ "path": "document/nl_ops_http_demo/index.html" }),
                        depends_on: vec!["s1".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s3".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "write_file".to_string(),
                        args: json!({
                            "path": "document/nl_ops_http_demo/index.html",
                            "content": "ops-repair-ok\n"
                        }),
                        depends_on: vec!["s2".to_string()],
                        why: String::new(),
                    },
                    PlanStep {
                        step_id: "s4".to_string(),
                        action_type: "call_skill".to_string(),
                        skill: "http_basic".to_string(),
                        args: json!({ "action": "get", "url": "http://127.0.0.1:51179/" }),
                        depends_on: vec!["s3".to_string()],
                        why: String::new(),
                    },
                ]),
                execution_recipe: apply_recipe,
            },
            VerifyMode::ObserveOnly,
        );
    assert_eq!(result.rewritten_steps.len(), 3);
    assert_eq!(result.rewritten_steps[0].step_id, "s2");
    assert_eq!(result.rewritten_steps[1].step_id, "s3");
    assert_eq!(result.rewritten_steps[2].step_id, "s4");
}
