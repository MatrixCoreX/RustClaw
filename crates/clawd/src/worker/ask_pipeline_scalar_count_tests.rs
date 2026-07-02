use super::test_support::{executable_filename_route, make_temp_root, test_state_with_root};
use super::{
    build_loop_context_after_boundary_preflight, route_reason_has_marker,
    unbound_targeted_evidence_route_should_defer_to_agent_loop,
};

fn assert_current_workspace_scope_boundary(prompt: &str, root: &std::path::Path) {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let block = prompt
        .split(START)
        .nth(1)
        .and_then(|tail| tail.split(END).next())
        .expect("boundary observation block should be present")
        .trim();
    let observation: serde_json::Value =
        serde_json::from_str(block).expect("boundary observation should be JSON");
    let scope = observation
        .get("current_workspace_scope")
        .expect("current workspace scope should be present");
    assert_eq!(
        scope.get("task_shape").and_then(serde_json::Value::as_str),
        Some("scalar_count")
    );
    assert_eq!(
        scope
            .get("contract_marker")
            .and_then(serde_json::Value::as_str),
        Some("scalar_count")
    );
    assert!(scope.get("semantic_kind").is_none());
    let root_display = root.display().to_string();
    assert_eq!(
        scope.get("target").and_then(serde_json::Value::as_str),
        Some(root_display.as_str())
    );
}

#[test]
fn unbound_targeted_evidence_allows_current_workspace_scalar_count_scope() {
    let mut route = executable_filename_route();
    route.resolved_intent = "count top-level workspace directories".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/tmp/rustclaw".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "count top-level repository directories",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn current_workspace_scalar_count_structured_locator_exports_boundary_scope() {
    let root = make_temp_root("current_workspace_scalar_count_root_hint");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "current-workspace-scalar-count-root-hint".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Count top-level workspace directories and return the scalar count.".to_string();
    route.route_reason = "semantic_contract_requires_evidence".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let resolved_intent = route.resolved_intent.clone();

    let applied = build_loop_context_after_boundary_preflight(
        &state,
        &task,
        "count top-level workspace directories and return only the number",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(
        !applied.execution_route_result.needs_clarify,
        "{}",
        applied.execution_route_result.route_reason
    );
    assert!(applied.execution_route_result.is_execute_gate());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "current_workspace_root_hint_prebound_for_scalar_count"
    ));
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "unbound_targeted_evidence_requires_clarify"
    ));
    assert_current_workspace_scope_boundary(&applied.prompt_with_memory_for_execution, &root);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_scalar_count_marker_from_clarify_route_exports_boundary_scope() {
    let root = make_temp_root("current_workspace_scalar_count_marker_clarify_route");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "current-workspace-scalar-count-marker-clarify-route".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.resolved_intent =
        "Count current workspace top-level entries excluding the VCS control directory and return only the number."
            .to_string();
    route.route_reason =
        "semantic_contract_requires_evidence; current_workspace_scope_from_current_request"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let resolved_intent = route.resolved_intent.clone();

    let applied = build_loop_context_after_boundary_preflight(
        &state,
        &task,
        "列出仓库顶层目录，但不要把 .git 算进去，只告诉我其它的有几个",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "current_workspace_root_hint_prebound_for_scalar_count"
    ));
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "unbound_targeted_evidence_requires_clarify"
    ));
    assert_current_workspace_scope_boundary(&applied.prompt_with_memory_for_execution, &root);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_scalar_count_one_sentence_exports_boundary_scope() {
    let root = make_temp_root("current_workspace_scalar_count_one_sentence_root_hint");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "current-workspace-scalar-count-one-sentence-root-hint".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.resolved_intent = "Count top-level workspace files and return one sentence.".to_string();
    route.route_reason =
        "semantic_contract_requires_evidence; current_workspace_scope_from_current_request"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.exact_sentence_count = Some(1);
    let resolved_intent = route.resolved_intent.clone();

    let applied = build_loop_context_after_boundary_preflight(
        &state,
        &task,
        "count top-level workspace files and return one sentence",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(
        !applied.execution_route_result.needs_clarify,
        "{}",
        applied.execution_route_result.route_reason
    );
    assert!(applied.execution_route_result.is_execute_gate());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "current_workspace_root_hint_prebound_for_scalar_count"
    ));
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "unbound_targeted_evidence_requires_clarify"
    ));
    assert_current_workspace_scope_boundary(&applied.prompt_with_memory_for_execution, &root);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn clarify_current_workspace_scalar_count_with_resolved_root_exports_boundary_scope() {
    let root = make_temp_root("clarify_current_workspace_scalar_count_resolved_root");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "clarify-current-workspace-scalar-count-resolved-root".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.resolved_intent = format!(
        "Count regular files directly under the current directory {} and provide one short explanation",
        root.display()
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.exact_sentence_count = Some(1);
    let resolved_intent = route.resolved_intent.clone();

    let applied = build_loop_context_after_boundary_preflight(
        &state,
        &task,
        "Count how many regular files are directly under the current directory, and reply with just the number plus one short explanation.",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "current_workspace_root_hint_prebound_for_scalar_count"
    ));
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "unbound_targeted_evidence_requires_clarify"
    ));
    assert_current_workspace_scope_boundary(&applied.prompt_with_memory_for_execution, &root);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_workspace_scalar_count_with_unmentioned_root_path_requires_clarify() {
    let root = make_temp_root("workspace_injected_root");
    let mut route = executable_filename_route();
    route.resolved_intent = format!(
        "Count the number of direct child entries in {} and output only the number",
        root.display()
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = root.display().to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(unbound_targeted_evidence_route_should_defer_to_agent_loop(
        "count that directory's direct children and output only the number",
        &route,
        &snapshot,
        "<none>",
    ));
}
