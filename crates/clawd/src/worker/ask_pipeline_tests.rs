use super::test_support::{
    executable_filename_route, make_temp_root, test_state_with_root, unresolved_deictic_analysis,
};
use super::{
    apply_ask_post_route, background_only_locator_route_should_force_clarify,
    clarify_fallback_source_or_default, current_request_resolves_workspace_child_locator,
    path_scoped_locator_guard_can_defer_to_prompt_targets,
    prebind_clarify_workspace_child_locator_from_current_request,
    prebind_existing_workspace_locator_hint_from_current_request,
    prebind_runtime_status_scalar_path_to_current_workspace,
    promote_clarify_path_scoped_filename_targets_to_execute,
    promote_clarify_resolved_multifile_targets_to_execute, route_reason_has_marker,
    unbound_model_context_target_route_should_force_clarify,
    unbound_targeted_evidence_route_should_force_clarify,
    WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
};

#[test]
fn compound_file_paths_summary_repair_runs_before_unbound_context_guard() {
    let state = test_state_with_root(make_temp_root("compound_file_paths_guard_order"));
    let task = crate::ClaimedTask {
        task_id: "compound-file-paths-guard-order".to_string(),
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
        "Find repository TOML files and provide a brief representative summary.".to_string();
    route.route_reason =
        "llm_semantic_contract_repair:file_paths_contract_needs_synthesis_upgrade".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::FilePaths;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "find toml files in this repo and briefly mention a few representative ones",
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
        applied.execution_route_result.output_contract.semantic_kind,
        crate::OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "compound_file_paths_plus_content_summary_contract_repaired"
    ));
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "unbound_model_context_target_requires_clarify"
    ));
}

#[test]
fn bare_stem_workspace_file_locator_executes_when_current_request_resolves_same_file() {
    let root = make_temp_root("bare_stem_file_locator_executes");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Test\n\nbody").expect("README fixture");
    let state = test_state_with_root(root);
    let task = crate::ClaimedTask {
        task_id: "bare-stem-file-locator-executes".to_string(),
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
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "Read README and summarize the opening in one sentence",
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
        crate::OutputLocatorKind::Path
    );
    assert_eq!(
        applied.execution_route_result.output_contract.locator_hint,
        readme
            .canonicalize()
            .expect("canonical readme")
            .display()
            .to_string()
    );
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST
    ));
    assert!(!route_reason_has_marker(
        &applied.execution_route_result,
        "model_completed_workspace_file_locator_requires_clarify"
    ));
}

#[test]
fn explicit_workspace_file_name_locator_still_executes() {
    let root = make_temp_root("explicit_file_name_locator_executes");
    std::fs::write(root.join("README.md"), "# Test\n\nbody").expect("README fixture");
    let state = test_state_with_root(root);
    let task = crate::ClaimedTask {
        task_id: "explicit-file-name-locator-executes".to_string(),
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
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "Read README.md and summarize the opening in one sentence",
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
        crate::OutputLocatorKind::Path
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .ends_with("README.md"));
}

#[test]
fn inferred_missing_workspace_locator_hint_requires_clarify() {
    let root = make_temp_root("inferred_missing_workspace_locator");
    let nested = root.join("locator_smart").join("case_only");
    std::fs::create_dir_all(&nested).expect("nested dir");
    std::fs::write(nested.join("Report.MD"), "# Report\n").expect("report fixture");
    let missing_direct_child = root.join("case_only");
    let state = test_state_with_root(root);
    let task = crate::ClaimedTask {
        task_id: "inferred-missing-workspace-locator".to_string(),
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
    route.resolved_intent = "Find report.md inside case_only and output the path.".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = missing_direct_child.display().to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "find report.md inside case_only",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(
        applied.execution_route_result.needs_clarify,
        "{}",
        applied.execution_route_result.route_reason
    );
    assert_eq!(
        applied.execution_route_result.gate_kind(),
        crate::RouteGateKind::Clarify
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
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "inferred_missing_workspace_locator_requires_clarify"
    ));
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "clarify_reason_code:missing_search_locator"
    ));
}

