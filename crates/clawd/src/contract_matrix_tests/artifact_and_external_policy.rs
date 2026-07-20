use super::*;

#[test]
fn generated_file_delivery_allows_parent_directory_creation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"make_dir","path":"tmp"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.make_dir");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_path_report_allows_command_then_write_and_returns_single_path_shape() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::GeneratedFilePathReport,
        requires_content_evidence: true,
        response_shape: OutputResponseShape::Scalar,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "pwd_line_abs.txt".to_string(),
        ..IntentOutputContract::default()
    };
    let run_policy =
        action_policy_for_output_contract(Some(&contract), "run_cmd", &serde_json::json!({}))
            .expect("run policy decision");
    assert!(run_policy.is_allowed(), "{run_policy:?}");
    assert_eq!(run_policy.action_key, "run_cmd");
    assert_eq!(run_policy.contract_match, "generated_file_path_report");
    assert_eq!(run_policy.final_answer_shape, "single_path");

    let write_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"write_text","path":"pwd_line_abs.txt","content":"x"}),
    )
    .expect("write policy decision");
    assert!(write_policy.is_allowed(), "{write_policy:?}");
    assert_eq!(write_policy.action_key, "fs_basic.write_text");
    assert_eq!(write_policy.contract_match, "generated_file_path_report");
    assert_eq!(write_policy.final_answer_shape, "single_path");
}

#[test]
fn generated_file_path_report_allows_image_generation_path_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFilePathReport,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::Scalar,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "document/skill_generate_smoke.png".to_string(),
            ..IntentOutputContract::default()
        }),
        "image_generate",
        &serde_json::json!({
            "prompt": "minimal RustClaw smoke test card",
            "output_path": "document/skill_generate_smoke.png"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "image_generate");
    assert_eq!(policy.contract_match, "generated_file_path_report");
    assert_eq!(policy.final_answer_shape, "single_path");
}

#[test]
fn generated_file_path_report_allows_audio_synthesis_path_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFilePathReport,
            requires_content_evidence: true,
            response_shape: OutputResponseShape::Scalar,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "document/skill_audio_smoke.mp3".to_string(),
            ..IntentOutputContract::default()
        }),
        "audio_synthesize",
        &serde_json::json!({
            "text": "RustClaw skill test passed",
            "output_path": "document/skill_audio_smoke.mp3"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "audio_synthesize");
    assert_eq!(policy.contract_match, "generated_file_path_report");
    assert_eq!(policy.final_answer_shape, "single_path");
}

#[test]
fn filesystem_mutation_result_allows_directory_creation_status() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "document/nl_skill_tmp".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"make_dir","path":"document/nl_skill_tmp"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.make_dir");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
    assert_eq!(policy.final_answer_shape, "lifecycle_result");
}

#[test]
fn filesystem_mutation_result_allows_path_removal_status() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "tmp/nl_skill_tmp".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"remove_path","path":"tmp/nl_skill_tmp"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.remove_path");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
    assert_eq!(policy.final_answer_shape, "lifecycle_result");
}

#[test]
fn filesystem_mutation_result_allows_readback_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "tmp/nl_skill_tmp/note.txt".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"read_text_range","path":"tmp/nl_skill_tmp/note.txt","mode":"head","n":20}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
    assert_eq!(policy.final_answer_shape, "lifecycle_result");
}

#[test]
fn generated_file_delivery_allows_existing_file_path_facts() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"stat_paths","paths":["README.md"]}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.stat_paths");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_existing_file_content_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "README.md".to_string(),
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"read_text_range","path":"README.md","mode":"head","n":30}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.read_text_range");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_directory_inventory_for_existing_selection() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":"document","names_only":true}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "fs_basic.list_dir");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_runtime_command_observation() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "pwd_report.txt".to_string(),
            ..IntentOutputContract::default()
        }),
        "run_cmd",
        &serde_json::json!({"command":"pwd > pwd_report.txt && cat pwd_report.txt"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "run_cmd");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_audio_synthesis_file_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "audio_synthesize",
        &serde_json::json!({
            "text": "RustClaw skill test passed",
            "output_path": "document/skill_audio_smoke.mp3"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "audio_synthesize");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_image_generation_file_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "image_generate",
        &serde_json::json!({
            "prompt": "minimal RustClaw smoke test card",
            "output_path": "document/skill_generate_smoke.png"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "image_generate");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn generated_file_delivery_allows_image_edit_file_output() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
            delivery_required: true,
            delivery_intent: OutputDeliveryIntent::FileSingle,
            response_shape: OutputResponseShape::FileToken,
            ..IntentOutputContract::default()
        }),
        "image_edit",
        &serde_json::json!({
            "action": "restyle",
            "instruction": "pixel art style",
            "image": {"url": "https://example.test/source.png"},
            "output_path": "document/rust_icon_pixel.png"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "image_edit.restyle");
    assert_eq!(policy.contract_match, "generated_file_delivery");
}

#[test]
fn content_excerpt_summary_allows_log_analyze_for_log_paths() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "logs".to_string(),
            ..IntentOutputContract::default()
        }),
        "log_analyze",
        &serde_json::json!({"action":"analyze","path":"logs"}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "log_analyze.analyze");
    assert_eq!(policy.contract_match, "content_excerpt_summary");
}

