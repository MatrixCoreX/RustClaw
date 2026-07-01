// Archive contract repair tests for intent_router.

use crate::FirstLayerDecision;

use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind,
};

#[test]
fn archive_unpack_missing_archive_locator_forces_clarify_even_with_destination_path() {
    let req = "extract the referenced archive into /tmp/unpack_dest and report the result";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/unpack_dest".to_string(),
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_archive_unpack_missing_archive_locator_clarify(
        &mut contract,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        reason,
        Some("archive_unpack_missing_archive_locator_clarify")
    );
    assert!(needs_clarify);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert!(contract.requires_content_evidence);
}

#[test]
fn archive_unpack_missing_archive_locator_allows_structural_archive_pair() {
    let req = "extract tmp/test_bundle.zip into /tmp/unpack_dest and report the result";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "tmp/test_bundle.zip | /tmp/unpack_dest".to_string(),
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_archive_unpack_missing_archive_locator_clarify(
        &mut contract,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(
        contract.locator_hint,
        "tmp/test_bundle.zip | /tmp/unpack_dest"
    );
}

#[test]
fn archive_pack_pair_repairs_generated_file_delivery_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/docs 打包成 tmp/contract_matrix_docs_bundle.zip，并告诉我生成路径。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/docs".to_string(),
            "tmp/contract_matrix_docs_bundle.zip".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_pack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchivePack);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs | tmp/contract_matrix_docs_bundle.zip"
    );
}

#[test]
fn archive_pair_does_not_treat_generated_delivery_semantic_kind_as_delivery_contract() {
    let req =
        "把 scripts/nl_tests/fixtures/device_local/docs 打包成 tmp/contract_matrix_docs_bundle.zip";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("archive_pack_pair_contract_repair"));
    assert_ne!(reason, Some("archive_unpack_pair_contract_repair"));
    assert!(
        !matches!(
            contract.semantic_kind,
            OutputSemanticKind::ArchivePack | OutputSemanticKind::ArchiveUnpack
        ),
        "semantic enum alone must not promote an archive pair operation"
    );
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
}

#[test]
fn archive_pack_pair_repairs_scalar_path_only_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/docs 打包成 tmp/contract_matrix_docs_bundle.zip，并告诉我生成路径。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_pack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchivePack);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/docs | tmp/contract_matrix_docs_bundle.zip"
    );
}

#[test]
fn archive_unpack_pair_repairs_generated_file_delivery_contract() {
    let req = "把 tmp/contract_matrix_docs_bundle.zip 解压到 tmp/contract_matrix_unpacked，并告诉我结果。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "tmp/contract_matrix_docs_bundle.zip".to_string(),
            "tmp/contract_matrix_unpacked".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "tmp/contract_matrix_docs_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_unpack_pair_repairs_generic_path_content_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
            "tmp/contract_matrix_unpacked".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_unpack_pair_repairs_filesystem_mutation_drift_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::FilesystemMutationResult,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "tmp/contract_matrix_unpacked".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_unpack_pair_repairs_content_excerpt_drift_contract() {
    let req = "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_unpack_pair_repairs_policy_suffix_contract() {
    let req = concat!(
        "把 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 解压到 tmp/contract_matrix_unpacked，并简短说明结果。",
        "\n[CONTRACT_TEST_HINT]\n",
        "candidate_wrong_action_ref=fs_basic.write_text\n",
        "policy_expectation=runtime_must_reject_or_replace_disallowed_action\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert_eq!(
        surface.locator_target_pair,
        Some((
            "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
            "tmp/contract_matrix_unpacked".to_string()
        ))
    );
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | tmp/contract_matrix_unpacked"
    );
}

#[test]
fn archive_read_member_repairs_content_excerpt_drift_contract() {
    let req = concat!(
        "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 里的 notes.txt 内容片段，并简短总结。",
        "\n[CONTRACT_TEST_HINT]\n",
        "contract_id=archive_read\n",
        "semantic_kind=archive_read\n",
        "preferred_action_ref=archive_basic.read\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert!(surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate == "test_bundle.zip"));
    assert!(surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate == "notes.txt"));
    assert!(!surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate.contains("archive_basic.read")));

    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt"
    );
}

#[test]
fn archive_read_member_repair_preserves_archive_sqlite_compound_contract() {
    let req = concat!(
        "列出 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 的成员并读取 notes.txt；",
        "再查看 scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite 的表列表。"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert!(surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate.ends_with(".zip")));
    assert!(surface
        .filename_candidates
        .iter()
        .any(|candidate| candidate.ends_with(".sqlite")));

    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | scripts/nl_tests/fixtures/device_local/data/test_contract.sqlite".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
    );
}

#[test]
fn archive_read_member_pair_is_not_treated_as_unpack_destination() {
    let req = "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 中 notes.txt 的内容片段并简短总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt"
    );
}

#[test]
fn archive_read_member_repairs_archive_unpack_drift_contract() {
    let req = "从压缩包 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 中提取 notes.txt 文件内容并简短总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | notes.txt"
    );
}

#[test]
fn archive_list_single_archive_repairs_archive_unpack_drift_contract() {
    let req = "查看 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 里面有哪些文件，只列文件名";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_list_single_archive_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveList);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
    );
}

#[test]
fn archive_list_single_archive_repairs_file_names_drift_contract() {
    let req = "查看 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 里面有哪些文件，只列文件名";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::FileNames,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_list_single_archive_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveList);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
    );
    assert_eq!(
        contract.self_extension.list_selector.target_kind,
        crate::OutputScalarCountTargetKind::File
    );
}

#[test]
fn archive_list_single_archive_repairs_directory_entry_groups_without_locator_hint() {
    let req = "查看 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 里面有哪些文件，只列文件名";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::DirectoryEntryGroups,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_list_single_archive_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveList);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "test_bundle.zip");
    assert_eq!(
        contract.self_extension.list_selector.target_kind,
        crate::OutputScalarCountTargetKind::File
    );
}

#[test]
fn archive_read_nested_member_path_is_not_unpack_destination() {
    let req = "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 中 nested/config.ini 的内容片段并简短总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveRead);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip | nested/config.ini"
    );
}

#[test]
fn archive_read_member_repair_requires_member_candidate() {
    let req =
        "读取 scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip 内容片段，并简短总结。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("archive_read_member_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/tmp/test_bundle.zip"
    );
}

#[test]
fn archive_pair_does_not_repair_plain_observation_contract() {
    let req =
        "比较 tmp/contract_matrix_docs_bundle.zip 和 tmp/contract_matrix_unpacked 的大小差异。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    assert!(surface.locator_target_pair.is_some());
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("archive_unpack_pair_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
}
