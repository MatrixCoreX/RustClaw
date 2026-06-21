use super::*;

#[test]
fn publishing_preview_allows_x_preview_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::PublishingPreview,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::OneSentence,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "x",
        &serde_json::json!({"action":"preview","text":"RustClaw release notes","dry_run":true}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "x.preview");
    assert_eq!(policy.contract_match, "publishing_preview");
    assert_eq!(policy.required_evidence, vec!["field_value"]);
}

#[test]
fn package_sqlite_and_docker_actions_remain_structured_contract_inputs() {
    let cases = [
        (
            OutputSemanticKind::PackageManagerDetection,
            "package_manager",
            serde_json::json!({"action":"detect","dry_run":true}),
            "package_manager.detect",
            "package_manager_detection",
        ),
        (
            OutputSemanticKind::SqliteTableListing,
            "db_basic",
            serde_json::json!({"action":"list_tables","db_path":"tmp/app.db"}),
            "db_basic.list_tables",
            "sqlite_table_listing",
        ),
        (
            OutputSemanticKind::SqliteSchemaVersion,
            "db_basic",
            serde_json::json!({"action":"schema_version","db_path":"tmp/app.db"}),
            "db_basic.schema_version",
            "sqlite_schema_version",
        ),
        (
            OutputSemanticKind::DockerContainerLifecycle,
            "docker_basic",
            serde_json::json!({"action":"restart","container":"rustclaw_api","dry_run":true}),
            "docker_basic.restart",
            "docker_container_lifecycle",
        ),
    ];

    for (semantic_kind, skill, args, expected_action, expected_contract) in cases {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract {
                semantic_kind,
                requires_content_evidence: true,
                response_shape: OutputResponseShape::Strict,
                ..IntentOutputContract::default()
            }),
            skill,
            &args,
        )
        .unwrap_or_else(|| panic!("policy decision for {expected_action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.original_action_ref, expected_action);
        assert_eq!(policy.replacement_action_ref, None);
        assert_eq!(policy.contract_repair_source, "none");
        assert_eq!(policy.contract_match, expected_contract);
        assert!(
            policy.required_evidence.iter().all(|token| {
                token
                    .chars()
                    .all(|ch| ch.is_ascii_lowercase() || ch == '_' || ch.is_ascii_digit())
            }),
            "required evidence should stay machine-tokenized: {:?}",
            policy.required_evidence
        );
    }
}

#[test]
fn generic_inline_transform_allows_transform_without_locator() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::Strict,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "transform",
        &serde_json::json!({
            "action": "transform_data",
            "data": [{"name":"alpha","score":7},{"name":"beta","score":12}],
            "ops": [{"op":"sort","by":"score","order":"desc"}],
            "output_format": "md_table"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "transform.transform_data");
    assert_eq!(policy.contract_match, "generic_inline_transform");
    assert_eq!(policy.required_evidence, vec!["field_value"]);
}

#[test]
fn content_excerpt_with_summary_contract_has_parsed_final_shape() {
    let output_contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptWithSummary,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "logs/model_io.log".to_string(),
        ..IntentOutputContract::default()
    };

    let shape =
        final_answer_shape_for_output_contract(&output_contract).expect("final answer shape");

    assert_eq!(shape, FinalAnswerShape::ExcerptPlusSummary);
    assert_eq!(shape.as_str(), "excerpt_plus_summary");
    assert_eq!(shape.class(), FinalAnswerShapeClass::GroundedSummary);
}

#[test]
fn content_excerpt_with_summary_allows_supplemental_directory_listing() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptWithSummary,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md"
            .to_string(),
        ..IntentOutputContract::default()
    };

    let list_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"scripts/nl_tests/fixtures/device_local/docs","names_only":true}),
    )
    .expect("list policy");
    assert!(list_policy.is_allowed(), "{list_policy:?}");
    assert_eq!(list_policy.action_key, "fs_basic.list_dir");
    assert_eq!(list_policy.contract_match, "content_excerpt_with_summary");

    let read_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"read_text_range","path":"scripts/nl_tests/fixtures/device_local/docs/release_checklist.md","mode":"head","n":20}),
    )
    .expect("read policy");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert_eq!(read_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(read_policy.contract_match, "content_excerpt_with_summary");
}

