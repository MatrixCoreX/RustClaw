// Observation clarify and contract-hint repair tests for intent_router.

use crate::FirstLayerDecision;

use super::test_support::make_temp_workspace_with_child;
use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, TargetTaskPolicy, TurnType,
};

#[test]
fn structured_observation_clarify_repair_routes_concrete_file_request_to_act() {
    let req = "读取 package.json 里的 name 字段，只输出值";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供 package.json 文件内容".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(contract.locator_hint, "package.json");
}

#[test]
fn structured_observation_clarify_repair_fills_file_delivery_filename_locator() {
    let req = "把 definitely_missing_named_file_phase0_runtime_20260515.txt 发给我";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question =
        "请提供 definitely_missing_named_file_phase0_runtime_20260515.txt 的完整路径".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(
        contract.locator_hint,
        "definitely_missing_named_file_phase0_runtime_20260515.txt"
    );
    assert!(contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_routes_multi_filename_request_to_workspace_act() {
    let req = "检查 README.md, README.zh-CN.md, Cargo.toml, and no_such_file_20260513.txt 是否存在，用表格返回";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供具体的文件夹路径".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, workspace_root.display().to_string());
}

#[test]
fn structured_observation_clarify_repair_requires_machine_shape_not_semantic_kind_alone() {
    let surface = crate::intent::surface_signals::PromptSurfaceSignals {
        filename_candidates: vec!["README.md".to_string()],
        single_filename_candidate: Some("README.md".to_string()),
        ..Default::default()
    };
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "provide the missing target".to_string();
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        "README.md",
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "provide the missing target");
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn workspace_default_observation_clarify_repair_routes_listing_without_absolute_path_to_act() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::FileNames,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Please provide the full UI directory path.".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_workspace_default_observation_clarify_repair(
        "file_names",
        &mut contract,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("workspace_default_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, workspace_root.display().to_string());
}

#[test]
fn workspace_default_observation_clarify_repair_keeps_unbound_scalar_count_clarify() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "provide the missing directory target".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_workspace_default_observation_clarify_repair(
        "",
        &mut contract,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "provide the missing directory target");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn workspace_default_observation_clarify_repair_ignores_legacy_docker_contract_marker() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::DockerContainerLifecycle,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_workspace_default_observation_clarify_repair(
        "docker_container_lifecycle",
        &mut contract,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert!(!clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::DockerContainerLifecycle
    );
}

#[test]
fn locatorless_observation_clarify_repair_routes_service_status_to_act() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::ServiceStatus,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Please provide the service target.".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_locatorless_observation_clarify_repair(
        "service_status",
        &mut contract,
        "",
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("locatorless_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ServiceStatus);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn locatorless_observation_clarify_repair_accepts_capability_ref_with_semantic_none() {
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Please provide the target.".to_string();
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_locatorless_observation_clarify_repair(
        "",
        &mut contract,
        "capability_ref=weather.current place=Beijing",
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("locatorless_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

#[test]
fn locatorless_observation_clarify_repair_accepts_any_capability_ref_machine_token() {
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Please provide the target.".to_string();
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_locatorless_observation_clarify_repair(
        "",
        &mut contract,
        "capability_ref=weather.current_extra place=Beijing",
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("locatorless_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
}

#[test]
fn deictic_missing_locator_state_patch_forces_boundary_clarify_contract() {
    let patch = serde_json::json!({"deictic_reference": {"target": "missing_locator"}});
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/home/guagua/rustclaw".to_string(),
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: false,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_deictic_missing_locator_state_patch_clarify_repair(
        &mut contract,
        Some(&patch),
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("state_patch_deictic_missing_locator_clarify"));
    assert!(needs_clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
}

#[test]
fn structured_contract_hint_repair_ignores_legacy_git_semantic_hint() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "检查这个仓库当前是否有未提交改动，用一句话说明。\n",
        "[CONTRACT_TEST_HINT]\n",
        "contract_id=git_repository_state\n",
        "semantic_kind=git_repository_state\n",
        "required_evidence_json=[\"field_value\"]\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: workspace_root.display().to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let surface_req = super::request_without_contract_test_hint(req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut wants_file_delivery = false;
    let reason = super::apply_structured_contract_hint_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        &mut wants_file_delivery,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert!(contract.requires_content_evidence);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(!needs_clarify);
}

#[test]
fn structured_contract_hint_repair_ignores_legacy_package_manager_semantic_hint() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "检测这台机器可用的包管理器，并说明依据。\n",
        "[CONTRACT_TEST_HINT]\n",
        "contract_id=package_manager_detection\n",
        "semantic_kind=package_manager_detection\n",
        "required_evidence_json=[\"field_value\"]\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "model-supplied-background-locator".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let surface_req = super::request_without_contract_test_hint(req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut wants_file_delivery = false;
    let reason = super::apply_structured_contract_hint_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        &mut wants_file_delivery,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "model-supplied-background-locator");
    assert!(needs_clarify);
    assert!(!clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
}

#[test]
fn structured_contract_hint_repair_keeps_tool_discovery_context_only() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "request.\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=tool_discovery\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "model-supplied-background-locator".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "provide locator".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let surface_req = super::request_without_contract_test_hint(req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut wants_file_delivery = false;
    let reason = super::apply_structured_contract_hint_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        &mut wants_file_delivery,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "model-supplied-background-locator");
    assert!(needs_clarify);
    assert_eq!(clarify_question, "provide locator");
    assert_eq!(decision, FirstLayerDecision::Clarify);
}

#[test]
fn structured_contract_hint_repair_sets_generated_file_delivery_defaults() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "写一个简单文本文件到 tmp/contract_matrix_generated_note.txt，内容是 RustClaw contract matrix test，然后把文件路径发给我。\n",
        "[CONTRACT_TEST_HINT]\n",
        "contract_id=generated_file_delivery\n",
        "semantic_kind=generated_file_delivery\n",
        "required_evidence_json=[\"path\"]\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface_req = super::request_without_contract_test_hint(req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: false,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };
    let mut wants_file_delivery = false;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要发送的文件路径或文件名。".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_structured_contract_hint_repair(
        &mut contract,
        req,
        &surface,
        workspace_root,
        &mut wants_file_delivery,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_contract_hint_repair"));
    assert!(wants_file_delivery);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert!(contract
        .locator_hint
        .contains("tmp/contract_matrix_generated_note.txt"));
    assert_eq!(decision, FirstLayerDecision::Clarify);
}

#[test]
fn request_without_contract_test_hint_removes_machine_block() {
    let req = "检测包管理器。\n[CONTRACT_TEST_HINT]\nsemantic_kind=package_manager_detection\n[/CONTRACT_TEST_HINT]\n谢谢";
    let stripped = super::request_without_contract_test_hint(req);
    assert!(stripped.contains("检测包管理器"));
    assert!(stripped.contains("谢谢"));
    assert!(!stripped.contains("CONTRACT_TEST_HINT"));
    assert!(!stripped.contains("[/"));
}

#[test]
fn current_turn_contract_repair_does_not_path_bind_package_manager_hint() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let raw_req = concat!(
        "检测这台机器可用的包管理器。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=package_manager_detection\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface_req = super::request_without_contract_test_hint(raw_req);
    let surface = crate::intent::surface_signals::analyze_prompt_surface(&surface_req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::PackageManagerDetection,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };
    let _ = super::apply_current_turn_structural_contract_repair(
        "",
        &mut contract,
        &surface_req,
        &surface,
        workspace_root,
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::Standalone),
    );

    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::PackageManagerDetection
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn recent_scalar_equality_requires_fresh_evidence() {
    assert!(super::output_semantic_kind_requires_fresh_evidence(
        OutputSemanticKind::RecentScalarEqualityCheck
    ));
}

#[test]
fn contract_hint_fallback_ignores_legacy_git_semantic_hint() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "处理这个请求。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=git_repository_state\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    );

    assert!(decision.is_none());
}

#[test]
fn contract_hint_fallback_ignores_legacy_package_manager_semantic_hint() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "处理这个请求。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=package_manager_detection\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    );

    assert!(decision.is_none());
}

#[test]
fn contract_hint_fallback_keeps_tool_discovery_context_only() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "request.\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=tool_discovery\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    );

    assert!(decision.is_none());
}

