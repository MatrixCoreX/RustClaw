use super::*;
use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    ResumeBehavior, RiskCeiling, ScheduleKind,
};

fn route_result() -> RouteResult {
    RouteResult {
        ask_mode: crate::AskMode::planner_execute_plain(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Unknown,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: OutputResponseShape::Scalar,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: Default::default(),
            semantic_kind: Default::default(),
            locator_hint: String::new(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn fuzzy_candidates_force_clarify_for_locator_requests() {
    let result = apply_post_route_policy(
        route_result(),
        LocatorResolution::Fuzzy(vec!["/tmp/a".to_string(), "/tmp/b".to_string()]),
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert_eq!(result.fuzzy_locator_suggestions.len(), 2);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_fuzzy_locator_candidates"
    );
    assert_eq!(result.gate_record.owner_layer, "boundary_locator_gate");
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Clarify);
}

#[test]
fn missing_locator_still_forces_clarify() {
    let result = apply_post_route_policy(route_result(), LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.missing_locator_for_path_scoped_content);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_missing_path_scoped_locator"
    );
    assert_eq!(result.gate_record.owner_layer, "boundary_locator_gate");
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Clarify);
}

#[test]
fn generated_file_delivery_without_locator_hint_can_execute() {
    let mut route = route_result();
    route.resolved_intent =
        "Create a shell script, save it as a file, and deliver the generated file".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint.clear();

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert!(!result.missing_locator_for_path_scoped_content);
    assert_eq!(result.gate_record.reason_code, "post_route_no_change");
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::NoChange);
}

#[test]
fn generated_file_delivery_misclassified_as_path_without_hint_can_execute() {
    let mut route = route_result();
    route.resolved_intent =
        "Create a shell script, save it as a file, and deliver the generated file".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert!(!result.missing_locator_for_path_scoped_content);
}

#[test]
fn existing_file_delivery_with_locator_hint_executes_for_runtime_not_found() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_delivery_locator; unresolved_file_delivery_requires_clarify"
            .to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "definitely_missing_named_file_rustclaw_001.txt".into();

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert!(!result.missing_locator_for_path_scoped_content);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_file_delivery_locator_hint_deferred_to_execution"
    );
    assert_eq!(result.gate_record.owner_layer, "boundary_delivery_gate");
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Execute);
}

#[test]
fn existing_file_delivery_without_locator_hint_stays_clarify() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason = "clarify_reason_code:missing_delivery_locator".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint.clear();

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_missing_path_scoped_locator"
    );
}

#[test]
fn existing_file_delivery_with_fuzzy_candidates_stays_clarify() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason = "clarify_reason_code:missing_delivery_locator".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "report.txt".into();

    let result = apply_post_route_policy(
        route,
        LocatorResolution::Fuzzy(vec!["/tmp/report-a.txt".into(), "/tmp/report-b.txt".into()]),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_fuzzy_locator_candidates"
    );
}

#[test]
fn generated_file_delivery_current_workspace_without_hint_can_execute() {
    let mut route = route_result();
    route.resolved_intent =
        "Create a small generated note in the workspace and deliver the generated file".to_string();
    route.wants_file_delivery = true;
    route.output_contract.response_shape = OutputResponseShape::FileToken;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.delivery_required = true;
    route.output_contract.delivery_intent = OutputDeliveryIntent::FileSingle;
    route.output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert!(!result.missing_locator_for_path_scoped_content);
    assert_eq!(result.gate_record.reason_code, "post_route_no_change");
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::NoChange);
}

#[test]
fn current_workspace_scope_does_not_force_missing_locator_clarify() {
    let mut route = route_result();
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/workspace".to_string()),
    );
    assert_ne!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(!result.missing_locator_for_path_scoped_content);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/workspace"));
}