#[test]
fn excerpt_kind_judgment_allows_directory_listing_context() {
    for (capability, args, expected_action) in [
        (
            "fs_basic",
            serde_json::json!({"action":"list_dir","path":"docs","names_only":true}),
            "fs_basic.list_dir",
        ),
        (
            "fs_basic",
            serde_json::json!({"action":"count_entries","path":"crates"}),
            "fs_basic.count_entries",
        ),
        (
            "fs_basic",
            serde_json::json!({"action":"find_entries","root":"crates","pattern":"skills"}),
            "fs_basic.find_entries",
        ),
        (
            "fs_basic",
            serde_json::json!({"action":"stat_paths","paths":["crates","crates/skills"]}),
            "fs_basic.stat_paths",
        ),
        (
            "system_basic",
            serde_json::json!({"action":"inventory_dir","path":"docs","names_only":true}),
            "fs_basic.list_dir",
        ),
    ] {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract {
                semantic_kind: OutputSemanticKind::ExcerptKindJudgment,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::Path,
                locator_hint: "docs/release_checklist.md".to_string(),
                ..IntentOutputContract::default()
            }),
            capability,
            &args,
        )
        .expect("policy decision");

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action);
        assert_eq!(policy.contract_match, "excerpt_kind_judgment");
    }
}

#[test]
fn excerpt_kind_judgment_allows_structured_field_context() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ExcerptKindJudgment,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "Cargo.toml|UI/package.json|README.md".to_string(),
            ..IntentOutputContract::default()
        }),
        "config_basic",
        &serde_json::json!({
            "action": "read_field",
            "path": "Cargo.toml",
            "field_path": "package.name"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.read_field");
    assert_eq!(policy.contract_match, "excerpt_kind_judgment");
}

#[test]
fn directory_purpose_summary_allows_log_analyze_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "logs".to_string(),
            ..IntentOutputContract::default()
        }),
        "log_analyze",
        &serde_json::json!({"path":"logs"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "log_analyze");
    assert_eq!(policy.contract_match, "directory_purpose_summary");
}

#[test]
fn directory_purpose_summary_allows_structured_field_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::DirectoryPurposeSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "extract_field",
            "path": "UI/package.json",
            "field_path": "name"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "config_basic.read_field");
    assert_eq!(policy.contract_match, "directory_purpose_summary");
}

#[test]
fn recent_artifacts_judgment_allows_bounded_content_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RecentArtifactsJudgment,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "docs".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "docs/config_basic_contract.md",
            "mode": "head",
            "n": 40
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
    assert_eq!(policy.contract_match, "recent_artifacts_judgment");
}

#[test]
fn workspace_project_summary_allows_structure_and_bounded_content_evidence() {
    let tree_policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({"action":"tree_summary","path":"/workspace","max_depth":1}),
    )
    .expect("policy decision");
    assert!(tree_policy.is_allowed(), "{tree_policy:?}");
    assert_eq!(tree_policy.action_key, "system_basic.tree_summary");
    assert_eq!(tree_policy.contract_match, "workspace_project_summary");
    assert_eq!(tree_policy.evidence_profile, "workspace_user_docs_first");

    let read_policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"read_text_range","path":"README.md","mode":"head","n":80}),
    )
    .expect("policy decision");
    assert!(read_policy.is_allowed(), "{read_policy:?}");
    assert_eq!(read_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(read_policy.contract_match, "workspace_project_summary");
    assert_eq!(read_policy.evidence_profile, "workspace_user_docs_first");

    let list_policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"/workspace","names_only":true}),
    )
    .expect("policy decision");
    assert!(list_policy.is_allowed(), "{list_policy:?}");
    assert_eq!(list_policy.action_key, "fs_basic.list_dir");
    assert_eq!(list_policy.contract_match, "workspace_project_summary");
    assert_eq!(list_policy.evidence_profile, "workspace_user_docs_first");
}

#[test]
fn generic_delivery_allows_directory_listing_for_selection() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "document".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"document","files_only":true}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.list_dir");
    assert_eq!(policy.contract_match, "generic_delivery");
}

