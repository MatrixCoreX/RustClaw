// Current-turn structural repair tests for intent_router.

use crate::FirstLayerDecision;

use super::{
    IntentOutputContract, OutputLocatorKind, OutputResponseShape, OutputSemanticKind,
    TargetTaskPolicy, TurnType,
};

#[test]
fn observed_context_summary_followup_does_not_force_fresh_evidence() {
    let mut contract = IntentOutputContract::default();
    contract.response_shape = OutputResponseShape::OneSentence;
    contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    contract.locator_kind = OutputLocatorKind::Filename;
    contract.locator_hint = "app.log".to_string();

    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "in one sentence tell me if anything looks abnormal",
    );
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        "in one sentence tell me if anything looks abnormal",
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::DirectAnswer,
        "",
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
    );

    assert_eq!(reason, Some("existing_observed_context_synthesis"));
    assert!(!contract.requires_content_evidence);
}

#[test]
fn explicit_locator_summary_still_requires_fresh_evidence() {
    let mut contract = IntentOutputContract::default();
    contract.response_shape = OutputResponseShape::OneSentence;
    contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    contract.locator_kind = OutputLocatorKind::Filename;
    contract.locator_hint = "app.log".to_string();

    let surface =
        crate::intent::surface_signals::analyze_prompt_surface("summarize app.log briefly");
    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        "summarize app.log briefly",
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::DirectAnswer,
        "",
        Some(TurnType::TaskAppend),
        Some(TargetTaskPolicy::ReuseActive),
    );

    assert_eq!(reason, Some("semantic_contract_requires_evidence"));
    assert!(contract.requires_content_evidence);
}

#[test]
fn fs_basic_lifecycle_machine_tokens_repair_command_summary_contract() {
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "tmp/nl_codex_resume_smoke".to_string(),
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_fs_basic_lifecycle_machine_contract_repair(
        &mut contract,
        "fs_basic.make_dir -> write_text -> append_text -> read_text_range -> remove_path(recursive)",
    );

    assert_eq!(reason, Some("fs_basic_lifecycle_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::FilesystemMutationResult
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
}

#[test]
fn media_generation_machine_tokens_repair_command_summary_contract() {
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "document/media_dry_run/image_status_card.png".to_string(),
        semantic_kind: OutputSemanticKind::CommandOutputSummary,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_media_generation_path_report_machine_contract_repair(
        &mut contract,
        "capability=image.generate dry_run=true output_path=document/media_dry_run/image_status_card.png planned_outputs",
    );

    assert_eq!(reason, Some("media_generation_path_report_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFilePathReport
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "document/media_dry_run/image_status_card.png"
    );
}

#[test]
fn media_generation_machine_tokens_override_publishing_preview_contract() {
    let request = "capability=image.generate dry_run=true output_path=document/media_dry_run/image_status_card.png planned_outputs";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        semantic_kind: OutputSemanticKind::PublishingPreview,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("media_generation_path_report_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFilePathReport
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn media_generation_path_tokens_force_evidence_for_generic_contract() {
    let request = "capability=music.generate dry_run=true output_path=document/media_dry_run/ambient_loop.mp3 planned_outputs";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: false,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("media_generation_path_report_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFilePathReport
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::CurrentWorkspace);
}

#[test]
fn media_generation_machine_tokens_repair_generic_contract() {
    let request = "capability=audio.synthesize dry_run=true output_path=document/media_dry_run/audio_check.mp3 planned_outputs";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "document/media_dry_run/audio_check.mp3".to_string(),
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("media_generation_path_report_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFilePathReport
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "document/media_dry_run/audio_check.mp3"
    );
}

#[test]
fn media_generation_machine_tokens_override_filesystem_mutation_contract() {
    let request = "capability=image.generate dry_run=true output_path=document/media_dry_run/image_status_card.png planned_outputs";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "document/media_dry_run/image_status_card.png".to_string(),
        semantic_kind: OutputSemanticKind::FilesystemMutationResult,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("media_generation_path_report_contract_repair"));
    assert_eq!(
        contract.semantic_kind,
        OutputSemanticKind::GeneratedFilePathReport
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
    assert_eq!(contract.response_shape, OutputResponseShape::Strict);
}

#[test]
fn structural_config_field_value_repairs_to_config_mutation_contract() {
    let request = "run/nl_eval_tmp/config_edit_smoke/config.toml skills.skill_switches.config_edit_nl_smoke = true";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("config_mutation_structural_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ConfigMutation);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "run/nl_eval_tmp/config_edit_smoke/config.toml"
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
}

#[test]
fn structural_config_field_value_overrides_risk_misroute_to_config_mutation_contract() {
    let request = "configs/config.toml skills.skill_switches.config_edit_nl_plan = true";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "configs/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::ConfigRiskAssessment,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("config_mutation_structural_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ConfigMutation);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(contract.locator_hint, "configs/config.toml");
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
}

#[test]
fn structural_config_field_value_overrides_failed_step_misroute_to_config_mutation_contract() {
    let request =
        "run/nl_eval_tmp/config_edit_smoke/config.toml skills.skill_switches.config_edit_nl_smoke = true";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::ExecutionFailedStep,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("config_mutation_structural_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ConfigMutation);
    assert_eq!(contract.response_shape, OutputResponseShape::Free);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "run/nl_eval_tmp/config_edit_smoke/config.toml"
    );
    assert!(contract.requires_content_evidence);
    assert!(!contract.delivery_required);
}

#[test]
fn config_mutation_contract_repairs_missing_locator_from_current_request() {
    let request = "run/nl_eval_tmp/config_edit_smoke/config.toml skills.skill_switches.config_edit_nl_smoke = true";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: String::new(),
        semantic_kind: OutputSemanticKind::ConfigMutation,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, Some("config_mutation_structural_contract_repair"));
    assert_eq!(contract.semantic_kind, OutputSemanticKind::ConfigMutation);
    assert_eq!(contract.locator_kind, OutputLocatorKind::Path);
    assert_eq!(
        contract.locator_hint,
        "run/nl_eval_tmp/config_edit_smoke/config.toml"
    );
    assert!(contract.requires_content_evidence);
}

#[test]
fn structural_config_field_without_value_does_not_repair_to_mutation() {
    let request =
        "run/nl_eval_tmp/config_edit_smoke/config.toml skills.skill_switches.config_edit_nl_smoke";
    let surface = crate::intent::surface_signals::analyze_prompt_surface(request);
    let mut contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string(),
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let reason = super::apply_current_turn_structural_contract_repair(
        &mut contract,
        request,
        &surface,
        std::path::Path::new("/workspace"),
        FirstLayerDecision::PlannerExecute,
        "",
        None,
        None,
    );

    assert_eq!(reason, None);
    assert_eq!(contract.semantic_kind, OutputSemanticKind::None);
}
