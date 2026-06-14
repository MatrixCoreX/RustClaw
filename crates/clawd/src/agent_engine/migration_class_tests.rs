use super::agent_decides_eligible_migration_class;
use crate::{
    AskMode, IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, ResumeBehavior, RiskCeiling, RouteResult, ScheduleKind,
};

fn route_result(shape: OutputResponseShape, kind: OutputSemanticKind) -> RouteResult {
    RouteResult {
        ask_mode: AskMode::planner_execute_chat_wrapped(),
        resolved_intent: "test".to_string(),
        needs_clarify: false,
        route_reason: String::new(),
        route_confidence: Some(0.9),
        visible_skill_candidates: Vec::new(),
        risk_ceiling: RiskCeiling::Low,
        resume_behavior: ResumeBehavior::None,
        schedule_kind: ScheduleKind::None,
        clarify_question: String::new(),
        schedule_intent: None,
        wants_file_delivery: false,
        should_refresh_long_term_memory: false,
        agent_display_name_hint: String::new(),
        output_contract: IntentOutputContract {
            exact_sentence_count: None,
            response_shape: shape,
            requires_content_evidence: true,
            delivery_required: false,
            locator_kind: OutputLocatorKind::Path,
            delivery_intent: OutputDeliveryIntent::None,
            semantic_kind: kind,
            locator_hint: "docs".to_string(),
            self_extension: crate::SelfExtensionContract::default(),
        },
    }
}

#[test]
fn exact_path_list_accepts_path_and_inventory_contracts() {
    for kind in [
        OutputSemanticKind::FilePaths,
        OutputSemanticKind::FileNames,
        OutputSemanticKind::DirectoryNames,
        OutputSemanticKind::DirectoryEntryGroups,
        OutputSemanticKind::HiddenEntriesCheck,
    ] {
        let route = route_result(OutputResponseShape::Strict, kind);

        assert_eq!(
            agent_decides_eligible_migration_class(&route),
            "exact_path_list"
        );
    }
}

#[test]
fn exact_path_list_requires_bound_locator_and_content_evidence() {
    let mut route = route_result(OutputResponseShape::Strict, OutputSemanticKind::FileNames);
    route.output_contract.locator_hint.clear();
    assert_eq!(agent_decides_eligible_migration_class(&route), "none");

    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.requires_content_evidence = false;
    assert_eq!(agent_decides_eligible_migration_class(&route), "none");
}

#[test]
fn bound_path_summary_accepts_grounded_summary_contracts() {
    for (shape, kind) in [
        (
            OutputResponseShape::Free,
            OutputSemanticKind::ContentExcerptSummary,
        ),
        (
            OutputResponseShape::OneSentence,
            OutputSemanticKind::ContentExcerptWithSummary,
        ),
        (
            OutputResponseShape::Strict,
            OutputSemanticKind::DirectoryPurposeSummary,
        ),
        (
            OutputResponseShape::Free,
            OutputSemanticKind::WorkspaceProjectSummary,
        ),
    ] {
        let route = route_result(shape, kind);

        assert_eq!(
            agent_decides_eligible_migration_class(&route),
            "bound_path_summary"
        );
    }
}

#[test]
fn bound_path_summary_requires_bound_locator_content_evidence_and_summary_shape() {
    let mut route = route_result(
        OutputResponseShape::Free,
        OutputSemanticKind::ContentExcerptSummary,
    );
    route.output_contract.locator_hint.clear();
    assert_eq!(agent_decides_eligible_migration_class(&route), "none");

    route.output_contract.locator_hint = "docs".to_string();
    route.output_contract.requires_content_evidence = false;
    assert_eq!(agent_decides_eligible_migration_class(&route), "none");

    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Scalar;
    assert_ne!(
        agent_decides_eligible_migration_class(&route),
        "bound_path_summary"
    );
}