#[test]
fn contract_hint_fallback_extracts_path_locator() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "处理 scripts/nl_tests/fixtures/device_local/docs/release_checklist.md。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=content_excerpt_summary\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    )
    .expect("contract hint fallback");

    assert!(!decision.needs_clarify);
    assert_eq!(
        decision.output_contract.semantic_kind,
        OutputSemanticKind::ContentExcerptSummary
    );
    assert_eq!(
        decision.output_contract.locator_kind,
        OutputLocatorKind::Path
    );
    assert!(decision
        .output_contract
        .locator_hint
        .contains("release_checklist.md"));
}

#[test]
fn contract_hint_workspace_summary_preserves_explicit_directory_locator() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = concat!(
        "快速看一下 scripts/nl_tests/fixtures/device_local，用非技术用户能听懂的话总结它是什么项目。\n",
        "[CONTRACT_TEST_HINT]\n",
        "semantic_kind=workspace_project_summary\n",
        "[/CONTRACT_TEST_HINT]"
    );
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let decision = super::contract_hint_fallback_decision(
        req,
        &surface,
        workspace_root,
        "normalizer_parse_failed_contract_hint",
    )
    .expect("contract hint fallback");

    assert!(!decision.needs_clarify);
    assert_eq!(
        decision.output_contract.semantic_kind,
        OutputSemanticKind::WorkspaceProjectSummary
    );
    assert_eq!(
        decision.output_contract.locator_kind,
        OutputLocatorKind::Path
    );
    assert!(decision
        .output_contract
        .locator_hint
        .contains("scripts/nl_tests/fixtures/device_local"));
}