#[test]
fn inferred_existing_workspace_locator_hint_still_executes() {
    let root = make_temp_root("inferred_existing_workspace_locator");
    let direct_child = root.join("case_only");
    std::fs::create_dir_all(&direct_child).expect("direct child dir");
    std::fs::write(direct_child.join("Report.MD"), "# Report\n").expect("report fixture");
    let state = test_state_with_root(root);
    let task = crate::ClaimedTask {
        task_id: "inferred-existing-workspace-locator".to_string(),
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
    route.resolved_intent = "Find report.md inside case_only and output the path.".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExistenceWithPathSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = direct_child.display().to_string();
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "find report.md inside case_only",
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
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "workspace_locator_hint_prebound_from_current_request"
    ));
}

#[test]
fn llm_failed_normalizer_source_uses_llm_unavailable_fallback_source() {
    assert_eq!(
        clarify_fallback_source_or_default(Some(
            crate::fallback::ClarifyFallbackSource::LlmUnavailable
        )),
        crate::fallback::ClarifyFallbackSource::LlmUnavailable
    );
}

#[test]
fn absent_normalizer_fallback_source_uses_intent_unresolved() {
    assert_eq!(
        clarify_fallback_source_or_default(None),
        crate::fallback::ClarifyFallbackSource::IntentUnresolved
    );
}

#[test]
fn unbound_current_workspace_count_clarify_survives_auto_locator_policy() {
    let root = make_temp_root("unbound_count_auto_locator_guard");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "unbound-count-auto-locator-guard".to_string(),
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
    route.resolved_intent = format!(
        "Count direct child entries in {} and output only the number",
        root.display()
    );
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        "数一下那个目录里有多少个直接子项，只输出数字",
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.execution_route_result.needs_clarify);
    assert!(applied.execution_route_result.is_clarify_gate());
    assert!(applied.auto_locator_path.is_none());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("unbound_targeted_evidence_requires_clarify"));
}

#[test]
fn deictic_count_with_model_current_workspace_rewrite_still_clarifies() {
    let root = make_temp_root("deictic_count_workspace_rewrite_guard");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "deictic-count-workspace-rewrite-guard".to_string(),
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
        "Count direct child entries in the current workspace and output only the number"
            .to_string();
    route.route_reason =
        "semantic_contract_requires_evidence; model_rewrote_deictic_target_to_current_workspace"
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarCount;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    let resolved_prompt = format!(
        "{}\n\n[AUTO_LOCATOR]\nResolved present workspace scope to: {}\nUse this path as the target unless user explicitly overrides it.\n",
        route.resolved_intent,
        root.display()
    );

    let applied = apply_ask_post_route(
        &state,
        &task,
        "数一下那个目录里有多少个直接子项，只输出数字",
        &resolved_prompt,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.execution_route_result.needs_clarify);
    assert!(applied.execution_route_result.is_clarify_gate());
    assert!(applied.auto_locator_path.is_none());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("unbound_targeted_evidence_requires_clarify"));
}

