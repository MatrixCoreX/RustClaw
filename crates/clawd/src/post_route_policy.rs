use crate::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, RouteResult, RoutedMode,
};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum ClarifyReasonKind {
    #[default]
    RouteReasonText,
    MissingPathScopedLocator,
    FuzzyLocatorCandidates,
}

#[derive(Debug, Clone)]
pub(crate) enum LocatorResolution {
    None,
    Direct(String),
    Fuzzy(Vec<String>),
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub(crate) struct PostRoutePolicyResult {
    pub(crate) execution_route_result: RouteResult,
    pub(crate) auto_locator_path: Option<String>,
    pub(crate) auto_locator_hint: Option<String>,
    pub(crate) auto_locator_resolved_direct: bool,
    pub(crate) fuzzy_locator_suggestions: Vec<String>,
    pub(crate) missing_locator_for_path_scoped_content: bool,
    pub(crate) clarify_reason: String,
    pub(crate) clarify_reason_kind: ClarifyReasonKind,
}

pub(crate) fn enforce_content_evidence_execution_mode(
    mode: RoutedMode,
    contract: &IntentOutputContract,
    needs_clarify: bool,
) -> RoutedMode {
    if needs_clarify || !mode.eq(&RoutedMode::Chat) || !contract.requires_content_evidence {
        return mode;
    }
    if matches!(contract.locator_kind, OutputLocatorKind::None)
        && !contract.delivery_required
        && !matches!(
            contract.response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::FileToken
        )
    {
        return mode;
    }
    if matches!(
        contract.response_shape,
        OutputResponseShape::Scalar | OutputResponseShape::FileToken
    ) {
        RoutedMode::Act
    } else {
        RoutedMode::ChatAct
    }
}

fn locator_kind_is_current_workspace(kind: OutputLocatorKind) -> bool {
    matches!(kind, OutputLocatorKind::CurrentWorkspace)
}

fn locator_kind_requires_path_binding(kind: OutputLocatorKind) -> bool {
    matches!(
        kind,
        OutputLocatorKind::Path | OutputLocatorKind::CurrentWorkspace | OutputLocatorKind::Filename
    )
}

fn semantic_locator_hint_satisfies_non_path_binding(route_result: &RouteResult) -> bool {
    route_result.output_contract.semantic_kind == OutputSemanticKind::ServiceStatus
        && !route_result.output_contract.locator_hint.trim().is_empty()
}

fn file_delivery_can_materialize_target_without_existing_locator(
    route_result: &RouteResult,
) -> bool {
    // New-file delivery may choose a filename during planning; an empty locator
    // hint is not necessarily a missing existing-file target.
    route_result.is_execute_gate()
        && !route_result.needs_clarify
        && route_result.wants_file_delivery
        && route_result.output_contract.delivery_required
        && route_result.output_contract.response_shape == OutputResponseShape::FileToken
        && route_result.output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && route_result.output_contract.requires_content_evidence
        && route_result.output_contract.semantic_kind == OutputSemanticKind::GeneratedFileDelivery
        && matches!(
            route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        )
        && route_result.output_contract.locator_hint.trim().is_empty()
}

fn path_is_existing_directory(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && Path::new(trimmed).is_dir()
}

fn should_force_content_evidence_for_path_bound_chat_act(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    if route_result.output_contract.semantic_kind != OutputSemanticKind::None
        || route_result.output_contract.delivery_required
        || route_result.routed_mode != RoutedMode::ChatAct
        || !matches!(
            route_result.output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        )
    {
        return false;
    }

    match route_result.output_contract.locator_kind {
        OutputLocatorKind::Path | OutputLocatorKind::CurrentWorkspace => {
            direct_locator_path.is_some_and(path_is_existing_directory)
        }
        _ => false,
    }
}

fn should_clear_scalar_count_for_non_scalar_contract(route_result: &RouteResult) -> bool {
    route_result.output_contract.semantic_kind == OutputSemanticKind::ScalarCount
        && route_result.output_contract.response_shape != OutputResponseShape::Scalar
}

fn should_clear_scalar_path_only_without_locator_binding(route_result: &RouteResult) -> bool {
    if route_result.output_contract.semantic_kind != OutputSemanticKind::ScalarPathOnly
        || route_result.output_contract.response_shape != OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
    {
        return false;
    }
    route_result.output_contract.locator_kind == OutputLocatorKind::None
        && route_result.output_contract.locator_hint.trim().is_empty()
}

fn should_clear_raw_command_output_for_contract_mismatch(route_result: &RouteResult) -> bool {
    if route_result.output_contract.semantic_kind != OutputSemanticKind::RawCommandOutput
        || route_result.output_contract.delivery_required
    {
        return false;
    }
    matches!(
        route_result.output_contract.response_shape,
        OutputResponseShape::OneSentence
    )
}

pub(crate) fn apply_post_route_policy(
    route_result: RouteResult,
    locator_resolution: LocatorResolution,
) -> PostRoutePolicyResult {
    let mut execution_route_result = route_result.clone();
    let path_scoped_content_request = route_result.output_contract.requires_content_evidence
        && locator_kind_requires_path_binding(route_result.output_contract.locator_kind)
        && !semantic_locator_hint_satisfies_non_path_binding(&route_result);
    let mut auto_locator_path = None;
    let mut auto_locator_hint = None;
    let mut auto_locator_resolved_direct = false;
    let mut fuzzy_locator_suggestions = Vec::new();
    let normalizer_locator_hint_present =
        !route_result.output_contract.locator_hint.trim().is_empty();
    let file_delivery_can_materialize_target =
        file_delivery_can_materialize_target_without_existing_locator(&route_result);
    let mut missing_locator_for_path_scoped_content = path_scoped_content_request
        && !locator_kind_is_current_workspace(route_result.output_contract.locator_kind)
        && !normalizer_locator_hint_present
        && !file_delivery_can_materialize_target;

    match locator_resolution {
        LocatorResolution::Direct(path) => {
            let locator_notice = if locator_kind_is_current_workspace(
                execution_route_result.output_contract.locator_kind,
            ) {
                format!(
                    "\n\n[AUTO_LOCATOR]\nResolved present workspace scope to: {path}\nUse this path as the target unless user explicitly overrides it.\n"
                )
            } else {
                format!(
                    "\n\n[AUTO_LOCATOR]\nResolved concrete path from default locator directory: {path}\nUse this path as the target unless user explicitly overrides it.\n"
                )
            };
            auto_locator_hint = Some(locator_notice);
            auto_locator_path = Some(path);
            auto_locator_resolved_direct = true;
            if missing_locator_for_path_scoped_content {
                missing_locator_for_path_scoped_content = false;
            }
        }
        LocatorResolution::Fuzzy(candidates) => {
            fuzzy_locator_suggestions = candidates;
        }
        LocatorResolution::None => {}
    }
    if !fuzzy_locator_suggestions.is_empty() {
        missing_locator_for_path_scoped_content = false;
    }

    if should_clear_scalar_count_for_non_scalar_contract(&execution_route_result) {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
    }
    if should_clear_scalar_path_only_without_locator_binding(&execution_route_result) {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
    }
    if should_clear_raw_command_output_for_contract_mismatch(&execution_route_result) {
        execution_route_result.output_contract.semantic_kind = OutputSemanticKind::None;
    }

    if should_force_content_evidence_for_path_bound_chat_act(
        &execution_route_result,
        auto_locator_path.as_deref(),
    ) {
        execution_route_result
            .output_contract
            .requires_content_evidence = true;
    }

    if auto_locator_resolved_direct
        && path_scoped_content_request
        && !execution_route_result.needs_clarify
    {
        execution_route_result.needs_clarify = false;
        if execution_route_result.is_clarify_gate() || execution_route_result.is_chat_gate() {
            execution_route_result.set_routed_mode(
                if matches!(
                    execution_route_result.output_contract.response_shape,
                    OutputResponseShape::Scalar | OutputResponseShape::FileToken
                ) {
                    RoutedMode::Act
                } else {
                    RoutedMode::ChatAct
                },
            );
        }
    }

    let fuzzy_locator_requires_clarify = !fuzzy_locator_suggestions.is_empty()
        && matches!(
            execution_route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        );
    let force_clarify = execution_route_result.is_clarify_gate()
        || (execution_route_result.needs_clarify && !auto_locator_resolved_direct)
        || missing_locator_for_path_scoped_content
        || fuzzy_locator_requires_clarify;
    if force_clarify {
        execution_route_result.needs_clarify = true;
        execution_route_result.set_routed_mode(RoutedMode::AskClarify);
    }

    let (clarify_reason, clarify_reason_kind) = if missing_locator_for_path_scoped_content {
        if execution_route_result.route_reason.trim().is_empty() {
            (
                "locator_required_for_path_scoped_content".to_string(),
                ClarifyReasonKind::MissingPathScopedLocator,
            )
        } else {
            (
                format!(
                    "{}; locator_required_for_path_scoped_content",
                    execution_route_result.route_reason
                ),
                ClarifyReasonKind::MissingPathScopedLocator,
            )
        }
    } else if !fuzzy_locator_suggestions.is_empty() {
        let reason = if execution_route_result.route_reason.trim().is_empty() {
            "fuzzy_locator_candidates".to_string()
        } else {
            execution_route_result.route_reason.clone()
        };
        (reason, ClarifyReasonKind::FuzzyLocatorCandidates)
    } else {
        (
            execution_route_result.route_reason.clone(),
            ClarifyReasonKind::RouteReasonText,
        )
    };

    PostRoutePolicyResult {
        execution_route_result,
        auto_locator_path,
        auto_locator_hint,
        auto_locator_resolved_direct,
        fuzzy_locator_suggestions,
        missing_locator_for_path_scoped_content,
        clarify_reason,
        clarify_reason_kind,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
        ResumeBehavior, RiskCeiling, ScheduleKind,
    };

    fn route_result() -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(RoutedMode::Act),
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
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert_eq!(result.fuzzy_locator_suggestions.len(), 2);
    }