#[test]
fn sqlite_schema_version_current_workspace_directory_auto_locator_requires_clarify() {
    let mut route = route_result();
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::SqliteSchemaVersion;

    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-sqlite-dir-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();

    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.missing_locator_for_path_scoped_content);
    assert!(result.auto_locator_path.is_none());
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_missing_path_scoped_locator"
    );

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn current_workspace_content_excerpt_without_direct_locator_requires_clarify() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    route.output_contract.locator_hint = "model_io_log".to_string();

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.missing_locator_for_path_scoped_content);
    assert!(result
        .clarify_reason
        .contains("locator_required_for_path_scoped_content"));
}

#[test]
fn service_status_locator_hint_does_not_force_path_clarify() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "telegramd".to_string();
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert!(!result.missing_locator_for_path_scoped_content);
}

#[test]
fn content_evidence_without_runtime_locator_stays_chat() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: Default::default(),
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        content_evidence_execution_finalize_style(&contract, false),
        None
    );
}

#[test]
fn content_excerpt_summary_without_runtime_locator_stays_chat() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: Default::default(),
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        content_evidence_execution_finalize_style(&contract, false),
        None
    );
}

#[test]
fn strict_list_contract_uses_plain_finalize_style_from_matrix() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: Default::default(),
        semantic_kind: OutputSemanticKind::FileNames,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        content_evidence_execution_finalize_style(&contract, false),
        Some(ActFinalizeStyle::Plain)
    );
}

#[test]
fn grounded_summary_contract_uses_chat_wrapped_finalize_style_from_matrix() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        delivery_intent: Default::default(),
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        ..IntentOutputContract::default()
    };

    assert_eq!(
        content_evidence_execution_finalize_style(&contract, false),
        Some(ActFinalizeStyle::ChatWrapped)
    );
}

#[test]
fn filename_scope_with_direct_auto_locator_rescues_clarify() {
    let mut route = route_result();
    route.needs_clarify = true;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/README.md".to_string()),
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/README.md"));
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_auto_locator_satisfied_path_scoped_content"
    );
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Execute);
}

#[test]
fn background_locator_clarify_is_satisfied_by_direct_auto_locator() {
    let mut route = route_result();
    route.needs_clarify = true;
    route.route_reason = "background_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/README.md".to_string()),
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/README.md"));
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_auto_locator_satisfied_background_clarify"
    );
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Execute);
}

#[test]
fn mutation_missing_source_is_not_satisfied_by_output_auto_locator() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_read_target; background_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-mutation-output-dir-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    route.output_contract.locator_hint = temp_dir.to_string_lossy().to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;

    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_boundary_clarify_required"
    );
    assert_eq!(result.gate_record.owner_layer, "boundary_clarify_gate");
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Clarify);
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn ordinary_semantic_clarify_defers_to_agent_loop() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.route_reason = "background_locator_requires_clarify".to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.response_shape = OutputResponseShape::Free;

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_semantic_clarify_deferred_to_agent_loop"
    );
    assert_eq!(result.gate_record.owner_layer, "agent_loop_semantic_defer");
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::NoChange);
}

#[test]
fn filesystem_mutation_with_matching_locator_hint_executes_despite_deictic_marker() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_read_target; background_locator_requires_clarify; deictic_bare_locator_requires_clarify"
            .to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "run/nl_eval_tmp/config_edit_smoke/config.toml".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route
        .output_contract
        .self_extension
        .structured_field_selector = Some("skills.skill_switches.config_edit_nl_smoke".to_string());

    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(
            "/home/guagua/rustclaw/run/nl_eval_tmp/config_edit_smoke/config.toml".to_string(),
        ),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_auto_locator_satisfied_background_clarify"
    );
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Execute);
}

#[test]
fn missing_read_target_content_excerpt_is_not_satisfied_by_default_auto_locator() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_read_target; background_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;

    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/README.md".to_string()),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/README.md"));
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_boundary_clarify_required"
    );
}

#[test]
fn document_heading_missing_read_target_with_matching_locator_hint_executes() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_read_target; background_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint =
        "scripts/nl_tests/fixtures/device_local/docs/service_notes.md".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::DocumentHeading;

    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(
            "/home/guagua/rustclaw/scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
                .to_string(),
        ),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_auto_locator_satisfied_background_clarify"
    );
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Execute);
}

