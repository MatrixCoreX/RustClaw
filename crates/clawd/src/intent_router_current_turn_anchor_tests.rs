// Current-turn anchor repair tests for intent_router.

use crate::FirstLayerDecision;

use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind,
};

#[test]
fn current_turn_anchor_drift_repair_discards_contextual_path_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::SqliteSchemaVersion,
        locator_hint: "/tmp/rustclaw-anchor-test/data/db-basic-contract.sqlite".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "查询 /tmp/rustclaw-anchor-test/data/db-basic-contract.sqlite 的 schema version",
        "/tmp/rustclaw-anchor-test/logs",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_hint, "/tmp/rustclaw-anchor-test/logs");
}

#[test]
fn current_turn_anchor_drift_repair_preserves_file_delivery_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: "/tmp/rustclaw-anchor-test/old.md".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "Send me /tmp/rustclaw-anchor-test/old.md",
        "/tmp/rustclaw-anchor-test/LICENSE.zh-CN.md",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(!contract.requires_content_evidence);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(
        contract.locator_hint,
        "/tmp/rustclaw-anchor-test/LICENSE.zh-CN.md"
    );
}

#[test]
fn current_turn_anchor_drift_repair_skips_generated_file_delivery_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_hint: "/tmp/rustclaw-anchor-test/hello_world.sh".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "Write a shell script and deliver the file.",
        "/tmp/rustclaw-anchor-test/hello.sh",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert!(contract.requires_content_evidence);
    assert_eq!(
        contract.locator_hint,
        "/tmp/rustclaw-anchor-test/hello_world.sh"
    );
}

#[test]
fn current_turn_anchor_drift_repair_skips_tool_discovery_context_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::ToolDiscovery,
        locator_hint: String::new(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "List projection tokens including /tmp/rustclaw-anchor-test/old-context.md",
        "/tmp/rustclaw-anchor-test/current.md",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ToolDiscovery);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn current_turn_anchor_drift_repair_preserves_raw_command_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_hint: "/tmp/rustclaw-anchor-test/README.md".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "执行 ls scripts，把结果按字母倒序排，只输出前 5 个",
        "/tmp/rustclaw-anchor-test/scripts",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn current_turn_anchor_drift_repair_preserves_execution_failed_step_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::ExecutionFailedStep,
        locator_hint: "/tmp/rustclaw-anchor-test/README.md".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "Execute a structured command sequence and report the failed step.",
        "/tmp/rustclaw-anchor-test/scripts",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExecutionFailedStep
    );
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn current_turn_anchor_drift_repair_preserves_quantity_comparison_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::QuantityComparison,
        locator_hint: "/tmp/rustclaw-anchor-test/Cargo.toml".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "比较 Cargo.lock 和 Cargo.toml 的大小比例",
        "/tmp/rustclaw-anchor-test/Cargo.lock",
        workspace,
    );

    assert_eq!(
        repair,
        Some("current_turn_anchor_overrides_contextual_target")
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::QuantityComparison
    );
    assert_eq!(contract.locator_hint, workspace.display().to_string());
}

#[test]
fn current_turn_anchor_drift_repair_keeps_compatible_child_path() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::FileNames,
        locator_hint: "/tmp/rustclaw-anchor-test/logs/clawd.log".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "列出 /tmp/rustclaw-anchor-test/logs/clawd.log 的基本信息",
        "/tmp/rustclaw-anchor-test/logs",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::FileNames);
    assert_eq!(
        contract.locator_hint,
        "/tmp/rustclaw-anchor-test/logs/clawd.log"
    );
}

#[test]
fn current_turn_anchor_drift_repair_keeps_multi_target_locator_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_hint: "README.md, README.zh-CN.md, Cargo.toml".to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "Check README.md, README.zh-CN.md, and Cargo.toml in the current workspace",
        "/tmp/rustclaw-anchor-test/README.md",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExistenceWithPath
    );
    assert_eq!(
        contract.locator_hint,
        "README.md, README.zh-CN.md, Cargo.toml"
    );
}

#[test]
fn current_turn_anchor_drift_repair_preserves_archive_pair_contract() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        semantic_kind: OutputSemanticKind::ArchiveUnpack,
        locator_hint:
            "/tmp/rustclaw-anchor-test/tmp/test_bundle.zip | /tmp/rustclaw-anchor-test/out"
                .to_string(),
        ..Default::default()
    };

    let repair = super::apply_current_turn_anchor_drift_repair(
        &mut contract,
        "archive unpack path pair",
        "/tmp/rustclaw-anchor-test/tmp/test_bundle.zip",
        workspace,
    );

    assert_eq!(repair, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchiveUnpack);
    assert_eq!(
        contract.locator_hint,
        "/tmp/rustclaw-anchor-test/tmp/test_bundle.zip | /tmp/rustclaw-anchor-test/out"
    );
}

#[test]
fn current_turn_anchor_repair_stays_off_for_executionless_chat() {
    let contract = IntentOutputContract::default();
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::DirectAnswer,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_allowed_for_structured_evidence_contract() {
    let contract = IntentOutputContract {
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::DirectAnswer,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_generated_file_delivery() {
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        ..IntentOutputContract::default()
    };
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        true,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_tool_discovery_context_contract() {
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::ToolDiscovery,
        ..IntentOutputContract::default()
    };
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_allowed_for_explicit_act_route() {
    let contract = IntentOutputContract::default();
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test");

    assert!(super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_request_session_alias_detection_uses_session_state() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                alias: "那个 README".to_string(),
                target: "/tmp/rustclaw-anchor-test/README.md".to_string(),
                updated_at_ts: 1,
            }],
            ..Default::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(super::current_request_mentions_session_alias(
        Some(&snapshot),
        "读一下那个 README 开头 5 行",
    ));
    assert!(!super::current_request_mentions_session_alias(
        Some(&snapshot),
        "读一下 README.md 开头 5 行",
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_structured_config_contract_with_locator() {
    let contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::ConfigRiskAssessment,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_current_workspace_root_identity() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");
    let contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "RustClaw".to_string(),
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}

#[test]
fn current_turn_anchor_repair_stays_off_for_current_workspace_absolute_hint() {
    let workspace = std::path::Path::new("/tmp/rustclaw-anchor-test/rustclaw");
    let contract = IntentOutputContract {
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "/tmp/rustclaw-anchor-test/rustclaw".to_string(),
        semantic_kind: OutputSemanticKind::None,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    assert!(!super::current_turn_anchor_drift_repair_allowed(
        FirstLayerDecision::PlannerExecute,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
        workspace,
    ));
}