#[test]
fn semantic_none_rejects_forbidden_action() {
    let matrix = load_workspace_matrix();
    let contract = matrix
        .match_output_contract(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::None,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        })
        .expect("matched contract");
    let action = ActionRef::parse("run_cmd").expect("action ref");

    assert_eq!(
        contract.action_policy(&action),
        ActionPolicyDecision::RejectedForbidden
    );
}

#[test]
fn action_policy_blocks_disallowed_structured_action_for_semantic_contract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FileNames,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "run_cmd",
        &serde_json::json!({"command":"ls"}),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::RejectedNotAllowed);
    assert_eq!(policy.contract_match, "file_names");
    assert_eq!(policy.required_evidence, vec!["candidates"]);
}

#[test]
fn action_policy_allows_process_snapshot_for_raw_command_output_contract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "process_basic",
        &serde_json::json!({
            "action": "ps",
            "limit": 10,
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "process_basic.ps");
    assert_eq!(policy.contract_match, "raw_command_output");
}

#[test]
fn action_policy_allows_http_observation_for_raw_command_output_contract() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::RawCommandOutput,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "http_basic",
        &serde_json::json!({
            "action": "get",
            "url": "http://127.0.0.1:8787/v1/health",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "http_basic.get");
    assert_eq!(policy.contract_match, "raw_command_output");
    assert!(policy
        .evidence_expression
        .any_of
        .contains(&"command_output".to_string()));
}

#[test]
fn command_output_summary_contract_allows_run_cmd_and_model_language() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::CommandOutputSummary,
            requires_content_evidence: true,
            ..IntentOutputContract::default()
        }),
        "run_cmd",
        &serde_json::json!({"command": "pwd"}),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "run_cmd");
    assert_eq!(policy.contract_match, "command_output_summary");
    assert_eq!(policy.required_evidence, vec!["command_output"]);
    assert!(policy.final_answer_shape_kind.allows_model_language());
}

#[test]
fn command_output_summary_allows_log_analyze_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::CommandOutputSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "logs".to_string(),
            ..IntentOutputContract::default()
        }),
        "log_analyze",
        &serde_json::json!({
            "action": "analyze",
            "path": "logs",
            "limit": 5,
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "log_analyze.analyze");
    assert_eq!(policy.contract_match, "command_output_summary");
    assert!(policy
        .evidence_expression
        .any_of
        .contains(&"field_value".to_string()));
}

#[test]
fn command_output_summary_allows_git_basic_state_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::CommandOutputSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            ..IntentOutputContract::default()
        }),
        "git_basic",
        &serde_json::json!({"action": "status"}),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "git_basic.status");
    assert_eq!(policy.contract_match, "command_output_summary");
    assert_eq!(policy.required_evidence, vec!["command_output"]);
    assert!(policy.final_answer_shape_kind.allows_model_language());
}

#[test]
fn command_output_summary_allows_supplemental_directory_inventory() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "logs".to_string(),
        ..IntentOutputContract::default()
    };

    let list_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({
            "action": "list_dir",
            "path": "logs",
            "files_only": true,
        }),
    )
    .expect("list policy decision");
    assert_eq!(list_policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(list_policy.action_key, "fs_basic.list_dir");
    assert_eq!(list_policy.contract_match, "command_output_summary");

    let find_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({
            "action": "find_entries",
            "path": "logs",
            "max_depth": 2,
        }),
    )
    .expect("find policy decision");
    assert_eq!(find_policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(find_policy.action_key, "fs_basic.find_entries");
    assert_eq!(find_policy.contract_match, "command_output_summary");

    let count_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({
            "action": "count_entries",
            "path": "logs",
        }),
    )
    .expect("count policy decision");
    assert_eq!(count_policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(count_policy.action_key, "fs_basic.count_entries");
    assert_eq!(count_policy.contract_match, "command_output_summary");
    assert!(count_policy
        .evidence_expression
        .any_of
        .contains(&"count".to_string()));
}