#[test]
fn post_route_suppresses_auto_locator_for_multiple_explicit_path_targets() {
    let root = make_temp_root("multi_explicit_path_auto_locator");
    std::fs::create_dir_all(root.join("UI")).expect("create ui");
    std::fs::create_dir_all(root.join("crates/clawd")).expect("create clawd");
    std::fs::write(root.join("UI/package.json"), r#"{"name":"react-example"}"#)
        .expect("write package");
    std::fs::write(
        root.join("crates/clawd/Cargo.toml"),
        "[package]\nname=\"clawd\"\n",
    )
    .expect("write cargo");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "multi-explicit-path-auto-locator".to_string(),
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
        "Compare UI package name with clawd crate package name and output a scalar result."
            .to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "UI/package.json".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let prompt = "read UI/package.json name and crates/clawd/Cargo.toml package.name, then compare";
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.auto_locator_path.is_none());
    assert!(!applied
        .prompt_with_memory_for_execution
        .contains("[AUTO_LOCATOR]"));
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("auto_locator_suppressed_multiple_explicit_paths"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_suppresses_auto_locator_for_mixed_bare_filename_and_path_targets() {
    let root = make_temp_root("mixed_bare_filename_path_auto_locator");
    std::fs::create_dir_all(root.join("UI")).expect("create ui");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"rustclaw\"\n").expect("write cargo");
    std::fs::write(root.join("README.md"), "# RustClaw\n").expect("write readme");
    std::fs::write(root.join("UI/package.json"), r#"{"name":"react-example"}"#)
        .expect("write package");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "mixed-explicit-path-auto-locator".to_string(),
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
        "Read package metadata and README excerpt, then classify the repository kind.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "UI/package.json".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let prompt =
        "先读 Cargo.toml 里的 package.name，再读 UI/package.json 里的 name，再看 README.md 开头";
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.auto_locator_path.is_none());
    assert!(!applied
        .prompt_with_memory_for_execution
        .contains("[AUTO_LOCATOR]"));
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("auto_locator_suppressed_multiple_explicit_paths"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_does_not_prebind_workspace_child_locator_for_mixed_multi_file_targets() {
    let root = make_temp_root("mixed_multi_file_no_workspace_child_prebind");
    std::fs::create_dir_all(root.join("UI")).expect("create ui");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"rustclaw\"\n").expect("write cargo");
    std::fs::write(root.join("README.md"), "# RustClaw\n").expect("write readme");
    std::fs::write(root.join("UI/package.json"), r#"{"name":"react-example"}"#)
        .expect("write package");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "mixed-multi-file-no-workspace-child-prebind".to_string(),
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
        "Read package metadata and README excerpt, then classify the repository kind.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let prompt =
        "先读 Cargo.toml 里的 package.name，再读 UI/package.json 里的 name，再看 README.md 开头";
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.auto_locator_path.is_none());
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .trim()
        .is_empty());
    assert!(!applied
        .execution_route_result
        .route_reason
        .contains("workspace_child_locator_prebound_from_current_request"));
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("auto_locator_suppressed_multiple_explicit_paths"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_suppresses_auto_locator_for_bare_directory_and_nested_path_targets() {
    let root = make_temp_root("bare_directory_nested_path_auto_locator");
    std::fs::create_dir_all(root.join("crates/skills")).expect("create skills");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "bare-directory-nested-path-auto-locator".to_string(),
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
        "Count direct entries under crates and crates/skills, then summarize layout.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "crates/skills".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::DirectoryNames;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let prompt = "count how many entries are directly under crates, then count how many are under crates/skills";
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(applied.auto_locator_path.is_none());
    assert!(!applied
        .prompt_with_memory_for_execution
        .contains("[AUTO_LOCATOR]"));
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("auto_locator_suppressed_multiple_explicit_paths"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn recent_scalar_clarify_with_explicit_workspace_files_prebinds_current_request_locator() {
    let root = make_temp_root("recent_scalar_explicit_files_prebind");
    let cargo = root.join("Cargo.toml");
    let readme = root.join("README.md");
    std::fs::write(&cargo, "[workspace.package]\nversion = \"0.1.0\"\n").expect("cargo");
    std::fs::write(&readme, "version: 0.1.0\n").expect("readme");
    let state = test_state_with_root(root.clone());
    let prompt = "Read workspace package version from Cargo.toml and compare it with the version mentioned in README.md, then answer in one sentence.";
    let expected = cargo
        .canonicalize()
        .expect("canonical cargo")
        .display()
        .to_string();
    assert_eq!(
        current_request_resolves_workspace_child_locator(&state, prompt).as_deref(),
        Some(expected.as_str())
    );

    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.ask_mode = crate::AskMode::clarify();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;

    assert!(
        prebind_clarify_workspace_child_locator_from_current_request(&state, prompt, &mut route,)
    );
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
    );
    assert_eq!(route.output_contract.locator_hint, expected);
    assert!(route
        .route_reason
        .contains("workspace_child_locator_prebound_from_clarify_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn path_scoped_locator_guard_defers_to_prompt_filename_targets() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::CommandOutputSummary;

    assert!(path_scoped_locator_guard_can_defer_to_prompt_targets(
        "read Cargo.toml package.name, UI/package.json name, and README.md opening",
        &route,
    ));
    assert!(!path_scoped_locator_guard_can_defer_to_prompt_targets(
        "read the package name and summarize the project kind",
        &route,
    ));
}

#[test]
fn clarify_path_scoped_filename_targets_promote_to_workspace_execution() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;

    assert!(promote_clarify_path_scoped_filename_targets_to_execute(
        "README.md AGENTS.md",
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route
        .route_reason
        .contains("clarify_path_scoped_filename_targets_promoted_to_execute"));
}

#[test]
fn clarify_current_workspace_filename_targets_promote_to_workspace_execution() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint = "/tmp/workspace".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.route_reason =
        "workspace_filename_targets_contract_repair; semantic_contract_requires_evidence"
            .to_string();

    assert!(promote_clarify_path_scoped_filename_targets_to_execute(
        "read the opening section of README.md, then read the opening section of AGENTS.md, and say in one short English sentence which one is for end users versus contributors",
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route
        .route_reason
        .contains("clarify_path_scoped_filename_targets_promoted_to_execute"));
}

#[test]
fn clarify_current_workspace_mixed_multifile_targets_promote_to_workspace_execution() {
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.route_reason = "semantic_contract_requires_evidence".to_string();

    assert!(promote_clarify_path_scoped_filename_targets_to_execute(
        "先读 Cargo.toml 里的 package.name，再读 UI/package.json 里的 name，再看 README.md 开头",
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route
        .route_reason
        .contains("clarify_path_scoped_filename_targets_promoted_to_execute"));
}

#[test]
fn clarify_resolved_multifile_targets_promote_to_workspace_execution_even_without_locator_kind() {
    let root = make_temp_root("resolved_multifile_promote_without_locator_kind");
    std::fs::create_dir_all(root.join("UI")).expect("create ui");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"rustclaw\"\n").expect("write cargo");
    std::fs::write(root.join("README.md"), "# RustClaw\n").expect("write readme");
    std::fs::write(root.join("UI/package.json"), r#"{"name":"react-example"}"#)
        .expect("write package");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.route_reason = "semantic_contract_requires_evidence".to_string();

    assert!(promote_clarify_resolved_multifile_targets_to_execute(
        &state,
        "先读 Cargo.toml 里的 package.name，再读 UI/package.json 里的 name，再看 README.md 开头",
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route
        .route_reason
        .contains("clarify_resolved_multifile_targets_promoted_to_execute"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn clarify_resolved_bare_readme_agents_targets_promote_to_workspace_execution() {
    let root = make_temp_root("resolved_readme_agents_promote");
    std::fs::write(root.join("README.md"), "# User Guide\n").expect("write readme");
    std::fs::write(root.join("AGENTS.md"), "# Agent Rules\n").expect("write agents");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.route_reason = "semantic_contract_requires_evidence".to_string();

    assert!(promote_clarify_resolved_multifile_targets_to_execute(
        &state,
        "read the opening section of README.md, then read the opening section of AGENTS.md, and say in one short English sentence which one is for end users versus contributors",
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn clarify_resolved_multifile_targets_promote_from_semantic_evidence_marker() {
    let root = make_temp_root("resolved_multifile_promote_from_marker");
    std::fs::write(root.join("README.md"), "# User Guide\n").expect("write readme");
    std::fs::write(root.join("AGENTS.md"), "# Agent Rules\n").expect("write agents");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.route_reason =
        "semantic_contract_requires_evidence; auto_locator_suppressed_multiple_explicit_paths"
            .to_string();

    assert!(promote_clarify_resolved_multifile_targets_to_execute(
        &state,
        "read the opening section of README.md, then read the opening section of AGENTS.md",
        &mut route,
    ));
    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert!(route.output_contract.requires_content_evidence);

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ambiguous_filename_token_does_not_prebind_memory_supplied_path_or_weak_file_stem() {
    let root = make_temp_root("ambiguous_filename_token");
    std::fs::write(root.join("README.md"), "# Root\n").expect("root readme");
    let docs_dir = root.join("docs");
    std::fs::create_dir_all(&docs_dir).expect("docs dir");
    let docs_readme = docs_dir.join("README.md");
    std::fs::write(&docs_readme, "# Docs\n").expect("docs readme");
    let state = test_state_with_root(root.clone());
    let docs_readme = docs_readme
        .canonicalize()
        .expect("canonical docs readme")
        .display()
        .to_string();
    let mut route = executable_filename_route();
    route.resolved_intent = format!("Read the beginning of {docs_readme}");
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = docs_readme;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;

    assert!(
        !prebind_existing_workspace_locator_hint_from_current_request(
            &state,
            "读一下 README 开头并总结",
            &mut route,
        )
    );
    assert!(!route
        .route_reason
        .contains(WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST));

    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "读一下 README 开头并总结",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn current_turn_resolved_stem_allows_matching_absolute_locator_hint() {
    let root = make_temp_root("current_turn_resolved_stem_absolute_locator");
    let readme = root.join("README.md");
    std::fs::write(&readme, "# Root\n").expect("root readme");
    let docs_dir = root.join("docs");
    std::fs::create_dir_all(&docs_dir).expect("docs dir");
    std::fs::write(docs_dir.join("README.md"), "# Docs\n").expect("docs readme");
    let state = test_state_with_root(root.clone());
    let readme = readme
        .canonicalize()
        .expect("canonical root readme")
        .display()
        .to_string();
    let mut route = executable_filename_route();
    route.resolved_intent = format!("Read the beginning of {readme}");
    route.route_reason = "current_turn_anchor_overrides_contextual_target".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = readme.clone();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let prompt = "读取 README 的前 20 行并做 3 句话总结";

    assert!(
        prebind_existing_workspace_locator_hint_from_current_request(&state, prompt, &mut route,)
    );
    assert_eq!(route.output_contract.locator_hint, readme);
    assert!(route_reason_has_marker(
        &route,
        WORKSPACE_LOCATOR_HINT_PREBOUND_FROM_CURRENT_REQUEST,
    ));
    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        prompt,
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn background_only_locator_rewrite_allows_current_turn_filename() {
    let state = test_state_with_root(make_temp_root("background_locator_filename"));
    let mut route = executable_filename_route();
    route.resolved_intent = "读取 README.md 前 3 行".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    route.output_contract.requires_content_evidence = true;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!background_only_locator_route_should_force_clarify(
        &state,
        "README.md",
        &route.resolved_intent,
        "<none>",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn recent_artifacts_judgment_uses_recent_execution_context_without_locator() {
    let mut route = executable_filename_route();
    route.resolved_intent =
        "Compare two recently observed file excerpts and return the selected filename".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentArtifactsJudgment;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let prompt = "compare the last two observed excerpts and return only the selected filename";
    assert!(unbound_targeted_evidence_route_should_force_clarify(
        prompt, &route, &snapshot, "<none>",
    ));
    let recent_execution_context = "\
### RECENT_EXECUTION_EVENTS
- ts=2 kind=ask request=read README.md result=# RustClaw
Chinese version: README.zh-CN.md
- ts=1 kind=ask request=read service_notes.md result=# Service Notes
RustClaw test fixture service notes.";

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        prompt,
        &route,
        &snapshot,
        recent_execution_context,
    ));
}

#[test]
fn clarify_recent_scalar_field_pair_with_two_explicit_paths_promotes_to_execution() {
    let root = make_temp_root("clarify_recent_scalar_field_pair_two_paths");
    std::fs::create_dir_all(root.join("UI")).expect("ui directory");
    std::fs::create_dir_all(root.join("crates/clawd")).expect("clawd directory");
    std::fs::write(root.join("UI/package.json"), r#"{"name":"react-example"}"#)
        .expect("package json");
    std::fs::write(
        root.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("cargo toml");
    let state = test_state_with_root(root.clone());
    let mut route = executable_filename_route();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.route_reason =
        "semantic_contract_requires_evidence; auto_locator_suppressed_multiple_explicit_paths"
            .to_string();

    assert!(promote_clarify_resolved_multifile_targets_to_execute(
        &state,
        "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，then 只用一行输出“前者 | 后者 | 一样/不一样”",
        &mut route,
    ));

    assert!(!route.needs_clarify);
    assert!(route.is_execute_gate());
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(route.output_contract.locator_hint.is_empty());
    assert!(route
        .route_reason
        .contains("clarify_resolved_multifile_targets_promoted_to_execute"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_promotes_clarify_recent_scalar_field_pair_with_two_explicit_paths() {
    let root = make_temp_root("post_route_scalar_field_pair_two_paths");
    std::fs::create_dir_all(root.join("UI")).expect("ui directory");
    std::fs::create_dir_all(root.join("crates/clawd")).expect("clawd directory");
    std::fs::write(root.join("UI/package.json"), r#"{"name":"react-example"}"#)
        .expect("package json");
    std::fs::write(
        root.join("crates/clawd/Cargo.toml"),
        r#"[package]
name = "clawd"
"#,
    )
    .expect("cargo toml");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "post-route-scalar-field-pair-two-paths".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let prompt = "读取 UI/package.json 里的 name，再读取 crates/clawd/Cargo.toml 里的 package.name，then 只用一行输出“前者 | 后者 | 一样/不一样”";
    let resolved_intent = "读取 UI/package.json 的 name 字段值和 crates/clawd/Cargo.toml 的 package.name 字段值，然后在一行内输出“前者 | 后者 | 一样/不一样”";
    let mut route = executable_filename_route();
    route.resolved_intent = resolved_intent.to_string();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::RecentScalarEqualityCheck;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.route_reason = "semantic_contract_requires_evidence".to_string();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(!applied.execution_route_result.needs_clarify);
    assert!(applied.execution_route_result.is_execute_gate());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace,
        "route_reason={} locator_hint={} needs_clarify={}",
        applied.execution_route_result.route_reason,
        applied.execution_route_result.output_contract.locator_hint,
        applied.execution_route_result.needs_clarify,
    );
    assert!(applied
        .execution_route_result
        .output_contract
        .locator_hint
        .is_empty());
    assert!(applied.auto_locator_path.is_none());
    assert!(!applied
        .prompt_with_memory_for_execution
        .contains("[AUTO_LOCATOR]"));
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("clarify_resolved_multifile_targets_promoted_to_execute"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_promotes_current_workspace_multifile_excerpt_judgment() {
    let root = make_temp_root("post_route_multifile_excerpt_judgment");
    std::fs::write(root.join("README.md"), "# User Guide\n\nFor users.\n").expect("readme");
    std::fs::write(
        root.join("AGENTS.md"),
        "# Agent Rules\n\nFor contributors.\n",
    )
    .expect("agents");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "post-route-multifile-excerpt-judgment".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let prompt = "先读 README.md 开头，再读 AGENTS.md 开头，最后只用一句中文说前者更像哪类文档";
    let resolved_intent = "读取项目根目录下的 README.md 和 AGENTS.md 的开头部分，随后用一句中文判断 README.md 更像面向用户的项目介绍还是协作者规则说明";
    let mut route = executable_filename_route();
    route.resolved_intent = resolved_intent.to_string();
    route.needs_clarify = false;
    route.set_planner_execute_finalize(crate::ActFinalizeStyle::ChatWrapped);
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExcerptKindJudgment;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.route_reason = "semantic_contract_requires_evidence".to_string();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    assert!(!applied.execution_route_result.needs_clarify);
    assert!(applied.execution_route_result.is_execute_gate());
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::CurrentWorkspace
    );
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("post_route_resolved_multifile_targets_promoted_to_execute"));
    assert!(applied.auto_locator_path.is_none());

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_defers_subagent_boundary_clarify_to_agent_loop_when_current_plan_exists() {
    let root = make_temp_root("post_route_subagent_boundary_plan");
    std::fs::write(root.join("AGENTS.md"), "# Agent Rules\n").expect("agents");
    std::fs::create_dir_all(root.join("plan")).expect("plan dir");
    std::fs::write(root.join("plan/current.md"), "# Current Plan\n").expect("plan");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "post-route-subagent-boundary-plan".to_string(),
        user_id: 1,
        chat_id: 1,
        user_key: None,
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: "{}".to_string(),
    };
    let prompt = "review subagent boundary audit";
    let resolved_intent = "review subagent boundary audit";
    let mut route = executable_filename_route();
    route.resolved_intent = resolved_intent.to_string();
    route.needs_clarify = true;
    route.set_clarify_gate();
    route.agent_display_name_hint = "review-subagent".to_string();
    route.risk_ceiling = crate::RiskCeiling::Low;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ContentExcerptWithSummary;
    route.output_contract.locator_kind = crate::OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    route.route_reason = "structured_locator_contract_repair".to_string();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        resolved_intent,
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
    assert!(route_reason_has_marker(
        &applied.execution_route_result,
        "subagent_boundary_clarify_deferred_to_agent_loop"
    ));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn post_route_quantity_compare_preserves_explicit_directory_pair_over_parent_locator() {
    let root = make_temp_root("quantity_pair_over_parent_locator");
    std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/docs"))
        .expect("docs directory");
    std::fs::create_dir_all(root.join("scripts/nl_tests/fixtures/device_local/logs"))
        .expect("logs directory");
    let state = test_state_with_root(root.clone());
    let task = crate::ClaimedTask {
        task_id: "quantity-pair-over-parent-locator".to_string(),
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
        "Count direct children of docs and logs directories and state which has more".to_string();
    route.output_contract.semantic_kind = crate::OutputSemanticKind::QuantityComparison;
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "scripts/nl_tests/fixtures/device_local".to_string();
    route.output_contract.response_shape = crate::OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = false;
    let prompt = "先数 scripts/nl_tests/fixtures/device_local/docs 直接子项数量，再数 scripts/nl_tests/fixtures/device_local/logs 直接子项数量，最后一句中文说哪个更多";
    let resolved_intent = route.resolved_intent.clone();

    let applied = apply_ask_post_route(
        &state,
        &task,
        prompt,
        &resolved_intent,
        "",
        None,
        route,
        String::new(),
        String::new(),
    );

    let targets = crate::task_contract::target_locators_for_route(&applied.execution_route_result);
    assert_eq!(targets.len(), 2, "{targets:?}");
    assert!(targets[0].ends_with("/docs"), "{targets:?}");
    assert!(targets[1].ends_with("/logs"), "{targets:?}");
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("quantity_compare_path_pair_prebound_from_current_request"));

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn scalar_path_with_active_ordered_anchor_without_ref_does_not_bind_current_workspace() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Return only the selected ordered-entry path.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskAppend),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "list matching files".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            bound_target: Some("/tmp/rustclaw".to_string()),
            ordered_entries: vec![
                "alpha.txt".to_string(),
                "beta.txt".to_string(),
                "gamma.txt".to_string(),
            ],
            source_task_id: "task-list".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..Default::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route,
        Some(&analysis),
        &snapshot,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_path_only_missing_ordered_entry_ref_not_bound_to_current_workspace"));
}

#[test]
fn scalar_path_active_task_update_without_locator_does_not_bind_current_workspace() {
    let mut route = executable_filename_route();
    route.resolved_intent = "Identify the file that still needs confirmation.".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.delivery_required = false;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ScalarPathOnly;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let analysis = crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskScopeUpdate),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::ReuseActive),
        should_interrupt_active_run: false,
        state_patch: None,
        attachment_processing_required: false,
    };
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!prebind_runtime_status_scalar_path_to_current_workspace(
        &mut route,
        Some(&analysis),
        &snapshot,
    ));
    assert_eq!(
        route.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(route
        .route_reason
        .contains("scalar_path_only_active_task_update_not_bound_to_current_workspace"));
}

#[test]
fn unbound_model_context_allows_file_surface_with_structured_reference() {
    let root = make_temp_root("structured_file_surface_with_reference");
    std::fs::write(root.join("package.json"), r#"{"name":"demo-ui"}"#).expect("package");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"demo\"\n").expect("manifest");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Read package.json name and Cargo.toml package.name, then return the first observed scalar value.".to_string();
    route.route_reason = "structured_file_scalar_repair".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    route.output_contract.response_shape = crate::OutputResponseShape::Scalar;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_model_context_target_route_should_force_clarify(
        &state,
        "Read package.json name, read Cargo.toml package.name, then return the former value.",
        &route,
        None,
        &snapshot,
    ));
}

#[test]
fn execution_failed_step_does_not_trigger_unbound_targeted_evidence_guard() {
    let mut route = executable_filename_route();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::ExecutionFailedStep;
    route.output_contract.response_shape = crate::OutputResponseShape::Strict;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(!unbound_targeted_evidence_route_should_force_clarify(
        "run the requested commands and report the failed execution step",
        &route,
        &snapshot,
        "<none>",
    ));
}

#[test]
fn unclassified_workspace_child_keyword_does_not_bypass_background_locator_guard() {
    let root = make_temp_root("workspace_child_keyword_not_locator");
    std::fs::create_dir_all(root.join("target")).expect("target dir");
    let state = test_state_with_root(root);
    let mut route = executable_filename_route();
    route.resolved_intent = "Inspect schema target enum".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route.output_contract.locator_hint = "target".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let analysis = unresolved_deictic_analysis();

    assert!(background_only_locator_route_should_force_clarify(
        &state,
        "inspect schema target enum",
        &route.resolved_intent,
        "<none>",
        &route,
        Some(&analysis),
        &snapshot,
    ));
}

#[test]
fn bare_topic_resolved_prompt_prebind_still_forces_clarify() {
    let root = make_temp_root("bare_topic_resolved_prompt_prebind");
    let config_dir = root.join("configs");
    std::fs::create_dir_all(&config_dir).expect("create config dir");
    let config_path = config_dir.join("config.toml");
    std::fs::write(&config_path, "[skills]\n[skills.skill_switches]\n").expect("write config");
    let state = test_state_with_root(root);
    let task = crate::ClaimedTask {
        task_id: "bare-topic-resolved-prebind".to_string(),
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
    route.ask_mode = crate::AskMode::clarify();
    route.resolved_intent = "Apply the config change to the resolved config path".to_string();
    route.output_contract.locator_kind = crate::OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = crate::OutputResponseShape::Free;
    route.output_contract.semantic_kind = crate::OutputSemanticKind::None;
    let resolved_prompt = format!(
        "Apply the config change to {}, setting skills.skill_switches.config_edit_nl_plan to true",
        config_path.display()
    );

    let applied = apply_ask_post_route(
        &state,
        &task,
        "apply_temp_config_cn",
        &resolved_prompt,
        "",
        None,
        route,
        resolved_prompt.clone(),
        resolved_prompt.clone(),
    );

    assert!(applied.execution_route_result.needs_clarify);
    assert_eq!(
        applied.execution_route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::None
    );
    assert!(applied
        .execution_route_result
        .route_reason
        .contains("bare_topic_model_supplied_locator_requires_clarify"));
}