#[test]
fn document_heading_missing_read_target_without_locator_hint_stays_clarify() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "clarify_reason_code:missing_read_target; background_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::DocumentHeading;

    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/service_notes.md".to_string()),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_boundary_clarify_required"
    );
}

#[test]
fn deictic_bare_locator_clarify_is_not_satisfied_by_direct_auto_locator() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason =
        "background_locator_requires_clarify; deictic_bare_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/document".to_string()),
    );

    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/document"));
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_boundary_clarify_required"
    );
    assert_eq!(result.gate_record.outcome, PostRoutePolicyOutcome::Clarify);
}

#[test]
fn background_content_summary_with_direct_file_auto_locator_executes() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason = "background_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/README.md".to_string()),
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/README.md"));
}

#[test]
fn background_content_summary_with_fuzzy_file_locator_still_clarifies() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.route_reason = "background_locator_requires_clarify".to_string();
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::ContentExcerptSummary;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Fuzzy(vec![
            "/tmp/README.md".to_string(),
            "/tmp/README.zh-CN.md".to_string(),
        ]),
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(result.fuzzy_locator_suggestions.len(), 2);
}

#[test]
fn current_workspace_auto_locator_rescues_clarify() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::clarify();
    route.needs_clarify = true;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/workspace".to_string()),
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_chat_wrapped()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/workspace"));
}

#[test]
fn inherited_operation_without_boundary_contract_defers_to_agent_loop_even_with_direct_locator() {
    let mut route = route_result();
    route.needs_clarify = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.requires_content_evidence = false;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/document".to_string()),
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/document"));
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_semantic_clarify_deferred_to_agent_loop"
    );
}

#[test]
fn explicit_relative_path_without_locator_hint_does_not_rescue_clarify_back_to_execution() {
    let mut route = route_result();
    route.needs_clarify = true;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
}

#[test]
fn explicit_relative_path_followup_without_locator_hint_stays_clarify() {
    let mut route = route_result();
    route.needs_clarify = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert!(result.execution_route_result.needs_clarify);
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::clarify()
    );
}

#[test]
fn inherited_operation_without_boundary_contract_defers_to_agent_loop_without_prior_clarify() {
    let mut route = route_result();
    route.needs_clarify = true;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.requires_content_evidence = false;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/restart_clawd_latest.sh".to_string()),
    );
    assert!(!result.execution_route_result.needs_clarify);
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_semantic_clarify_deferred_to_agent_loop"
    );
}

#[test]
fn file_like_content_request_keeps_semantic_kind_none_for_filename_locator() {
    let mut route = route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = OutputLocatorKind::Filename;
    route.output_contract.locator_hint = "README.md".to_string();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/README.md".to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
}

