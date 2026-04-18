use crate::{
    IntentOutputContract, OutputLocatorKind, OutputResponseShape, OutputSemanticKind, RouteResult,
    RoutedMode,
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

fn path_is_existing_file(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && Path::new(trimmed).is_file()
}

fn path_is_existing_directory(path: &str) -> bool {
    let trimmed = path.trim();
    !trimmed.is_empty() && Path::new(trimmed).is_dir()
}

fn locator_hint_looks_file_like(locator_hint: &str) -> bool {
    let trimmed = locator_hint.trim();
    if trimmed.is_empty() {
        return false;
    }
    let path = Path::new(trimmed);
    path.extension().is_some()
        || path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("readme"))
}

/// 检查 normalizer 给的 `locator_hint` 字段里是否含有任意一个**真实存在**的绝对路径。
/// 用于支持别名/多目标场景：normalizer 把"甲/乙"等别名解析后写成
/// `"乙对应/abs/foo.md；甲对应/abs/bar.md"`，此时 raw_prompt 里只有别名、
/// `has_concrete_locator_hint(prompt)` 会返回 false，但 normalizer 自己已经
/// 给出了具体可执行的 path，post_route_policy 不应再强行触发 clarify。
fn locator_hint_contains_existing_absolute_path(locator_hint: &str) -> bool {
    let trimmed = locator_hint.trim();
    if trimmed.is_empty() {
        return false;
    }
    // locator_hint 可能是单个 path（简单形式），也可能是包含中英文标签的多 path 拼接，如：
    //   "/home/.../README.md"
    //   "乙对应/home/.../service_notes.md；甲对应/home/.../README.md"
    //   "乙: /home/.../foo.md, 甲: /home/.../bar.md"
    // 用一个宽松的拆分：按空白 / 逗号 / 分号 / 中文分号 / 冒号 / 中文冒号 切，
    // 然后挑出以 '/' 开头的 token，去掉首尾标点，逐个测 exists。
    let separators: &[char] = &[
        ' ', '\t', '\n', '\r', ',', '，', ';', '；', ':', '：', '"', '“', '”', '\'', '‘', '’',
        '(', ')', '（', '）', '[', ']', '【', '】',
    ];
    for token in trimmed.split(separators) {
        let token = token.trim_matches(|c: char| {
            !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-'
        });
        if token.starts_with('/') && Path::new(token).exists() {
            return true;
        }
    }
    false
}

fn should_default_to_content_excerpt_summary(
    route_result: &RouteResult,
    direct_locator_path: Option<&str>,
) -> bool {
    if route_result.output_contract.semantic_kind != OutputSemanticKind::None
        || route_result.output_contract.delivery_required
        || !route_result.output_contract.requires_content_evidence
        || !matches!(
            route_result.output_contract.response_shape,
            OutputResponseShape::Free | OutputResponseShape::OneSentence
        )
    {
        return false;
    }

    match route_result.output_contract.locator_kind {
        OutputLocatorKind::Filename => true,
        OutputLocatorKind::CurrentWorkspace => {
            direct_locator_path.is_some_and(path_is_existing_file)
        }
        OutputLocatorKind::Path => {
            direct_locator_path.is_some_and(path_is_existing_file)
                || locator_hint_looks_file_like(&route_result.output_contract.locator_hint)
        }
        _ => false,
    }
}

