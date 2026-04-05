use crate::{
    IntentOutputContract, OutputLocatorKind, OutputResponseShape, RouteResult, RoutedMode,
};

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
}

pub(crate) fn enforce_content_evidence_execution_mode(
    mode: RoutedMode,
    contract: &IntentOutputContract,
    needs_clarify: bool,
) -> RoutedMode {
    if needs_clarify
        || mode != RoutedMode::Chat
        || !contract.requires_content_evidence
        || matches!(contract.locator_kind, OutputLocatorKind::None)
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

pub(crate) fn apply_post_route_policy(
    route_result: RouteResult,
    raw_has_concrete_locator_hint: bool,
    resolved_has_concrete_locator_hint: bool,
    resolved_intent_inherits_prior_operation: bool,
    locator_resolution: LocatorResolution,
) -> PostRoutePolicyResult {
    let mut execution_route_result = route_result.clone();
    let path_scoped_content_request = route_result.output_contract.requires_content_evidence
        && locator_kind_requires_path_binding(route_result.output_contract.locator_kind);
    let mut auto_locator_path = None;
    let mut auto_locator_hint = None;
    let mut auto_locator_resolved_direct = false;
    let mut fuzzy_locator_suggestions = Vec::new();
    let mut missing_locator_for_path_scoped_content = path_scoped_content_request
        && !locator_kind_is_current_workspace(route_result.output_contract.locator_kind)
        && !raw_has_concrete_locator_hint
        && !resolved_has_concrete_locator_hint;

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

    if auto_locator_resolved_direct && path_scoped_content_request {
        execution_route_result.needs_clarify = false;
        if matches!(
            execution_route_result.routed_mode,
            RoutedMode::AskClarify | RoutedMode::Chat
        ) {
            execution_route_result.routed_mode = if matches!(
                execution_route_result.output_contract.response_shape,
                OutputResponseShape::Scalar | OutputResponseShape::FileToken
            ) {
                RoutedMode::Act
            } else {
                RoutedMode::ChatAct
            };
        }
    }

    let inherited_operation_with_direct_locator = auto_locator_resolved_direct
        && resolved_intent_inherits_prior_operation
        && matches!(execution_route_result.routed_mode, RoutedMode::AskClarify)
        && execution_route_result.needs_clarify;
    if inherited_operation_with_direct_locator {
        execution_route_result.needs_clarify = false;
        execution_route_result.routed_mode = if matches!(
            execution_route_result.output_contract.response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::FileToken
        ) || execution_route_result.output_contract.delivery_required
        {
            RoutedMode::Act
        } else if execution_route_result.output_contract.requires_content_evidence {
            RoutedMode::ChatAct
        } else {
            RoutedMode::Act
        };
    }

    let fuzzy_locator_requires_clarify = !fuzzy_locator_suggestions.is_empty()
        && matches!(
            execution_route_result.output_contract.locator_kind,
            OutputLocatorKind::Path | OutputLocatorKind::Filename
        );
    let force_clarify = matches!(execution_route_result.routed_mode, RoutedMode::AskClarify)
        || (execution_route_result.needs_clarify && !auto_locator_resolved_direct)
        || missing_locator_for_path_scoped_content
        || fuzzy_locator_requires_clarify;
    if force_clarify {
        execution_route_result.needs_clarify = true;
        execution_route_result.routed_mode = RoutedMode::AskClarify;
    }

    let clarify_reason = if missing_locator_for_path_scoped_content {
        if execution_route_result.route_reason.trim().is_empty() {
            "missing_concrete_locator_for_path_scoped_content".to_string()
        } else {
            format!(
                "{}; missing_concrete_locator_for_path_scoped_content",
                execution_route_result.route_reason
            )
        }
    } else if !fuzzy_locator_suggestions.is_empty() {
        let joined = fuzzy_locator_suggestions.join(" | ");
        if execution_route_result.route_reason.trim().is_empty() {
            format!("fuzzy_locator_candidates={joined}")
        } else {
            format!(
                "{}; fuzzy_locator_candidates={joined}",
                execution_route_result.route_reason
            )
        }
    } else {
        execution_route_result.route_reason.clone()
    };

    PostRoutePolicyResult {
        execution_route_result,
        auto_locator_path,
        auto_locator_hint,
        auto_locator_resolved_direct,
        fuzzy_locator_suggestions,
        missing_locator_for_path_scoped_content,
        clarify_reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntentOutputContract, OutputLocatorKind, OutputResponseShape, ResumeBehavior, RiskCeiling,
        ScheduleKind,
    };

    fn route_result() -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::Act,
            resolved_intent: "test".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: RiskCeiling::Unknown,
            resume_behavior: ResumeBehavior::None,
            schedule_kind: ScheduleKind::None,
            wants_file_delivery: false,
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: Default::default(),
                locator_hint: String::new(),
            },
        }
    }

    #[test]
    fn fuzzy_candidates_force_clarify_for_locator_requests() {
        let result = apply_post_route_policy(
            route_result(),
            true,
            true,
            false,
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
        let result =
            apply_post_route_policy(route_result(), false, false, false, LocatorResolution::None);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert!(result.missing_locator_for_path_scoped_content);
    }

    #[test]
    fn current_workspace_scope_does_not_force_missing_locator_clarify() {
        let mut route = route_result();
        route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
        let result = apply_post_route_policy(
            route,
            false,
            false,
            false,
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
    fn filename_scope_with_direct_auto_locator_escalates_back_to_execution() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        let result = apply_post_route_policy(
            route,
            true,
            true,
            false,
            LocatorResolution::Direct("/tmp/README.md".to_string()),
        );
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::Act
        ));
        assert!(!result.execution_route_result.needs_clarify);
        assert!(!matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
        assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/README.md"));
    }

    #[test]
    fn inherited_operation_with_direct_locator_rescues_from_second_clarify() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.locator_kind = OutputLocatorKind::None;
        route.output_contract.requires_content_evidence = false;
        let result = apply_post_route_policy(
            route,
            false,
            false,
            true,
            LocatorResolution::Direct("/tmp/document".to_string()),
        );
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::Act
        ));
        assert!(!result.execution_route_result.needs_clarify);
        assert_eq!(result.auto_locator_path.as_deref(), Some("/tmp/document"));
    }
}