    #[test]
    fn missing_locator_still_forces_clarify() {
        let result = apply_post_route_policy(route_result(), LocatorResolution::None);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert!(result.missing_locator_for_path_scoped_content);
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

        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::Act
        ));
        assert!(!result.execution_route_result.needs_clarify);
        assert!(!result.missing_locator_for_path_scoped_content);
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

        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::Act
        ));
        assert!(!result.execution_route_result.needs_clarify);
        assert!(!result.missing_locator_for_path_scoped_content);
    }

    #[test]
    fn current_workspace_scope_does_not_force_missing_locator_clarify() {
        let mut route = route_result();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        let result = apply_post_route_policy(
            route,
            LocatorResolution::Direct("/tmp/workspace".to_string()),
        );
        assert!(!matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert!(!result.missing_locator_for_path_scoped_content);
        assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/workspace"));
    }

    #[test]
    fn service_status_locator_hint_does_not_force_path_clarify() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::ChatAct);
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.semantic_kind = OutputSemanticKind::ServiceStatus;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint = "telegramd".to_string();
        let result = apply_post_route_policy(route, LocatorResolution::None);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::ChatAct
        ));
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
            enforce_content_evidence_execution_mode(RoutedMode::Chat, &contract, false),
            RoutedMode::Chat
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
            enforce_content_evidence_execution_mode(RoutedMode::Chat, &contract, false),
            RoutedMode::Chat
        );
    }

    #[test]
    fn filename_scope_with_direct_auto_locator_does_not_override_clarify() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        let result = apply_post_route_policy(
            route,
            LocatorResolution::Direct("/tmp/README.md".to_string()),
        );
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert!(result.execution_route_result.needs_clarify);
        assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/README.md"));
    }

    #[test]
    fn current_workspace_auto_locator_does_not_override_clarify() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::AskClarify);
        route.needs_clarify = true;
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        let result = apply_post_route_policy(
            route,
            LocatorResolution::Direct("/tmp/workspace".to_string()),
        );
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert!(result.execution_route_result.needs_clarify);
        assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/workspace"));
    }

    #[test]
    fn inherited_operation_with_direct_locator_no_longer_rescues_from_second_clarify() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.locator_kind = OutputLocatorKind::None;
        route.output_contract.requires_content_evidence = false;
        let result = apply_post_route_policy(
            route,
            LocatorResolution::Direct("/tmp/document".to_string()),
        );
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert!(result.execution_route_result.needs_clarify);
        assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/document"));
    }

    #[test]
    fn explicit_relative_path_without_locator_hint_does_not_rescue_clarify_back_to_execution() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        let result = apply_post_route_policy(route, LocatorResolution::None);
        assert!(result.execution_route_result.needs_clarify);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert_eq!(
            result.execution_route_result.ask_mode,
            crate::AskMode::from_routed_mode(RoutedMode::AskClarify)
        );
    }

    #[test]
    fn explicit_relative_path_followup_without_locator_hint_stays_clarify() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        let result = apply_post_route_policy(route, LocatorResolution::None);
        assert!(result.execution_route_result.needs_clarify);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert_eq!(
            result.execution_route_result.ask_mode,
            crate::AskMode::from_routed_mode(RoutedMode::AskClarify)
        );
    }

    #[test]
    fn inherited_operation_without_prior_clarify_stays_in_ask_clarify() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.locator_kind = OutputLocatorKind::None;
        route.output_contract.requires_content_evidence = false;
        let result = apply_post_route_policy(
            route,
            LocatorResolution::Direct("/tmp/restart_clawd_latest.sh".to_string()),
        );
        assert!(result.execution_route_result.needs_clarify);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
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
    fn directory_like_chat_act_request_requires_content_evidence_without_forcing_semantic_kind() {
        let mut route = route_result();
        route.resolved_intent =
            "列出 docs 目录最近修改的两个文件，再判断这些是干什么的".to_string();
        route.routed_mode = RoutedMode::ChatAct;
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
    fn generic_directory_chat_act_request_no_longer_defaults_to_directory_purpose_summary() {
        let mut route = route_result();
        route.resolved_intent = "看看 docs 目录".to_string();
        route.routed_mode = RoutedMode::ChatAct;
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
        route.routed_mode = RoutedMode::Act;
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
    fn one_sentence_command_plus_explanation_clears_raw_command_output() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::ChatAct);
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
            OutputSemanticKind::None
        );
        assert!(result.execution_route_result.route_reason.trim().is_empty());
    }

    #[test]
    fn direct_scalar_command_result_keeps_raw_command_output() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::Act;
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::Act);
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
        route.routed_mode = RoutedMode::ChatAct;
        route.ask_mode = crate::AskMode::from_routed_mode(RoutedMode::ChatAct);
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
        route.output_contract.locator_hint =
            "/tmp/device_local/docs/release_checklist.md".to_string();
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
}
