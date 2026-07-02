use super::*;

#[test]
fn archive_unpack_semantic_kind_without_capability_ref_does_not_plan() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
            .to_string();
    let loop_state = LoopState::new(1);

    assert!(archive_unpack_deterministic_plan_result(
        "unpack archive",
        &state,
        Some(&route),
        &loop_state,
    )
    .is_none());
}

#[test]
fn archive_pack_semantic_kind_without_capability_ref_does_not_plan() {
    let state = test_state_with_enabled_skills(&["archive_basic"]);
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();
    let loop_state = LoopState::new(1);

    assert!(archive_pack_deterministic_plan_result(
        "pack archive",
        &state,
        Some(&route),
        &loop_state,
        "Zip scripts/skill_calls into tmp/nl_archive_case_en.zip",
        Some("Zip scripts/skill_calls into tmp/nl_archive_case_en.zip"),
        None,
    )
    .is_none());
}