#[test]
fn action_policy_allows_safe_file_read_equivalent_for_raw_command_output_contract() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    let fs_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "logs/clawd.log",
            "mode": "tail",
            "n": 20,
        }),
    )
    .expect("fs policy decision");
    assert_eq!(fs_policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(fs_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(fs_policy.contract_match, "raw_command_output");
    assert!(fs_policy
        .evidence_expression
        .any_of
        .contains(&"content_excerpt".to_string()));

    let system_policy = action_policy_for_output_contract(
        Some(&contract),
        "system_basic",
        &serde_json::json!({
            "action": "read_range",
            "path": "logs/clawd.log",
            "mode": "tail",
            "n": 20,
        }),
    )
    .expect("system policy decision");
    assert_eq!(system_policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(system_policy.action_key, "fs_basic.read_text_range");
    assert_eq!(system_policy.contract_match, "raw_command_output");
}

#[test]
fn action_policy_allows_runtime_equivalent_for_virtual_config_validation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.contract_match, "config_validation");
}

#[test]
fn command_output_summary_allows_structured_config_validation_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::CommandOutputSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "configs/config.toml".to_string(),
            ..IntentOutputContract::default()
        }),
        "config_basic",
        &serde_json::json!({
            "action": "validate",
            "path": "configs/config.toml",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.contract_match, "command_output_summary");
}

#[test]
fn command_output_summary_allows_structured_config_field_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::CommandOutputSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "configs/config.toml".to_string(),
            ..IntentOutputContract::default()
        }),
        "config_basic",
        &serde_json::json!({
            "action": "read_field",
            "path": "configs/config.toml",
            "field_path": "llm.selected_vendor",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "config_basic.read_field");
    assert_eq!(policy.contract_match, "command_output_summary");
    assert!(policy
        .evidence_expression
        .any_of
        .contains(&"field_value".to_string()));
}

#[test]
fn service_status_allows_task_control_list_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ServiceStatus,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "task_control",
        &serde_json::json!({
            "action": "list",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "task_control.list");
    assert_eq!(policy.contract_match, "service_status");
}

#[test]
fn command_output_summary_allows_task_control_get_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::CommandOutputSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            ..IntentOutputContract::default()
        }),
        "task_control",
        &serde_json::json!({
            "action": "get",
            "task_id": "00000000-0000-4000-8000-000000000001",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "task_control.get");
    assert_eq!(policy.contract_match, "command_output_summary");
}

#[test]
fn action_policy_allows_runtime_equivalent_for_virtual_config_guard() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "config_edit",
        &serde_json::json!({
            "action": "guard_config",
            "path": "configs/app_config.toml",
            "format": "toml",
        }),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "config_basic.guard_rustclaw_config");
    assert_eq!(policy.contract_match, "config_validation");
}

#[test]
fn config_mutation_contract_allows_plan_apply_validate_and_read_back() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ConfigMutation,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        ..IntentOutputContract::default()
    };

    for action in [
        "plan_config_change",
        "apply_config_change",
        "validate_config",
        "read_back",
    ] {
        let policy = action_policy_for_output_contract(
            Some(&contract),
            "config_edit",
            &serde_json::json!({
                "action": action,
                "path": "configs/config.toml",
                "field_path": "skills.skill_switches.example",
                "value": true,
            }),
        )
        .expect("policy decision");

        assert_eq!(policy.decision, ActionPolicyDecision::Allowed, "{action}");
        assert_eq!(policy.contract_match, "config_mutation");
    }
}

