use crate::{OutputResponseShape, RouteResult};

// Surface helper only. This module does not synthesize plans or route requests;
// planner-first execution owns semantic planning.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitScalarQueryKind {
    CurrentBranch,
    RecentCommitSubject,
}

fn detect_git_scalar_query_kind_from_text(user_text: &str) -> Option<GitScalarQueryKind> {
    let combined = user_text.trim();
    if combined.is_empty() {
        return None;
    }
    let lower = combined.to_ascii_lowercase();
    if !lower.contains("git") && !combined.contains("提交") && !combined.contains("分支") {
        return None;
    }
    let asks_branch = combined.contains("分支") || lower.contains("branch");
    let asks_commit = combined.contains("提交") || lower.contains("commit");
    let asks_recent = combined.contains("最近")
        || combined.contains("最新")
        || lower.contains("recent")
        || lower.contains("latest")
        || lower.contains("most recent");
    let asks_subject =
        combined.contains("标题") || lower.contains("title") || lower.contains("subject");
    if asks_branch && !asks_commit {
        return Some(GitScalarQueryKind::CurrentBranch);
    }
    if asks_commit && (asks_recent || asks_subject) {
        return Some(GitScalarQueryKind::RecentCommitSubject);
    }
    None
}

fn detect_git_scalar_query_kind(
    route_result: &RouteResult,
    user_text: &str,
) -> Option<GitScalarQueryKind> {
    if route_result.output_contract.response_shape != OutputResponseShape::Scalar
        || route_result.output_contract.delivery_required
        || route_result.output_contract.requires_content_evidence
    {
        return None;
    }
    let combined = if user_text.trim().is_empty() {
        route_result.resolved_intent.as_str()
    } else {
        user_text
    };
    detect_git_scalar_query_kind_from_text(combined)
}

pub(crate) fn route_has_git_scalar_surface(route_result: &RouteResult) -> bool {
    detect_git_scalar_query_kind(route_result, "").is_some()
}

pub(crate) fn text_has_git_scalar_surface(user_text: &str) -> bool {
    detect_git_scalar_query_kind_from_text(user_text).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntentOutputContract, OutputResponseShape, RoutedMode};

    fn scalar_route(text: &str) -> RouteResult {
        RouteResult {
            routed_mode: RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(RoutedMode::Act),
            resolved_intent: text.to_string(),
            needs_clarify: false,
            clarify_question: String::new(),
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Low,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: IntentOutputContract {
                response_shape: OutputResponseShape::Scalar,
                ..IntentOutputContract::default()
            },
            direct_reply_candidate: String::new(),
            direct_reply_confidence: 0.0,
        }
    }

    #[test]
    fn git_scalar_surface_recognizes_branch_and_recent_commit_subject() {
        assert!(text_has_git_scalar_surface(
            "what is the current git branch"
        ));
        assert!(text_has_git_scalar_surface("最近一次提交标题是什么"));
        assert!(route_has_git_scalar_surface(&scalar_route(
            "git branch name only"
        )));
        assert!(route_has_git_scalar_surface(&scalar_route(
            "latest git commit subject"
        )));
    }

    #[test]
    fn git_scalar_surface_does_not_claim_non_scalar_routes() {
        let mut route = scalar_route("latest git commit subject");
        route.output_contract.response_shape = OutputResponseShape::Free;
        assert!(!route_has_git_scalar_surface(&route));
        assert!(!text_has_git_scalar_surface(
            "summarize the repository"
        ));
    }
}