#[test]
fn structured_observation_clarify_repair_routes_two_explicit_targets_to_act() {
    let req = "比较 configs/skills_registry.toml 和 docker/config/skills_registry.toml 哪个文件更大，只回答文件名和大小差";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "您希望我执行文件大小比较操作吗？".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        workspace_root,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, Some("structured_observation_clarify_repair"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "configs/skills_registry.toml, docker/config/skills_registry.toml"
    );
}

#[test]
fn resolved_directory_observation_clarify_repair_routes_existing_workspace_dir_to_act() {
    let workspace_root = make_temp_workspace_with_child("resolved_dir_clarify", "docs");
    std::fs::write(workspace_root.join("docs").join("a.md"), "alpha").expect("write a");
    std::fs::write(workspace_root.join("docs").join("b.md"), "beta").expect("write b");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    let req = "List the two largest files directly under docs and say what kind of docs they appear to be.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Should I use document or docs?".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_resolved_directory_observation_clarify_repair(
        &state,
        &mut contract,
        req,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        reason,
        Some("resolved_directory_observation_clarify_repair")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        workspace_root
            .join("docs")
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn resolved_directory_observation_clarify_repair_recovers_empty_extension_listing_contract() {
    let workspace_root = make_temp_workspace_with_child("resolved_dir_empty_listing", "document");
    std::fs::write(workspace_root.join("document").join("alpha.md"), "alpha").expect("write alpha");
    std::fs::write(workspace_root.join("document").join("README.md"), "readme")
        .expect("write readme");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    let req = "列出 document 目录里所有 .md 文件，但排除 README，告诉我还剩哪些";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "Please provide the directory path.".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_resolved_directory_observation_clarify_repair(
        &state,
        &mut contract,
        req,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        reason,
        Some("resolved_directory_observation_clarify_repair")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        workspace_root
            .join("document")
            .canonicalize()
            .unwrap()
            .display()
            .to_string()
    );
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn resolved_directory_observation_clarify_repair_preserves_non_locator_semantics() {
    let workspace_root = make_temp_workspace_with_child("resolved_dir_non_locator", "target");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    let req = "查看那个 schema 里的 target enum";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要查看的 schema 文件的路径或名称。".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_resolved_directory_observation_clarify_repair(
        &state,
        &mut contract,
        req,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.locator_hint.is_empty());
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn resolved_directory_observation_clarify_repair_preserves_bare_locator_only_reply() {
    let workspace_root = make_temp_workspace_with_child("resolved_dir_bare", "docs");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace_root.clone();
    state.skill_rt.default_locator_search_dir = workspace_root.clone();
    let req = "docs";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "What should I do with docs?".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_resolved_directory_observation_clarify_repair(
        &state,
        &mut contract,
        req,
        &surface,
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(clarify_question, "What should I do with docs?");
    std::fs::remove_dir_all(workspace_root).ok();
}

#[test]
fn unbound_workspace_generic_content_repair_clarifies_short_topic() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root");
    let req = "opaquetopic";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: workspace_root.display().to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_unbound_workspace_generic_content_clarify_repair(
        &mut contract,
        req,
        &surface,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        reason,
        Some("unbound_workspace_generic_content_requires_clarify")
    );
    assert!(needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn unbound_workspace_generic_content_repair_preserves_structured_semantic() {
    let req = "opaquetopic";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::QuantityComparison,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/workspace".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_unbound_workspace_generic_content_clarify_repair(
        &mut contract,
        req,
        &surface,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::QuantityComparison
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "/workspace");
}

#[test]
fn unbound_workspace_generic_content_repair_preserves_concrete_locator_surface() {
    let req = "Cargo.toml";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/workspace".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;

    let reason = super::apply_unbound_workspace_generic_content_clarify_repair(
        &mut contract,
        req,
        &surface,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(!needs_clarify);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn structured_observation_clarify_repair_preserves_named_target_without_clean_locator() {
    let req = "读一下 README 然后用恰好三句话总结，不要多也不要少";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供 README 的具体内容或文件路径".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供 README 的具体内容或文件路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_deictic_bare_target_clarify() {
    let req = "读一下那个 README 开头并用一句话总结";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请确认具体 README 路径".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let patch = serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}});

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        Some(&patch),
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请确认具体 README 路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_unbound_scope_filename_target() {
    let req = "去那个 case_only 目录里找 report.md，只输出路径";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供 case_only 目录的完整路径".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "existence_with_path",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供 case_only 目录的完整路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn structured_observation_clarify_repair_preserves_version_correction_clarify() {
    let req = "Correction: mention Python 3.11, not Python 3.10.";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请确认要修正哪段内容".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请确认要修正哪段内容");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_deictic_with_destination_path_clarify() {
    let req = "把那个压缩包解压到 /tmp/unpack_dest 然后告诉我结果";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/unpack_dest".to_string(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供压缩包路径".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let patch = serde_json::json!({"deictic_reference":{"target":"unresolved_prior_object"}});

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        Some(&patch),
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供压缩包路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
}

#[test]
fn structured_observation_clarify_repair_preserves_deictic_destination_without_patch() {
    let req = "把那个压缩包解压到 /tmp/unpack_dest 然后告诉我结果";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(req);
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };
    let mut needs_clarify = true;
    let mut clarify_question = "请提供压缩包路径".to_string();
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;

    let reason = super::apply_spurious_structured_observation_clarify_repair(
        "",
        &mut contract,
        req,
        &surface,
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("workspace root"),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(reason, None);
    assert!(needs_clarify);
    assert_eq!(clarify_question, "请提供压缩包路径");
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}
