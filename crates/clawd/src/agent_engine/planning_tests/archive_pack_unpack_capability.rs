use super::*;

#[test]
fn archive_unpack_semantic_kind_without_capability_ref_does_not_plan() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchiveUnpack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
            .to_string();

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "ArchiveUnpack semantic marker alone must not choose archive.unpack before the planner"
    );
}

#[test]
fn archive_pack_semantic_kind_without_capability_ref_does_not_plan() {
    let mut route = base_route_result();
    route.output_contract.semantic_kind = OutputSemanticKind::ArchivePack;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/skill_calls | tmp/nl_archive_case_en.zip".to_string();

    assert!(
        crate::evidence_policy::capability_ref_action_refs_for_route(&route, false).is_empty(),
        "ArchivePack semantic marker alone must not choose archive.pack before the planner"
    );
}