#[test]
fn stable_semantic_action_preferences_live_in_task_contract_matrix() {
    let matrix = load_workspace_matrix();
    let cases = [
        (
            "config_validation",
            OutputSemanticKind::ConfigValidation,
            "config_basic.validate",
        ),
        (
            "config_mutation",
            OutputSemanticKind::ConfigMutation,
            "config_edit.plan_config_change",
        ),
        (
            "config_risk_assessment",
            OutputSemanticKind::ConfigRiskAssessment,
            "config_basic.guard_rustclaw_config",
        ),
        (
            "filesystem_mutation_result",
            OutputSemanticKind::FilesystemMutationResult,
            "fs_basic.write_text",
        ),
        (
            "existence_with_path",
            OutputSemanticKind::ExistenceWithPath,
            "fs_basic.stat_paths",
        ),
        (
            "document_heading",
            OutputSemanticKind::DocumentHeading,
            "fs_basic.read_text_range",
        ),
        (
            "content_presence_check",
            OutputSemanticKind::ContentPresenceCheck,
            "fs_basic.grep_text",
        ),
        (
            "package_manager_detection",
            OutputSemanticKind::PackageManagerDetection,
            "package_manager.detect",
        ),
        (
            "archive_read",
            OutputSemanticKind::ArchiveRead,
            "archive_basic.read",
        ),
        (
            "docker_container_lifecycle",
            OutputSemanticKind::DockerContainerLifecycle,
            "docker_basic",
        ),
    ];

    for (contract_name, semantic_kind, preferred_action) in cases {
        let contract = matrix
            .semantic_contract(semantic_kind)
            .unwrap_or_else(|| panic!("missing contract for {contract_name}"));
        assert!(
            contract
                .preferred_actions
                .iter()
                .any(|action| action == preferred_action),
            "contract `{contract_name}` should prefer `{preferred_action}`, got {:?}",
            contract.preferred_actions
        );
        assert!(
            contract
                .allowed_actions
                .iter()
                .any(|action| action == preferred_action),
            "contract `{contract_name}` should allow `{preferred_action}`, got {:?}",
            contract.allowed_actions
        );
    }
}

#[test]
fn action_policy_skips_unstructured_none_contracts() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract::default()),
        "run_cmd",
        &serde_json::json!({"command":"echo ok"}),
    );

    assert!(policy.is_none());
}

#[test]
fn arg_policy_defers_unresolved_template_targets() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "{{s1.path}}"
        }),
    )
    .expect("arg policy decision");

    assert_eq!(policy.decision, ArgPolicyDecision::DeferredTemplateArg);
    assert!(policy.missing_target_args.is_empty());
    assert_eq!(policy.deferred_target_args, vec!["path"]);
}

#[test]
fn arg_policy_rejects_missing_bound_target_after_resolution() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "start_line": 1,
            "end_line": 20
        }),
    )
    .expect("arg policy decision");

    assert_eq!(policy.decision, ArgPolicyDecision::MissingTargetBinding);
    assert_eq!(policy.missing_target_args, vec!["path"]);
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
}

#[test]
fn arg_policy_allows_concrete_bound_target() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({
            "action": "read_text_range",
            "path": "/tmp/readme.md"
        }),
    )
    .expect("arg policy decision");

    assert!(policy.is_allowed());
    assert!(policy.missing_target_args.is_empty());
    assert!(policy.deferred_target_args.is_empty());
}

#[test]
fn arg_policy_uses_virtual_equivalent_target_groups() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "system_basic",
        &serde_json::json!({
            "action": "validate_structured",
            "path": "configs/config.toml",
            "format": "toml",
        }),
    )
    .expect("arg policy decision");

    assert!(policy.is_allowed());
    assert_eq!(policy.action_key, "config_basic.validate");
    assert_eq!(policy.expected_target_args, vec!["path"]);
}

#[test]
fn arg_policy_uses_virtual_guard_equivalent_target_groups() {
    let policy = arg_policy_decision(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ConfigValidation,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            ..IntentOutputContract::default()
        }),
        "config_edit",
        &serde_json::json!({
            "action": "guard_config",
            "path": "configs/app_config.toml",
        }),
    )
    .expect("arg policy decision");

    assert!(policy.is_allowed());
    assert_eq!(policy.action_key, "config_basic.guard_rustclaw_config");
    assert_eq!(policy.expected_target_args, vec!["path"]);
}

#[test]
fn action_ref_prefers_structured_action_from_args() {
    let action = ActionRef::from_skill_args("fs-basic", &serde_json::json!({"action":"list_dir"}))
        .expect("action ref");

    assert_eq!(action.as_key(), "fs_basic.list_dir");
}

#[test]
fn contract_matrix_references_registered_skills() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

    let unknown = matrix.unknown_matrix_skills(&registry);

    assert!(unknown.is_empty(), "unknown matrix skills: {unknown:?}");
}

#[test]
fn contract_matrix_action_refs_are_declared_in_registry() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

    let unknown = matrix.unknown_matrix_action_refs(&registry);

    assert!(
        unknown.is_empty(),
        "unknown matrix action refs: {unknown:?}"
    );
}

