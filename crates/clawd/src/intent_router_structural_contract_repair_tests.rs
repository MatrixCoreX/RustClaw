// Structural contract repair tests for intent_router.

use crate::FirstLayerDecision;

use super::test_support::make_temp_workspace_with_child;
use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind,
};

#[test]
fn structural_contract_repair_routes_file_field_scalar_to_evidence() {
    let req = "读取 Cargo.toml 的 package.name，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let reason = super::apply_current_turn_structural_contract_repair(
        "scalar_path_only",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert!(
        matches!(
            reason,
            Some("structured_file_scalar_repair") | Some("scalar_locator_requires_evidence")
        ),
        "unexpected repair reason: {reason:?}"
    );
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "Cargo.toml");
}

#[test]
fn structural_contract_repair_file_paths_missing_file_locator_uses_parent_dir() {
    let req = "Read plan/definitely_missing_20260511.md; if it is missing, search plan for matching md files and return paths.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    assert!(workspace_root.join("plan").is_dir());
    assert!(!workspace_root
        .join("plan/definitely_missing_20260511.md")
        .exists());
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::FilePaths,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "plan/definitely_missing_20260511.md".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "existence_with_path",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("file_paths_missing_file_locator_parent_dir_repair")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FilePaths);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "plan");
}

#[test]
fn structural_contract_repair_promotes_mixed_existence_and_content_summary() {
    let req = "Check /home/guagua/rustclaw/not_real_20260511, then use README.md to write one RustClaw project sentence.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/home/guagua/rustclaw/not_real_20260511".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("existence_with_path_mixed_locator_summary_repair")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPathSummary
    );
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_contract_repair_promotes_strict_mixed_existence_and_content_summary() {
    let req = "先检查一个不存在的路径 /home/guagua/rustclaw/not_real_20260511，然后基于真实 README.md 写一句 RustClaw 项目说明";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/home/guagua/rustclaw/not_real_20260511".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "existence_with_path",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("existence_with_path_mixed_locator_summary_repair")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPathSummary
    );
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_contract_repair_keeps_single_existence_path_verdict() {
    let req = "Check /home/guagua/rustclaw/README.md and answer in one sentence whether it exists.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/home/guagua/rustclaw/README.md".to_string(),
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "existence_with_path",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("semantic_contract_requires_evidence"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
}

#[test]
fn structural_contract_repair_preserves_directory_scoped_scalar_path_lookup() {
    let req =
        "In scripts/nl_tests/fixtures/locator_smart/case_only, where's report.md? only the path";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_ne!(reason, Some("structured_file_scalar_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
}

#[test]
fn structural_contract_repair_keeps_workspace_summary_on_workspace_root_name() {
    let req = "把 RustClaw 当成当前项目来介绍，先查证 README 和 Cargo.toml";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        locator_hint: "RustClaw".to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let workspace_root = std::path::Path::new("/tmp/rustclaw");
    let _ = super::apply_current_turn_structural_contract_repair(
        "workspace_project_summary",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "RustClaw");
}

#[test]
fn structural_contract_repair_preserves_chat_workspace_name_without_evidence() {
    let req = "用一句话介绍 RustClaw 是什么，不要查询文件";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: false,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/tmp/rustclaw"),
        "RustClaw 是一个面向自然语言自动化的本地 agent 项目。",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn structural_contract_repair_preserves_file_path_only_delivery() {
    let req = "Run pwd, write one short line based on it into pwd_line.txt, and output only the file path.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "pwd_line.txt".to_string(),
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("scalar_locator_requires_evidence"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "pwd_line.txt");
}

#[test]
fn structural_contract_repair_promotes_file_token_delivery_to_generated_artifact() {
    let req =
        "创建一个文本文件到 tmp/对抗测试_笔记.txt，内容是「adversarial v1」，然后把文件发给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "generated_file_delivery",
        &mut contract,
        req,
        &surface,
        std::path::Path::new("/workspace"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("file_token_delivery_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_contract_repair_keeps_filename_delivery_out_of_generated_artifact() {
    let req = "把 definitely_missing_named_file_golden_001.txt 发给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Filename,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
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

    assert_eq!(reason, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert!(contract.locator_hint.is_empty());
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
}

#[test]
fn structural_contract_repair_downgrades_filename_only_generated_delivery_to_existing_file() {
    let workspace = make_temp_workspace_with_child("existing_generated_delivery", "docs");
    std::fs::write(
        workspace.join("existing_named_file_golden_001.txt"),
        "existing file",
    )
    .expect("write existing file");
    let req = "把 existing_named_file_golden_001.txt 发给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "generated_file_delivery",
        &mut contract,
        req,
        &surface,
        &workspace,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("generated_file_delivery_filename_only_existing_target_repair")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "existing_named_file_golden_001.txt");
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_contract_repair_keeps_new_filename_generated_delivery() {
    let workspace = make_temp_workspace_with_child("new_generated_delivery", "docs");
    let req = "Run pwd first, save one short line to worker_line_explicit.txt, then tell me the saved path.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_hint: "worker_line_explicit.txt".to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "generated_file_delivery",
        &mut contract,
        req,
        &surface,
        &workspace,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("semantic_contract_requires_evidence"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "worker_line_explicit.txt");
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
}

#[test]
fn structural_contract_repair_converts_existing_generated_delivery_with_counted_summary() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let target = workspace_root.join("README.md");
    assert!(target.is_file());
    let req = format!(
        "把 {path} 发给我，并用一句话说明它主要是做什么的",
        path = target.display()
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_hint: target.display().to_string(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        &req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("generated_file_delivery_existing_content_summary_repair")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentExcerptWithSummary
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.exact_sentence_count, Some(1));
    assert!(contract.requires_content_evidence);
}

#[test]
fn semantic_contract_repair_ignores_invented_answer_candidate_for_observation() {
    let req = "检查仓库里有没有 rustclaw.service，只回答有或没有，并给出路径";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "existence_with_path",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "没有 (路径未找到)",
        None,
        None,
    );

    assert_eq!(reason, Some("semantic_contract_requires_evidence"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "rustclaw.service");
}

#[test]
fn semantic_contract_repair_promotes_empty_path_locator_for_multi_path_facts() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new("/workspace");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_hint: String::new(),
        self_extension: crate::SelfExtensionContract::default(),
    };

    super::apply_current_turn_structural_contract_repair(
        "existence_with_path",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "/workspace");
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
}

#[test]
fn semantic_contract_repair_replaces_combined_path_hint_for_multi_path_facts() {
    let req = "Inspecte ces chemins: scripts/nl_tests/fixtures/device_local/package.json et scripts/nl_tests/fixtures/device_local/nope.json; indique existence et type.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new("/workspace");
    let mut contract = IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Strict,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: OutputSemanticKind::ExistenceWithPath,
            locator_hint: "scripts/nl_tests/fixtures/device_local/package.json, scripts/nl_tests/fixtures/device_local/nope.json".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        };

    super::apply_current_turn_structural_contract_repair(
        "existence_with_path",
        &mut contract,
        req,
        &surface,
        workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "/workspace");
}

#[test]
fn scalar_file_contract_repair_ignores_invented_answer_candidate() {
    let req = "读取 package.json 里的 name 字段，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "rustclaw",
        None,
        None,
    );

    assert_eq!(reason, Some("scalar_locator_requires_evidence"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn dotted_structured_field_repair_overrides_structured_keys_contract() {
    let req =
        "读取 scripts/nl_tests/fixtures/device_local/configs/app_config.toml 中 app.name，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/configs/app_config.toml".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "structured_keys",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_field_selector_requires_scalar_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml"
    );
}

#[test]
fn dotted_structured_field_repair_overrides_config_validation_contract() {
    let req = "读取 configs/config.toml 中 skills.skill_switches.config_basic 的值；若该字段不存在，说明它未显式配置";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::ConfigValidation,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "config_validation",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("config_validation_field_selector_requires_scalar_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn dotted_structured_field_repair_overrides_quantity_comparison_contract() {
    let req = "读取 configs/config.toml 里的 tools.allow_path_outside_workspace，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::QuantityComparison,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_field_selector_requires_scalar_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn structured_field_pair_repair_overrides_quantity_comparison_contract() {
    let req = "Read the name from scripts/nl_tests/fixtures/device_local/package.json. Read package.name from crates/clawd/Cargo.toml. Then answer in one line with the two names and whether they are the same or different.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: String::new(),
        semantic_kind: OutputSemanticKind::QuantityComparison,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_field_pair_requires_scalar_equality_check")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::RecentScalarEqualityCheck
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert!(contract
        .locator_hint
        .contains("scripts/nl_tests/fixtures/device_local/package.json"));
    assert!(contract.locator_hint.contains("crates/clawd/Cargo.toml"));
}

#[test]
fn structured_config_keys_repair_overrides_file_names_contract() {
    let req = "读取 configs/config.toml 的顶层键名，只输出键名列表";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("structured_config_keys_overrides_file_names"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::StructuredKeys);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn structured_identifier_presence_repair_overrides_file_existence_contract() {
    let req = "Read docker/config/skills_registry.toml and answer whether fs_basic is registered.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "docker/config/skills_registry.toml".to_string(),
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_identifier_presence_requires_content_evidence")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "docker/config/skills_registry.toml");
}

#[test]
fn structured_identifier_presence_repair_overrides_config_validation_contract() {
    let req = "Read docker/config/skills_registry.toml and answer whether fs_basic is registered.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "docker/config/skills_registry.toml".to_string(),
        semantic_kind: OutputSemanticKind::ConfigValidation,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_identifier_presence_requires_content_evidence")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "docker/config/skills_registry.toml");
}

#[test]
fn structured_identifier_presence_repair_overrides_content_presence_contract() {
    let req = "Read docker/config/skills_registry.toml and answer whether fs_basic is registered.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "docker/config/skills_registry.toml".to_string(),
        semantic_kind: OutputSemanticKind::ContentPresenceCheck,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_identifier_presence_requires_content_evidence")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "docker/config/skills_registry.toml");
}

#[test]
fn quoted_literal_presence_repair_overrides_path_existence_contract() {
    let req = "Check crates/clawd/src/virtual_tools.rs for “NEEDLE_TOKEN_123”.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "crates/clawd/src/virtual_tools.rs".to_string(),
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("quoted_literal_content_presence_contract_repair")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentPresenceCheck
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
}

#[test]
fn scalar_structured_keys_contract_repairs_to_field_value_contract() {
    let req = "去 package.json 里把项目名找出来，只把 name 的值回给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "package.json".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "structured_keys",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_keys_scalar_response_requires_field_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn current_workspace_scalar_structured_keys_contract_repairs_to_field_value_contract() {
    let req = "package.json 里的 name 到底是什么，只给值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "package.json".to_string(),
        semantic_kind: OutputSemanticKind::StructuredKeys,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        "structured_keys",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(
        reason,
        Some("structured_keys_scalar_response_requires_field_value")
    );
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn planner_locator_contract_repair_requires_evidence_for_sparse_contract() {
    let req = "Read configs/config.toml and output the selected_vendor field and value";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("planner_locator_requires_evidence"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn finalizer_language_policy_dry_run_keeps_locatorless_contract() {
    let req = "Render the final answer through the language policy.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "dry_run message_key=clawd.finalizer.language_policy finalizer i18n structured_evidence",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn prompt_finalizer_language_policy_dry_run_without_real_locator_keeps_locatorless_contract() {
    let req = "Dry-run only: final natural language is generated by finalizer/LLM/i18n in the user's language; runtime only emits message_key or structured evidence.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn prompt_only_finalizer_language_policy_words_do_not_skip_locator_repair() {
    let req = "Read configs/config.toml. Dry-run only: finalizer/LLM/i18n may render user language, but runtime returns message_key or structured_evidence.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("planner_locator_requires_evidence"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
}

#[test]
fn inline_json_payload_context_is_not_repaired_as_path_content() {
    let req = r#"[{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("inline_structured_payload_context_execute"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

#[test]
fn inline_json_transform_repairs_misclassified_content_excerpt_contract() {
    let req = r#"Sort this JSON array by score descending and output only a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#;
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::ContentExcerptWithSummary,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("inline_structured_transform_contract_repair"));
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
}

#[test]
fn scalar_direct_answer_candidate_is_not_promoted_by_filename_like_text() {
    let req = "Literal text: app.log, answer only acknowledged.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "acknowledged",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn scalar_empty_answer_candidate_does_not_promote_filename_like_literal_text() {
    let req = "Literal text: app.log, answer only acknowledged.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn structural_contract_repair_does_not_bind_workspace_child_mentions() {
    let workspace_root = make_temp_workspace_with_child("workspace_child_mentions", "document");
    let req = "列出document目录下有哪些文件，只输出文件名列表";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        &workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn structural_contract_repair_does_not_bind_case_mismatched_product_name() {
    let workspace_root = make_temp_workspace_with_child("workspace_child_product_name", "rustclaw");
    let req = "你好，我正在做 RustClaw 的真实客户端连续会话测试，请用一句中文回复确认。";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let reason = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        req,
        &surface,
        &workspace_root,
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn executionless_chat_wrapped_execute_cleans_finalize_trace_before_final_gate() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };

    let reason = super::cleanup_executionless_finalize_trace(
        &mut finalize_style,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
    );

    assert_eq!(reason, Some("executionless_finalize_trace_plain"));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
}

#[test]
fn parse_output_semantic_kind_accepts_command_output_summary() {
    assert_eq!(
        super::parse_output_semantic_kind("command_output_summary"),
        OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(
        super::parse_output_semantic_kind("command_result_summary"),
        OutputSemanticKind::CommandOutputSummary
    );
}

#[test]
fn parse_output_semantic_kind_accepts_generated_file_path_report() {
    assert_eq!(
        super::parse_output_semantic_kind("generated_file_path_report"),
        OutputSemanticKind::GeneratedFilePathReport
    );
    assert_eq!(
        super::normalize_output_semantic_kind_for_schema("write_then_report_path"),
        OutputSemanticKind::GeneratedFilePathReport.as_str()
    );
}

#[test]
fn parse_output_semantic_kind_accepts_photo_organization() {
    assert_eq!(
        super::parse_output_semantic_kind("photo_organization"),
        OutputSemanticKind::PhotoOrganization
    );
    assert_eq!(
        super::parse_output_semantic_kind("photo_source_candidates"),
        OutputSemanticKind::PhotoOrganization
    );
    assert_eq!(
        super::normalize_output_semantic_kind_for_schema("photo_organize"),
        OutputSemanticKind::PhotoOrganization.as_str()
    );
}

#[test]
fn normalize_output_contract_for_schema_keeps_generated_file_path_report_non_delivery() {
    let raw = r#"{
          "decision":"planner_execute",
          "needs_clarify":false,
          "output_contract":{
            "response_shape":"file_token",
            "requires_content_evidence":false,
            "delivery_required":true,
            "locator_kind":"filename",
            "delivery_intent":"file_single",
            "contract_marker":"write_then_report_path",
            "locator_hint":"pwd_line_abs.txt"
          }
        }"#;
    let (normalized, _report) =
        super::normalize_intent_normalizer_raw_for_schema_with_report(raw, "write path report");
    let value = serde_json::from_str::<serde_json::Value>(&normalized).expect("json");

    assert_eq!(
        value
            .pointer("/output_contract/contract_marker")
            .and_then(|value| value.as_str()),
        Some("generated_file_path_report")
    );
    assert_eq!(
        value
            .pointer("/output_contract/response_shape")
            .and_then(|value| value.as_str()),
        Some("scalar")
    );
    assert_eq!(
        value
            .pointer("/output_contract/delivery_required")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
    assert_eq!(
        value
            .pointer("/output_contract/delivery_intent")
            .and_then(|value| value.as_str()),
        Some("none")
    );
}