#[test]
fn content_excerpt_summary_allows_inline_transform_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "docs/service_notes.md".to_string(),
            response_shape: OutputResponseShape::Free,
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
    assert_eq!(policy.contract_match, "content_excerpt_summary");
}

#[test]
fn content_excerpt_summary_allows_health_check_field_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::ContentExcerptSummary,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::None,
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "health_check",
        &serde_json::json!({}),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "health_check");
    assert_eq!(policy.contract_match, "content_excerpt_summary");
}

#[test]
fn content_excerpt_summary_allows_structured_field_evidence() {
    for (action, expected_action_key) in [
        (
            serde_json::json!({
                "action": "read_field",
                "path": "package.json",
                "field_path": "name"
            }),
            "config_basic.read_field",
        ),
        (
            serde_json::json!({
                "action": "read_fields",
                "path": "Cargo.toml",
                "field_paths": ["package.name", "workspace.package.version"]
            }),
            "config_basic.read_fields",
        ),
    ] {
        let policy = action_policy_for_output_contract(
            Some(&IntentOutputContract {
                semantic_kind: OutputSemanticKind::ContentExcerptSummary,
                requires_content_evidence: true,
                locator_kind: OutputLocatorKind::Path,
                locator_hint: "package.json".to_string(),
                response_shape: OutputResponseShape::Scalar,
                ..IntentOutputContract::default()
            }),
            "config_basic",
            &action,
        )
        .expect("policy decision");

        assert!(policy.is_allowed(), "{policy:?}");
        assert_eq!(policy.action_key, expected_action_key);
        assert_eq!(policy.contract_match, "content_excerpt_summary");
    }
}

#[test]
fn filesystem_mutation_result_allows_archive_pack_path_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::Path,
            locator_hint: "tmp/nl_archive_case_en.zip".to_string(),
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "archive_basic",
        &serde_json::json!({
            "action": "pack",
            "source": "scripts/skill_calls",
            "archive": "tmp/nl_archive_case_en.zip",
            "format": "zip"
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "archive_basic.pack");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
}

#[test]
fn filesystem_mutation_result_allows_kb_ingest_path_evidence() {
    let policy = action_policy_for_output_contract(
        Some(&IntentOutputContract {
            semantic_kind: OutputSemanticKind::FilesystemMutationResult,
            requires_content_evidence: true,
            locator_kind: OutputLocatorKind::CurrentWorkspace,
            locator_hint: "README.md".to_string(),
            response_shape: OutputResponseShape::OneSentence,
            ..IntentOutputContract::default()
        }),
        "kb",
        &serde_json::json!({
            "action": "ingest",
            "namespace": "demo_docs_nl",
            "paths": ["/home/guagua/rustclaw/README.md"]
        }),
    )
    .expect("policy decision");

    assert!(policy.is_allowed(), "{policy:?}");
    assert_eq!(policy.action_key, "kb.ingest");
    assert_eq!(policy.contract_match, "filesystem_mutation_result");
}

#[test]
fn filesystem_mutation_result_allows_kb_followup_observation_actions() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::FilesystemMutationResult,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string(),
        response_shape: OutputResponseShape::OneSentence,
        ..IntentOutputContract::default()
    };

    for (action, args) in [
        (
            "search",
            serde_json::json!({
                "action": "search",
                "namespace": "demo_docs_nl",
                "query": "service status"
            }),
        ),
        (
            "stats",
            serde_json::json!({
                "action": "stats",
                "namespace": "demo_docs_nl"
            }),
        ),
    ] {
        let policy = action_policy_for_output_contract(Some(&contract), "kb", &args)
            .unwrap_or_else(|| panic!("missing policy decision for kb.{action}"));

        assert!(policy.is_allowed(), "{policy:?}");
        assert!(policy.action_matches_preferred(), "{policy:?}");
        assert_eq!(policy.action_key, format!("kb.{action}"));
        assert_eq!(policy.contract_match, "filesystem_mutation_result");
    }
}

#[test]
fn content_excerpt_summary_allows_supplemental_directory_inventory() {
    let contract = IntentOutputContract {
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "UI/package.json".to_string(),
        response_shape: OutputResponseShape::OneSentence,
        ..IntentOutputContract::default()
    };

    let list_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"list_dir","path":".","names_only":true}),
    )
    .expect("list policy");
    assert!(list_policy.is_allowed(), "{list_policy:?}");
    assert_eq!(list_policy.action_key, "fs_basic.list_dir");
    assert_eq!(list_policy.contract_match, "content_excerpt_summary");

    let find_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"find_entries","root":".","pattern":"package.json"}),
    )
    .expect("find policy");
    assert!(find_policy.is_allowed(), "{find_policy:?}");
    assert_eq!(find_policy.action_key, "fs_basic.find_entries");
    assert_eq!(find_policy.contract_match, "content_excerpt_summary");

    let count_policy = action_policy_for_output_contract(
        Some(&contract),
        "fs_basic",
        &serde_json::json!({"action":"count_entries","path":"crates"}),
    )
    .expect("count policy");
    assert!(count_policy.is_allowed(), "{count_policy:?}");
    assert_eq!(count_policy.action_key, "fs_basic.count_entries");
    assert_eq!(count_policy.contract_match, "content_excerpt_summary");
}
