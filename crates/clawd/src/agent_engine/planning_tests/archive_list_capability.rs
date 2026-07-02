use super::*;

#[test]
fn archive_list_capability_ref_plans_list_without_archive_list_semantic_kind() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.route_reason = "capability_ref=archive.list".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = archive.to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "inspect archive contents",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
    )
    .expect("archive.list capability ref should plan archive listing");

    assert_eq!(plan.steps.len(), 3);
    match &plan.steps[0].to_agent_action() {
        Some(AgentAction::CallSkill { skill, args }) => {
            assert_eq!(skill, "archive_basic");
            assert_eq!(args.get("action").and_then(Value::as_str), Some("list"));
            assert_eq!(args.get("archive").and_then(Value::as_str), Some(archive));
        }
        other => panic!("expected archive_basic list action, got {other:?}"),
    }
}

#[test]
fn archive_list_semantic_kind_without_capability_ref_does_not_plan_list() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveList;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = archive.to_string();
    let loop_state = LoopState::new(1);

    let plan = archive_list_auto_locator_deterministic_plan_result(
        "inspect archive contents",
        &state,
        Some(&route),
        &loop_state,
        Some(archive),
    );

    assert!(
        plan.is_none(),
        "ArchiveList output marker alone must not choose archive.list before the planner"
    );
}
