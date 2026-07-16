// Execution contract repair tests for intent_router.

use crate::FirstLayerDecision;

use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind,
};

#[test]
fn explicit_command_execution_repair_prevents_executionless_downgrade() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/home/guagua/rustclaw".to_string(),
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "请执行 `git rev-parse --abbrev-ref HEAD`，只输出命令结果",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );
    let downgrade = super::cleanup_executionless_finalize_trace(
        &mut finalize_style,
        false,
        &contract,
        false,
        crate::ScheduleKind::None,
        None,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert_eq!(downgrade, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_preserves_directory_entry_groups_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts".to_string(),
        semantic_kind: OutputSemanticKind::DirectoryEntryGroups,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "执行 `ls scripts`",
        "directory_entry_groups",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("explicit_command_preserves_structured_observation_contract")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert!(contract.requires_content_evidence);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::DirectoryEntryGroups
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "scripts");
}

#[test]
fn explicit_ls_selector_repair_converts_raw_command_to_directory_entry_groups() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "执行 ls scripts，把结果按字母倒序排，只输出前 5 个",
        "selector_target_kind=any selector_limit=5 selector_sort_by=name_desc selector_include_hidden=false.",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("explicit_command_directory_listing_selector_contract_repair")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::DirectoryEntryGroups
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "scripts");
    assert_eq!(contract.self_extension.list_selector.limit, Some(5));
    assert_eq!(
        contract.self_extension.list_selector.sort_by.as_deref(),
        Some("name_desc")
    );
    assert_eq!(
        contract.self_extension.list_selector.target_kind,
        crate::OutputScalarCountTargetKind::Any
    );
    assert_eq!(
        contract.self_extension.list_selector.include_hidden,
        Some(false)
    );
}

#[test]
fn explicit_command_execution_repair_preserves_current_workspace_scalar_path_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::ScalarPathOnly,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "Run `pwd` and output only the raw result.",
        "scalar_path_only",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("explicit_command_preserves_structured_observation_contract")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ScalarPathOnly);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn explicit_command_execution_repair_preserves_command_summary_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "Run `whoami` and `pwd`, then create one signature line from those results.",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("explicit_command_requires_command_output_summary_execution")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(contract.requires_content_evidence);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_keeps_raw_one_sentence_without_synthesis_marker() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "run `pwd` and return the observed stdout as one line.",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_defaults_unclassified_evidence_contract_to_raw_output() {
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "run `pwd`",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_raw_contract_ignores_legacy_decision_token_without_synthesis_marker() {
    let decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "Run `pwd` first, then create one reply line from the observed result.",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.response_shape, OutputResponseShape::OneSentence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_upgrades_raw_strict_with_synthesis_marker() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "执行 `pwd`，然后基于结果输出一行文本。",
        "command_result_synthesis",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("explicit_command_requires_command_output_summary_execution")
    );
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(contract.requires_content_evidence);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_preserves_failed_step_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        semantic_kind: OutputSemanticKind::ExecutionFailedStep,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "执行一个会失败的只读检查命令：`cat /definitely_missing_rustclaw_contract_case`，然后说明失败原因。",
        "execution_failed_step",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExecutionFailedStep
    );
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_preserves_generated_file_delivery_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_hint: "worker_line_explicit.txt".to_string(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "Run `pwd` first, save one short line to worker_line_explicit.txt, then tell me the saved path.",
        "generated_file_delivery",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("explicit_command_preserves_generated_file_delivery_execution")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFileDelivery
    );
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "worker_line_explicit.txt");
    assert!(contract.requires_content_evidence);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
}

#[test]
fn explicit_command_execution_repair_preserves_generated_file_path_report_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::GeneratedFilePathReport,
        locator_hint: "worker_line_explicit.txt".to_string(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "Run `pwd` first, save one short line to worker_line_explicit.txt, then return the saved path.",
        "generated_file_path_report",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("explicit_command_preserves_generated_file_path_report_execution")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFilePathReport
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
    assert_eq!(contract.locator_hint, "worker_line_explicit.txt");
    assert!(contract.requires_content_evidence);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
}