#[test]
fn contract_matrix_action_refs_have_registry_schemas() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    let mut missing = Vec::new();

    for token in matrix.all_action_tokens() {
        let Some(action_ref) = ActionRef::parse(&token) else {
            continue;
        };
        let Some(skill) = registry.resolve_canonical(&action_ref.skill) else {
            continue;
        };
        let Some(manifest) = registry.manifest(skill) else {
            continue;
        };
        if manifest.input_schema.is_none() {
            missing.push(format!("{}.input_schema", action_ref.skill));
        }
        if manifest.output_schema.is_none() {
            missing.push(format!("{}.output_schema", action_ref.skill));
        }
    }
    missing.sort();
    missing.dedup();

    assert!(missing.is_empty(), "missing registry schemas: {missing:?}");
}

#[test]
fn legacy_virtual_tool_canonicalizations_are_covered_by_matrix_action_policy() {
    let cases = [
        (
            OutputSemanticKind::ExistenceWithPath,
            "system_basic",
            json!({"action":"path_batch_facts", "paths":["README.md"]}),
            "fs_basic.stat_paths",
        ),
        (
            OutputSemanticKind::FileNames,
            "system_basic",
            json!({"action":"inventory_dir", "path":"scripts"}),
            "fs_basic.list_dir",
        ),
        (
            OutputSemanticKind::ScalarCount,
            "system_basic",
            json!({"action":"count_inventory", "path":"scripts"}),
            "fs_basic.count_entries",
        ),
        (
            OutputSemanticKind::ContentExcerptSummary,
            "system_basic",
            json!({"action":"read_range", "path":"README.md", "mode":"head", "n":5}),
            "fs_basic.read_text_range",
        ),
        (
            OutputSemanticKind::QuantityComparison,
            "system_basic",
            json!({"action":"compare_paths", "paths":["Cargo.toml", "README.md"]}),
            "fs_basic.compare_paths",
        ),
        (
            OutputSemanticKind::QuantityComparison,
            "system_basic",
            json!({"action":"count_inventory", "path":"target", "recursive":true}),
            "fs_basic.count_entries",
        ),
        (
            OutputSemanticKind::ConfigValidation,
            "system_basic",
            json!({"action":"validate_structured", "path":"configs/config.toml", "format":"toml"}),
            "config_basic.validate",
        ),
        (
            OutputSemanticKind::FilePaths,
            "fs_search",
            json!({"action":"find_ext", "root":"scripts", "ext":"sh"}),
            "fs_basic.find_entries",
        ),
        (
            OutputSemanticKind::ContentPresenceCheck,
            "fs_search",
            json!({"action":"grep_text", "root":".", "query":"FirstLayerDecision"}),
            "fs_basic.grep_text",
        ),
        (
            OutputSemanticKind::FilePaths,
            "fs_search",
            json!({"action":"grep_text", "root":".", "query":"FirstLayerDecision"}),
            "fs_basic.grep_text",
        ),
        (
            OutputSemanticKind::FileNames,
            "fs_search",
            json!({"action":"grep_text", "root":".", "query":"FirstLayerDecision"}),
            "fs_basic.grep_text",
        ),
        (
            OutputSemanticKind::ConfigRiskAssessment,
            "config_guard",
            json!({"path":"configs/config.toml"}),
            "config_guard",
        ),
        (
            OutputSemanticKind::ContentExcerptSummary,
            "read_file",
            json!({"path":"README.md"}),
            "fs_basic.read_text_range",
        ),
        (
            OutputSemanticKind::FileNames,
            "list_dir",
            json!({"path":"scripts"}),
            "fs_basic.list_dir",
        ),
        (
            OutputSemanticKind::GeneratedFileDelivery,
            "write_file",
            json!({"path":"tmp/out.txt", "content":"ok"}),
            "fs_basic.write_text",
        ),
    ];

    for (semantic_kind, skill, args, expected_action_key) in cases {
        let route = IntentOutputContract {
            semantic_kind,
            ..IntentOutputContract::default()
        };
        let policy = action_policy_for_output_contract(Some(&route), skill, &args)
            .unwrap_or_else(|| panic!("missing policy for {skill} -> {expected_action_key}"));
        assert!(
            policy.is_allowed(),
            "legacy {skill} should be allowed as {expected_action_key}, got {:?}",
            policy.decision
        );
        assert_eq!(policy.action_key, expected_action_key);
    }
}

