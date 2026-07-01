use super::*;

#[test]
fn existence_summary_explicit_file_targets_collect_metadata_before_content_reads() {
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
    let mut state = test_state_with_enabled_skills(&["fs_basic"]);
    state.skill_rt.workspace_root = root.path.clone();
    let mut route = route_result(
        crate::AskMode::planner_execute_with_chat_finalizer(),
        true,
        OutputResponseShape::Strict,
    );
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root.path.display().to_string();
    route.output_contract.delivery_required = false;
    route.resolved_intent = format!("compare {left} and {right} existence metadata");
    let user_text = format!("Return path metadata for {left} and {right}.");

    let plan = content_excerpt_explicit_file_targets_deterministic_plan_result(
        &state,
        "return metadata and content summary for explicit paths",
        Some(&route),
        &LoopState::new(1),
        &user_text,
        None,
        Some(root.path.to_string_lossy().as_ref()),
    )
    .expect("existence summary should collect metadata before content reads");

    assert_eq!(plan.steps.len(), 5);
    let actions = plan
        .steps
        .iter()
        .filter_map(|step| step.to_agent_action())
        .collect::<Vec<_>>();
    let stat_args = expect_planned_call(&actions[0], "fs_basic", "stat_paths");
    let stat_paths = stat_args
        .get("paths")
        .and_then(Value::as_array)
        .expect("stat paths");
    assert_eq!(stat_paths.len(), 2);
    assert!(matches!(
        &actions[1],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str).is_some_and(|path| path.ends_with(left))
    ));
    assert!(matches!(
        &actions[2],
        AgentAction::CallTool { tool, args }
            if tool == "fs_basic"
                && args.get("action").and_then(Value::as_str) == Some("read_text_range")
                && args.get("path").and_then(Value::as_str).is_some_and(|path| path.ends_with(right))
    ));
}
