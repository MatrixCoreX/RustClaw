use super::*;

#[test]
fn archive_list_capability_ref_plans_list_without_archive_list_semantic_kind() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.route_reason = "capability_ref=archive.list".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = archive.to_string();

    let policy = crate::evidence_policy::capability_ref_action_policy_for_route(
        Some(&route),
        "archive_basic",
        &serde_json::json!({"action": "list", "archive": archive}),
    )
    .expect("archive.list capability ref should expose archive_basic.list to the planner");

    assert!(policy.is_allowed(), "{policy:?}");
    assert!(policy.action_matches_preferred(), "{policy:?}");
}

#[test]
fn archive_list_semantic_kind_without_capability_ref_does_not_plan_list() {
    let archive = "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip";
    let mut route = base_route_result();
    route.ask_mode = crate::AskMode::planner_execute_with_chat_finalizer();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveList;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.locator_hint = archive.to_string();

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "ArchiveList output marker alone must not choose archive.list before the planner"
    );
}