#[test]
fn directory_like_content_request_does_not_default_to_content_excerpt_summary() {
    let mut route = route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "docs".to_string();
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-dir-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn directory_like_chat_wrapped_execution_requires_content_evidence_without_forcing_semantic_kind() {
    let mut route = route_result();
    route.resolved_intent = "列出 docs 目录最近修改的两个文件，再判断这些是干什么的".to_string();
    route.set_planner_execute_finalize(ActFinalizeStyle::ChatWrapped);
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "docs".to_string();
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-dir-summary-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    assert!(
        result
            .execution_route_result
            .output_contract
            .requires_content_evidence
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn generic_directory_chat_wrapped_execution_no_longer_defaults_to_directory_purpose_summary() {
    let mut route = route_result();
    route.resolved_intent = "看看 docs 目录".to_string();
    route.set_planner_execute_finalize(ActFinalizeStyle::ChatWrapped);
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "docs".to_string();
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-generic-dir-summary-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    assert!(
        result
            .execution_route_result
            .output_contract
            .requires_content_evidence
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn act_directory_listing_does_not_default_to_directory_purpose_summary() {
    let mut route = route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "document".to_string();
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-dir-act-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn scalar_count_contract_is_cleared_for_non_scalar_shape() {
    let mut route = route_result();
    route.resolved_intent = "列出 document 目录下前 5 个文件名".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    assert_eq!(result.gate_record.owner_layer, "boundary_contract_gate");
    assert_eq!(
        result.gate_record.reason_code,
        "post_route_contract_refined"
    );
}

#[test]
fn contract_matrix_snapshot_uses_post_route_policy_execution_contract() {
    let mut route = route_result();
    route.resolved_intent = "列出 document 目录下前 5 个文件名".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let pre_snapshot =
        crate::contract_matrix::trace_snapshot_for_route(&route).expect("pre snapshot");
    assert_eq!(
        pre_snapshot
            .get("contract_match")
            .and_then(serde_json::Value::as_str),
        Some("scalar_count")
    );

    let result = apply_post_route_policy(route, LocatorResolution::None);
    let post_snapshot =
        crate::contract_matrix::trace_snapshot_for_route(&result.execution_route_result)
            .expect("post snapshot");

    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    assert_eq!(
        post_snapshot
            .get("semantic_kind")
            .and_then(serde_json::Value::as_str),
        Some("none")
    );
    assert_eq!(
        post_snapshot
            .get("contract_match")
            .and_then(serde_json::Value::as_str),
        Some("generic_path_content")
    );
}

#[test]
fn scalar_count_contract_stays_for_true_scalar_shape() {
    let mut route = route_result();
    route.resolved_intent = "当前目录下有几个文件".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-true-count-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarCount
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn scalar_count_contract_stays_for_one_sentence_shape() {
    let mut route = route_result();
    route.resolved_intent = "count current workspace files and explain briefly".to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-count-one-sentence-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarCount
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn scalar_count_contract_stays_for_strict_single_sentence_shape() {
    let mut route = route_result();
    route.resolved_intent =
        "count direct child files in the current workspace and return one sentence".to_string();
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.exact_sentence_count = Some(1);
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-count-strict-one-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarCount
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn bounded_filename_listing_no_longer_repairs_misclassified_scalar_contract() {
    let mut route = route_result();
    route.resolved_intent = "列出 logs 目录最近修改的 2 个文件名，只输出文件名".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarCount;
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-listing-names-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.response_shape,
        OutputResponseShape::Scalar
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarCount
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn scalar_path_only_contract_is_not_repaired_from_dotted_field_text() {
    let mut route = route_result();
    route.resolved_intent =
        "读取 /tmp/config.toml 中的 tools.allow_sudo 字段值，并只输出该值".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/config.toml".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/config.toml".to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarPathOnly
    );
}

#[test]
fn scalar_path_only_free_contract_no_longer_uses_listing_surface_repair() {
    let mut route = route_result();
    route.resolved_intent = "列出 logs 目录最近修改的 2 个文件名，只输出文件名".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "logs".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-scalar-path-listing-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarPathOnly
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn scalar_path_only_contract_stays_for_real_path_only_request() {
    let mut route = route_result();
    route.resolved_intent = "只输出 /tmp/config.toml 的绝对路径，不要解释".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/config.toml".to_string();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let result = apply_post_route_policy(
        route,
        LocatorResolution::Direct("/tmp/config.toml".to_string()),
    );
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarPathOnly
    );
}

#[test]
fn scalar_path_only_output_without_input_locator_can_execute() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent = "执行 which bash，只输出 bash 的路径".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;

    let result = apply_post_route_policy(route, LocatorResolution::None);

    assert!(!result.missing_locator_for_path_scoped_content);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarPathOnly
    );
    assert_eq!(
        result.execution_route_result.ask_mode,
        crate::AskMode::planner_execute_plain()
    );
}

#[test]
fn scalar_path_only_contract_is_cleared_when_no_locator_binding_exists() {
    let mut route = route_result();
    route.resolved_intent = "只输出当前机器 hostname".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    route.output_contract.locator_hint.clear();
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
}

#[test]
fn scalar_path_only_contract_stays_for_workspace_scope_without_locator() {
    let mut route = route_result();
    route.resolved_intent = "output only the current workspace scalar value".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = false;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.semantic_kind = OutputSemanticKind::ScalarPathOnly;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::ScalarPathOnly
    );
}

#[test]
fn one_sentence_command_summary_keeps_raw_command_output() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent =
        "执行 pwd 命令获取当前工作目录路径，然后用一句话简要解释这个路径大概是什么（只输出一句话）"
            .to_string();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::RawCommandOutput
    );
    assert!(result.execution_route_result.route_reason.trim().is_empty());
}

#[test]
fn direct_scalar_command_result_keeps_raw_command_output() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::planner_execute_plain();
    route.resolved_intent = "执行 pwd，只输出当前路径，不要解释".to_string();
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::RawCommandOutput
    );
}

#[test]
fn brief_command_explanation_no_longer_uses_surface_shape_to_clear_raw_output() {
    let mut route = route_result();
    route.ask_mode = crate::AskMode::planner_execute_chat_wrapped();
    route.resolved_intent = "run pwd, then briefly explain what this path is".to_string();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.requires_content_evidence = true;
    route.output_contract.semantic_kind = OutputSemanticKind::RawCommandOutput;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::RawCommandOutput
    );
}

