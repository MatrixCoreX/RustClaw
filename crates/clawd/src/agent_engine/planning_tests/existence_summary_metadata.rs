use super::*;

#[test]
fn existence_summary_explicit_file_targets_allow_metadata_and_content_evidence() {
    let root = TempDirGuard::new("existence_summary_explicit_file_targets");
    let docs_dir = root.path.join("docs");
    fs::create_dir_all(&docs_dir).expect("create docs dir");
    fs::write(docs_dir.join("service_notes.md"), "# Service Notes\n").expect("write notes");
    fs::write(
        docs_dir.join("release_checklist.md"),
        "# Release Checklist\n",
    )
    .expect("write checklist");
    let left = "docs/service_notes.md";
    let right = "docs/release_checklist.md";
    let mut route = route_result(
        crate::AskMode::direct_answer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root.path.display().to_string();
    route.output_contract.delivery_required = false;
    route.resolved_intent = format!("compare {left} and {right} existence metadata");

    let contract = route.effective_output_contract();
    let stat_policy = crate::evidence_policy::action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({
            "action": "stat_paths",
            "paths": [left, right],
        }),
    )
    .expect("existence summary should allow metadata evidence");
    assert!(stat_policy.is_allowed(), "{stat_policy:?}");
    assert!(stat_policy.action_matches_preferred(), "{stat_policy:?}");

    let read_policy = crate::evidence_policy::action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &json!({
            "action": "read_text_range",
            "path": left,
            "mode": "head",
            "n": 80,
        }),
    )
    .expect("existence summary should allow content evidence");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert!(read_policy.action_matches_preferred(), "{read_policy:?}");
}