#[test]
fn allowed_action_keeps_original_ref_without_preferred_replacement() {
    let route = IntentOutputContract {
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        ..IntentOutputContract::default()
    };
    let policy = action_policy_for_output_contract(
        Some(&route),
        "fs_basic",
        &json!({"action":"stat_paths", "paths":["README.md"]}),
    )
    .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.action_key, "fs_basic.stat_paths");
    assert_eq!(policy.original_action_ref, "fs_basic.stat_paths");
    assert_eq!(policy.replacement_action_ref, None);
    assert_eq!(policy.contract_repair_source, "none");
    assert_eq!(policy.preferred_replacement_reason_code, None);
}

#[test]
fn legacy_canonicalization_records_original_and_replacement_refs() {
    let route = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..IntentOutputContract::default()
    };
    let policy =
        action_policy_for_output_contract(Some(&route), "read_file", &json!({"path":"README.md"}))
            .expect("policy decision");

    assert_eq!(policy.decision, ActionPolicyDecision::Allowed);
    assert_eq!(policy.original_action_ref, "read_file");
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
    assert_eq!(
        policy.replacement_action_ref.as_deref(),
        Some("fs_basic.read_text_range")
    );
    assert_eq!(
        policy.contract_repair_source,
        "legacy_tool_canonicalization"
    );
    assert_eq!(
        policy.preferred_replacement_reason_code.as_deref(),
        Some("legacy_tool_canonical_action_allowed")
    );
}

#[test]
fn bundled_matrix_observation_sources_have_extractor_registry_refs() {
    let matrix = load_workspace_matrix();
    let mut missing = Vec::new();

    for (name, contract) in &matrix.contracts {
        for extractor in contract.observation_extractors() {
            if crate::task_journal::evidence_extractor_registry_trace(
                &extractor.source,
                &extractor.extractor_kind,
            )
            .is_none()
            {
                missing.push(format!(
                    "contract `{name}` observation_source `{}` extractor_kind `{}`",
                    extractor.source, extractor.extractor_kind
                ));
            }
        }
    }
    for profile in &matrix.generic_profiles {
        for extractor in profile.observation_extractors() {
            if crate::task_journal::evidence_extractor_registry_trace(
                &extractor.source,
                &extractor.extractor_kind,
            )
            .is_none()
            {
                missing.push(format!(
                    "generic profile `{}` observation_source `{}` extractor_kind `{}`",
                    profile.name, extractor.source, extractor.extractor_kind
                ));
            }
        }
    }

    assert!(
        missing.is_empty(),
        "missing observation extractor registry refs: {missing:?}"
    );
}

#[test]
fn contract_matrix_external_observation_sources_are_admitted() {
    let matrix = load_workspace_matrix();
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");

    let errors = matrix.external_observation_admission_errors(&registry);

    assert!(
        errors.is_empty(),
        "external observation sources need matrix admission: {errors:?}"
    );
}

#[test]
fn external_observation_source_requires_matrix_admission() {
    let matrix = ContractMatrix {
        generic_profiles: vec![GenericProfile {
            name: "external_scalar".to_string(),
            required_evidence: vec!["field_value".to_string()],
            observation_sources: vec!["demo_skill.ping".to_string()],
            ..Default::default()
        }],
        ..Default::default()
    };
    let not_admitted = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = false, declared_actions = [], evidence_sources = [], required_extra_fields = [], extractor_kind = "structured_json", admission_version = "external-v1" }
"#,
    );

    let errors = matrix.external_observation_admission_errors(&not_admitted);

    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("demo_skill.ping"));
    assert!(errors[0].contains("matrix_admission.eligible=true"));

    let action_mismatch = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = true, declared_actions = ["other"], evidence_sources = ["structured_json"], required_extra_fields = ["extra.message"], extractor_kind = "structured_json", admission_version = "external-v1" }
"#,
    );
    let errors = matrix.external_observation_admission_errors(&action_mismatch);
    assert_eq!(errors.len(), 1);

    let admitted = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = true, declared_actions = ["ping"], evidence_sources = ["structured_json"], required_extra_fields = ["extra.message"], extractor_kind = "structured_json", admission_version = "external-v1" }