#[test]
fn explicit_file_path_hint_keeps_semantic_kind_none_without_auto_locator() {
    let mut route = route_result();
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "/tmp/device_local/docs/release_checklist.md".to_string();
    let result = apply_post_route_policy(route, LocatorResolution::None);
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
}

#[test]
fn current_workspace_file_resolution_keeps_semantic_kind_none() {
    let mut route = route_result();
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    route.output_contract.locator_hint.clear();

    let temp_dir = std::env::temp_dir().join(format!(
        "clawd-post-route-policy-workspace-file-{}-{}",
        std::process::id(),
        crate::now_ts_u64()
    ));
    std::fs::create_dir_all(&temp_dir).unwrap();
    let readme_path = temp_dir.join("README.md");
    std::fs::write(&readme_path, "# title\n").unwrap();
    let resolved = readme_path
        .canonicalize()
        .unwrap_or_else(|_| readme_path.clone())
        .display()
        .to_string();

    let result = apply_post_route_policy(route, LocatorResolution::Direct(resolved));
    assert_eq!(
        result.execution_route_result.output_contract.semantic_kind,
        OutputSemanticKind::None
    );
    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn missing_path_scoped_locator_sets_structured_clarify_reason_kind() {
    let result = apply_post_route_policy(route_result(), LocatorResolution::None);
    assert_eq!(
        result.clarify_reason_kind,
        ClarifyReasonKind::MissingPathScopedLocator
    );
}

#[test]
fn fuzzy_locator_candidates_set_structured_clarify_reason_kind() {
    let result = apply_post_route_policy(
        route_result(),
        LocatorResolution::Fuzzy(vec!["/tmp/a".to_string(), "/tmp/b".to_string()]),
    );
    assert_eq!(
        result.clarify_reason_kind,
        ClarifyReasonKind::FuzzyLocatorCandidates
    );
}

#[test]
fn clarify_reason_kind_dispatch_tokens_keep_boundary_and_legacy_semantic_separate() {
    assert_eq!(
        ClarifyReasonKind::RouteReasonText.dispatch_event(),
        "legacy_semantic_clarify_compat"
    );
    assert_eq!(
        ClarifyReasonKind::RouteReasonText.dispatch_new_owner(),
        "agent_loop_terminal_clarify_pending"
    );
    assert_eq!(
        ClarifyReasonKind::RouteReasonText.dispatch_chosen_path(),
        "ask_pipeline_legacy_semantic_clarify_compat"
    );
    assert_eq!(
        ClarifyReasonKind::MissingPathScopedLocator.dispatch_event(),
        "clarify_boundary_shortcut"
    );
    assert_eq!(
        ClarifyReasonKind::MissingPathScopedLocator.dispatch_new_owner(),
        "boundary_clarify_gate"
    );
    assert_eq!(
        ClarifyReasonKind::FuzzyLocatorCandidates.dispatch_chosen_path(),
        "ask_pipeline_boundary_clarify_shortcut"
    );
}