fn should_default_to_directory_purpose_summary(
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

pub(crate) fn apply_post_route_policy(
    route_result: RouteResult,
    raw_has_concrete_locator_hint: bool,
    resolved_has_concrete_locator_hint: bool,
    raw_has_explicit_path_locator_hint: bool,
    resolved_has_explicit_path_locator_hint: bool,
    resolved_intent_inherits_prior_operation: bool,
    immediate_prior_turn_was_clarify: bool,
    locator_resolution: LocatorResolution,
) -> PostRoutePolicyResult {
    let mut execution_route_result = route_result.clone();
    let path_scoped_content_request = route_result.output_contract.requires_content_evidence
        && locator_kind_requires_path_binding(route_result.output_contract.locator_kind);
    let mut auto_locator_path = None;
    let mut auto_locator_hint = None;
    let mut auto_locator_resolved_direct = false;
    let mut fuzzy_locator_suggestions = Vec::new();
    let normalizer_locator_hint_has_existing_path =
        locator_hint_contains_existing_absolute_path(&route_result.output_contract.locator_hint);
    let mut missing_locator_for_path_scoped_content = path_scoped_content_request
        && !locator_kind_is_current_workspace(route_result.output_contract.locator_kind)
        && !raw_has_concrete_locator_hint
        && !resolved_has_concrete_locator_hint
        && !normalizer_locator_hint_has_existing_path;

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

    if should_default_to_content_excerpt_summary(
        &execution_route_result,
        auto_locator_path.as_deref(),
    ) {
        execution_route_result.output_contract.semantic_kind =
            OutputSemanticKind::ContentExcerptSummary;
    } else if should_default_to_directory_purpose_summary(
        &execution_route_result,
        auto_locator_path.as_deref(),
    ) {
        execution_route_result.output_contract.semantic_kind =
            OutputSemanticKind::DirectoryPurposeSummary;
        execution_route_result
            .output_contract
            .requires_content_evidence = true;
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
        && immediate_prior_turn_was_clarify
        && matches!(execution_route_result.routed_mode, RoutedMode::AskClarify)
        && execution_route_result.needs_clarify;
    if inherited_operation_with_direct_locator {
        execution_route_result.needs_clarify = false;
        execution_route_result.routed_mode =
            if matches!(
                execution_route_result.output_contract.response_shape,
                OutputResponseShape::Scalar | OutputResponseShape::FileToken
            ) || execution_route_result.output_contract.delivery_required
            {
                RoutedMode::Act
            } else if execution_route_result
                .output_contract
                .requires_content_evidence
            {
                RoutedMode::ChatAct
            } else {
                RoutedMode::Act
            };
    }

    let explicit_path_requires_execution =
        matches!(execution_route_result.routed_mode, RoutedMode::AskClarify)
            && execution_route_result.needs_clarify
            && !auto_locator_resolved_direct
            && locator_kind_requires_path_binding(
                execution_route_result.output_contract.locator_kind,
            )
            && (raw_has_explicit_path_locator_hint || resolved_has_explicit_path_locator_hint)
            && (resolved_intent_inherits_prior_operation
                || execution_route_result
                    .output_contract
                    .requires_content_evidence
                || execution_route_result.output_contract.delivery_required
                || matches!(
                    execution_route_result.output_contract.response_shape,
                    OutputResponseShape::Scalar
                ));
    if explicit_path_requires_execution {
        execution_route_result.needs_clarify = false;
        execution_route_result.routed_mode = if execution_route_result
            .output_contract
            .delivery_required
            || matches!(
                execution_route_result.output_contract.response_shape,
                OutputResponseShape::Scalar | OutputResponseShape::FileToken
            ) {
            RoutedMode::Act
        } else if execution_route_result
            .output_contract
            .requires_content_evidence
        {
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
        let joined = fuzzy_locator_suggestions.join(" | ");
        if execution_route_result.route_reason.trim().is_empty() {
            (
                format!("fuzzy_locator_candidates={joined}"),
                ClarifyReasonKind::FuzzyLocatorCandidates,
            )
        } else {
            (
                format!(
                    "{}; fuzzy_locator_candidates={joined}",
                    execution_route_result.route_reason
                ),
                ClarifyReasonKind::FuzzyLocatorCandidates,
            )
        }
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
        IntentOutputContract, OutputLocatorKind, OutputResponseShape, ResumeBehavior, RiskCeiling,
        ScheduleKind,
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
                response_shape: OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: OutputLocatorKind::Path,
                delivery_intent: Default::default(),
                semantic_kind: Default::default(),
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn fuzzy_candidates_force_clarify_for_locator_requests() {
        let result = apply_post_route_policy(
            route_result(),
            true,
            true,
            false,
            false,
            false,
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
        let result = apply_post_route_policy(
            route_result(),
            false,
            false,
            false,
            false,
            false,
            false,
            LocatorResolution::None,
        );
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
            false,
            false,
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
            false,
            false,
            true,
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

    #[test]
    fn explicit_relative_path_can_rescue_ask_clarify_back_to_execution() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.requires_content_evidence = true;
        route.output_contract.response_shape = OutputResponseShape::Free;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        let result = apply_post_route_policy(
            route,
            true,
            true,
            true,
            true,
            false,
            false,
            LocatorResolution::None,
        );
        assert!(!result.execution_route_result.needs_clarify);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::ChatAct
        ));
    }

    #[test]
    fn explicit_relative_path_followup_rescues_scalar_binding_execution() {
        let mut route = route_result();
        route.routed_mode = RoutedMode::AskClarify;
        route.needs_clarify = true;
        route.output_contract.response_shape = OutputResponseShape::Scalar;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        let result = apply_post_route_policy(
            route,
            true,
            true,
            true,
            true,
            true,
            false,
            LocatorResolution::None,
        );
        assert!(!result.execution_route_result.needs_clarify);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::Act
        ));
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
            false,
            false,
            false,
            false,
            true,
            false,
            LocatorResolution::Direct("/tmp/restart_clawd_latest.sh".to_string()),
        );
        assert!(result.execution_route_result.needs_clarify);
        assert!(matches!(
            result.execution_route_result.routed_mode,
            RoutedMode::AskClarify
        ));
    }

    #[test]
    fn file_like_content_request_defaults_to_content_excerpt_summary_for_filename_locator() {
        let mut route = route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.locator_kind = OutputLocatorKind::Filename;
        route.output_contract.locator_hint = "README.md".to_string();
        let result = apply_post_route_policy(
            route,
            true,
            true,
            false,
            false,
            false,
            false,
            LocatorResolution::Direct("/tmp/README.md".to_string()),
        );
        assert_eq!(
            result.execution_route_result.output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptSummary
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
            true,
            true,
            false,
            false,
            false,
            false,
            LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
        );
        assert_eq!(
            result.execution_route_result.output_contract.semantic_kind,
            OutputSemanticKind::None
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn directory_like_chat_act_request_defaults_to_directory_purpose_summary() {
        let mut route = route_result();
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
            true,
            true,
            false,
            false,
            false,
            false,
            LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
        );
        assert_eq!(
            result.execution_route_result.output_contract.semantic_kind,
            OutputSemanticKind::DirectoryPurposeSummary
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
            true,
            true,
            false,
            false,
            false,
            false,
            LocatorResolution::Direct(temp_dir.to_string_lossy().to_string()),
        );
        assert_eq!(
            result.execution_route_result.output_contract.semantic_kind,
            OutputSemanticKind::None
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn explicit_file_path_hint_defaults_to_content_excerpt_summary_without_auto_locator() {
        let mut route = route_result();
        route.output_contract.response_shape = OutputResponseShape::OneSentence;
        route.output_contract.locator_kind = OutputLocatorKind::Path;
        route.output_contract.locator_hint =
            "/tmp/device_local/docs/release_checklist.md".to_string();
        let result = apply_post_route_policy(
            route,
            true,
            true,
            true,
            true,
            false,
            false,
            LocatorResolution::None,
        );
        assert_eq!(
            result.execution_route_result.output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptSummary
        );
    }

    #[test]
    fn current_workspace_file_resolution_defaults_to_content_excerpt_summary() {
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

        let result = apply_post_route_policy(
            route,
            false,
            false,
            false,
            false,
            false,
            false,
            LocatorResolution::Direct(resolved),
        );
        assert_eq!(
            result.execution_route_result.output_contract.semantic_kind,
            OutputSemanticKind::ContentExcerptSummary
        );
        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn missing_path_scoped_locator_sets_structured_clarify_reason_kind() {
        let result = apply_post_route_policy(
            route_result(),
            false,
            false,
            false,
            false,
            false,
            false,
            LocatorResolution::None,
        );
        assert_eq!(
            result.clarify_reason_kind,
            ClarifyReasonKind::MissingPathScopedLocator
        );
    }

    #[test]
    fn fuzzy_locator_candidates_set_structured_clarify_reason_kind() {
        let result = apply_post_route_policy(
            route_result(),
            true,
            true,
            false,
            false,
            false,
            false,
            LocatorResolution::Fuzzy(vec!["/tmp/a".to_string(), "/tmp/b".to_string()]),
        );
        assert_eq!(
            result.clarify_reason_kind,
            ClarifyReasonKind::FuzzyLocatorCandidates
        );
    }
}