"#,
    );
    assert!(matrix
        .external_observation_admission_errors(&admitted)
        .is_empty());

    let text_legacy_matrix = ContractMatrix {
        generic_profiles: vec![GenericProfile {
            name: "external_scalar".to_string(),
            required_evidence: vec!["field_value".to_string()],
            observation_sources: vec!["demo_skill.ping".to_string()],
            observation_extractors: vec![ObservationExtractor {
                source: "demo_skill.ping".to_string(),
                extractor_kind: "text_legacy".to_string(),
            }],
            ..Default::default()
        }],
        ..Default::default()
    };
    let errors = text_legacy_matrix.external_observation_admission_errors(&admitted);
    assert_eq!(errors.len(), 1);
    assert!(errors[0].contains("text_legacy extractor"));

    let text_legacy_admitted = load_registry_from_text(
        r#"
[[skills]]
name = "demo_skill"
kind = "runner"
matrix_admission = { eligible = true, declared_actions = ["ping"], evidence_sources = ["text_legacy"], required_extra_fields = ["extra.message"], extractor_kind = "text_legacy", admission_version = "external-v1" }
"#,
    );
    assert!(text_legacy_matrix
        .external_observation_admission_errors(&text_legacy_admitted)
        .is_empty());
}

#[test]
fn contract_matrix_main_contracts_do_not_reference_backing_tools() {
    let matrix = load_workspace_matrix();

    let backing_refs = matrix.backing_tool_refs_in_main_contracts();

    assert!(
        backing_refs.is_empty(),
        "matrix should use planner-facing actions, not backing tools: {backing_refs:?}"
    );
}

#[test]
fn registry_action_index_contains_skill_level_and_action_level_refs() {
    let registry_path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    let refs = available_action_refs_from_registry(&registry);

    assert!(refs.contains("fs_basic"));
    assert!(refs.contains("fs_basic.list_dir"));
    assert!(refs.contains("archive_basic.pack"));
}

#[test]
fn matrix_generated_cases_cover_at_least_100_unique_contract_paths() {
    let matrix = load_workspace_matrix();
    let cases = generated_contract_cases(&matrix, 100);

    let mut ids = BTreeSet::new();
    let mut semantic_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut generic_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut decisions = BTreeSet::new();

    for case in &cases {
        assert!(
            ids.insert(case.id.as_str()),
            "duplicate case id: {}",
            case.id
        );

        match &case.matched {
            GeneratedContractMatch::Semantic(kind) => {
                *semantic_counts.entry(kind.as_str()).or_default() += 1;
            }
            GeneratedContractMatch::Generic(name) => {
                *generic_counts.entry(name.clone()).or_default() += 1;
            }
        }

        let matched = matched_for_generated_case(&matrix, case);
        assert_eq!(
            case.expected_required_evidence,
            matched.required_evidence(),
            "required evidence drift in generated case {}",
            case.id
        );
        assert_eq!(
            case.expected_final_answer_shape,
            matched.final_answer_shape(),
            "final answer shape drift in generated case {}",
            case.id
        );

        if let Some(action) = &case.action {
            let expected = case
                .expected_decision
                .expect("action case has expected decision");
            let actual = matched.action_policy(action);
            assert_eq!(
                actual, expected,
                "action decision drift in generated case {}",
                case.id
            );
            decisions.insert(actual.as_str());
        }
    }

    assert!(
        OutputSemanticKind::ALL
            .iter()
            .all(|kind| semantic_counts.contains_key(kind.as_str())),
        "generated cases must cover every semantic kind"
    );
    assert!(
        matrix
            .generic_profiles
            .iter()
            .all(|profile| generic_counts.contains_key(&profile.name)),
        "generated cases must cover every generic profile"
    );
    assert!(
        decisions.contains(ActionPolicyDecision::Allowed.as_str()),
        "generated cases must include allowed action decisions"
    );
    assert!(
        decisions.contains(ActionPolicyDecision::RejectedForbidden.as_str()),
        "generated cases must include forbidden action decisions"
    );
    assert!(
        decisions.contains(ActionPolicyDecision::RejectedNotAllowed.as_str()),
        "generated cases must include not-allowed action decisions"
    );
}