#[test]
fn explicit_command_execution_repair_respects_pure_direct_answer_contract() {
    let decision = FirstLayerDecision::DirectAnswer;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "execute ls -la: explain what this command means, do not run it",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, None);
    assert_eq!(decision, FirstLayerDecision::DirectAnswer);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn explicit_command_execution_repair_ignores_quoted_replacement_payload() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "Continue, but change the remaining step so it says `echo AFTER_PATCHED_STEP` instead.",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(!contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

#[test]
fn explicit_command_execution_repair_clears_spurious_clarify() {
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "请执行 `pwd`，只输出命令结果",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn embedded_standalone_command_execution_repair_clears_spurious_clarify() {
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_explicit_command_execution_contract_repair(
        "运行 pwd -P，只返回物理工作目录路径",
        "",
        &mut needs_clarify,
        &mut clarify_question,
        &mut contract,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("explicit_command_requires_fresh_execution"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::RawCommandOutput);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn command_payload_contract_repair_preserves_agent_loop_contract_when_semantic_missing() {
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut clarify_question = "请提供要读取或检查的具体文件、目录或路径。".to_string();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::None,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_command_payload_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("command_payload_preserved_for_agent_loop"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
}

#[test]
fn command_payload_contract_repair_preserves_command_summary_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_command_payload_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("command_payload_requires_command_output_summary_execution")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
    assert!(contract.requires_content_evidence);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn command_payload_contract_repair_preserves_service_status_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ServiceStatus,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_command_payload_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(repair, Some("command_payload_preserved_for_agent_loop"));
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert!(contract.requires_content_evidence);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ServiceStatus);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn command_payload_contract_repair_preserves_strict_command_summary_contract() {
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        locator_kind: OutputLocatorKind::None,
        ..IntentOutputContract::default()
    };

    let repair = super::apply_command_payload_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("command_payload_requires_command_output_summary_execution")
    );
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::CommandOutputSummary
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
}

#[test]
fn command_payload_contract_repair_preserves_failed_step_contract() {
    let decision = FirstLayerDecision::Clarify;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = true;
    let mut clarify_question = "Need a path target.".to_string();
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ExecutionFailedStep,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/rustclaw-anchor-test".to_string(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_command_payload_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("command_payload_requires_raw_output_execution")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::Clarify);
    assert!(contract.requires_content_evidence);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ExecutionFailedStep
    );
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn file_delivery_contract_repair_preserves_named_file_delivery_request() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ExecutionFailedStep,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "definitely_missing_named_file_rustclaw_24687.md".to_string(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_file_delivery_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("file_delivery_request_preserves_delivery_contract")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Filename);
    assert_eq!(
        contract.locator_hint,
        "definitely_missing_named_file_rustclaw_24687.md"
    );
}

#[test]
fn file_delivery_contract_repair_keeps_content_summary_delivery_contract() {
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        exact_sentence_count: Some(1),
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        semantic_kind: OutputSemanticKind::ContentExcerptWithSummary,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/config.toml".to_string(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_file_delivery_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(repair, None);
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::ContentExcerptWithSummary
    );
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
}

#[test]
fn file_delivery_contract_repair_does_not_let_generated_semantic_block_delivery_fields() {
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = true;
    let mut clarify_question = "provide the file path".to_string();
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::GeneratedFileDelivery,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "/tmp/generated-report.md".to_string(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_file_delivery_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(
        repair,
        Some("file_delivery_request_preserves_delivery_contract")
    );
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(contract.response_shape, OutputResponseShape::FileToken);
    assert!(contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::FileSingle);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}

#[test]
fn file_delivery_contract_repair_preserves_archive_pack_contract() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        delivery_required: false,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::ArchivePack,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/skill_calls | document/skill_calls_smoke.zip".to_string(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_file_delivery_contract_repair(
        true,
        &mut contract,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize_style,
    );

    assert_eq!(repair, None);
    assert!(!needs_clarify);
    assert!(clarify_question.is_empty());
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ArchivePack);
    assert_eq!(contract.response_shape, OutputResponseShape::Scalar);
    assert!(!contract.delivery_required);
    assert_eq!(contract.delivery_intent, OutputDeliveryIntent::None);
    assert_eq!(
        contract.locator_hint,
        "scripts/skill_calls | document/skill_calls_smoke.zip"
    );
}

#[test]
fn raw_output_explicit_locator_repair_restores_path_for_non_command_read() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_raw_output_explicit_locator_repair(
        &mut contract,
        "raw_command_output",
        "读 /etc/shadow 第一行，告诉我里面是什么",
    );

    assert_eq!(repair, Some("raw_output_explicit_locator_contract_repair"));
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "/etc/shadow");
}

#[test]
fn raw_output_explicit_locator_repair_skips_literal_command_requests() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_raw_output_explicit_locator_repair(
        &mut contract,
        "raw_command_output",
        "run `cat /etc/shadow`",
    );

    assert_eq!(repair, None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn raw_output_explicit_locator_repair_ignores_semantic_kind_without_machine_marker() {
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::RawCommandOutput,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    let repair = super::apply_raw_output_explicit_locator_repair(
        &mut contract,
        "",
        "读 /etc/shadow 第一行，告诉我里面是什么",
    );

    assert_eq!(repair, None);
    assert_eq!(contract.locator_kind, OutputLocatorKind::None);
    assert!(contract.locator_hint.is_empty());
}

#[test]
fn plain_execute_keeps_plain_finalizer_when_contract_is_sparse() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
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

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::Plain);
}

#[test]
fn execution_signal_act_route_stays_executable() {
    let decision = FirstLayerDecision::PlannerExecute;
    let mut finalize_style = crate::ActFinalizeStyle::ChatWrapped;
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::FileNames,
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

    assert_eq!(reason, None);
    assert_eq!(decision, FirstLayerDecision::PlannerExecute);
    assert_eq!(finalize_style, crate::ActFinalizeStyle::ChatWrapped);
}
