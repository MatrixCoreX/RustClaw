use crate::clarify_followup::ClarifyLocatorReplyRewrite;

#[derive(Debug, Clone)]
pub(crate) enum ClarifyFollowupResolution {
    None,
    NormalizerRewrite { rewritten_prompt: String },
    LocatorReplyRewrite(ClarifyLocatorReplyRewrite),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FreshDeicticClarifyDecision {
    pub(crate) reason: &'static str,
}

pub(crate) fn immediate_prior_turn_was_clarify(last_turn_full: &str) -> bool {
    crate::clarify_followup::last_turn_was_clarify(last_turn_full)
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_clarify_followup(
    prompt: &str,
    last_turn_full: Option<&str>,
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
) -> ClarifyFollowupResolution {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    resolve_clarify_followup_with_surface(
        prompt,
        last_turn_full,
        active_followup_frame,
        active_clarify_state,
        active_observed_facts,
        &surface,
    )
}

pub(crate) fn resolve_clarify_followup_with_surface(
    prompt: &str,
    last_turn_full: Option<&str>,
    active_followup_frame: Option<&crate::followup_frame::FollowupFrame>,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    _active_observed_facts: Option<&crate::observed_facts::ObservedFacts>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> ClarifyFollowupResolution {
    if let Some(state_hit) = active_clarify_state.and_then(|state| {
        synthesize_clarify_state_reply_resolution_with_surface(state, prompt, &surface)
    }) {
        return state_hit;
    }
    if let Some(frame_hit) = active_followup_frame.and_then(|frame| {
        crate::followup_frame::synthesize_locator_reply_resolved_intent(frame, prompt).map(
            |resolved_intent| crate::clarify_followup::ClarifyLocatorReplyRewrite {
                resolved_intent,
                prior_user_text: frame.source_request.clone(),
                current_user_text: prompt.trim().to_string(),
                reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
            },
        )
    }) {
        return ClarifyFollowupResolution::LocatorReplyRewrite(frame_hit);
    }
    let Some(last_turn_full) = last_turn_full else {
        return ClarifyFollowupResolution::None;
    };
    if !immediate_prior_turn_was_clarify(last_turn_full) {
        return ClarifyFollowupResolution::None;
    }
    if let Some(hit) = crate::clarify_followup::try_clarify_reply_rewrite(prompt, last_turn_full) {
        return ClarifyFollowupResolution::LocatorReplyRewrite(hit);
    }
    if !prompt_looks_like_clarify_target_only_with_surface(&surface) {
        return ClarifyFollowupResolution::None;
    }
    let Some(prior_user_text) = crate::clarify_followup::extract_prior_user_text(last_turn_full)
    else {
        return ClarifyFollowupResolution::None;
    };
    let rewritten_prompt = format!(
        "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target/content: {}",
        prior_user_text.trim(),
        prompt.trim()
    );
    ClarifyFollowupResolution::NormalizerRewrite { rewritten_prompt }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resolve_clarify_followup_from_session(
    prompt: &str,
    last_turn_full: Option<&str>,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> ClarifyFollowupResolution {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    resolve_clarify_followup_from_session_with_surface(
        prompt,
        last_turn_full,
        session_snapshot,
        &surface,
    )
}

pub(crate) fn resolve_clarify_followup_from_session_with_surface(
    prompt: &str,
    last_turn_full: Option<&str>,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> ClarifyFollowupResolution {
    let active_followup_frame =
        session_snapshot.and_then(|snapshot| snapshot.active_followup_frame.as_ref());
    let active_clarify_state =
        session_snapshot.and_then(|snapshot| snapshot.active_clarify_state.as_ref());
    let active_observed_facts =
        session_snapshot.and_then(|snapshot| snapshot.active_observed_facts.as_ref());
    resolve_clarify_followup_with_surface(
        prompt,
        last_turn_full,
        active_followup_frame,
        active_clarify_state,
        active_observed_facts,
        surface,
    )
}

pub(crate) fn context_contains_immediate_locator_anchor(text: &str) -> bool {
    fn locator_probe_text(text: &str) -> &str {
        let Some((_, rest)) = text.split_once("short_preview=") else {
            return text.trim();
        };
        rest.split_once(" has_code_block=")
            .map(|(preview, _)| preview.trim())
            .unwrap_or_else(|| rest.trim())
    }

    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed == "<none>" {
        return false;
    }
    if trimmed.starts_with("### RECENT_ASSISTANT_REPLIES") {
        let Some(immediate_line) = trimmed
            .lines()
            .find(|line| line.contains("turn_id=assistant[-1]"))
        else {
            return false;
        };
        let probe = locator_probe_text(immediate_line);
        return !crate::extract_delivery_file_tokens(probe).is_empty()
            || crate::delivery_utils::has_concrete_locator_input(probe);
    }
    let probe = locator_probe_text(trimmed);
    !crate::extract_delivery_file_tokens(probe).is_empty()
        || crate::delivery_utils::has_concrete_locator_input(probe)
}

#[allow(dead_code)]
pub(crate) fn resolve_fresh_deictic_clarify_guard(
    route_result: &crate::RouteResult,
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    last_turn_full: &str,
    recent_assistant_replies: &str,
) -> Option<FreshDeicticClarifyDecision> {
    if route_result.needs_clarify {
        return None;
    }
    let surface = super::surface_signals::analyze_prompt_surface(prompt);
    resolve_fresh_deictic_clarify_guard_with_surface(
        route_result,
        prompt,
        session_snapshot,
        last_turn_full,
        recent_assistant_replies,
        &surface,
    )
}

pub(crate) fn resolve_fresh_deictic_clarify_guard_with_surface(
    route_result: &crate::RouteResult,
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    last_turn_full: &str,
    recent_assistant_replies: &str,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> Option<FreshDeicticClarifyDecision> {
    if route_result.needs_clarify {
        return None;
    }
    let has_current_turn_generated_anchor =
        route_has_current_turn_generated_anchor(route_result, surface);
    let has_current_turn_filename_anchor =
        route_has_current_turn_filename_anchor(route_result, surface);
    let has_immediate_anchor =
        crate::conversation_state::session_alias_target_for_prompt(prompt, session_snapshot)
            .is_some()
            || session_contains_immediate_locator_anchor(session_snapshot)
            || context_contains_immediate_locator_anchor(last_turn_full)
            || context_contains_immediate_locator_anchor(recent_assistant_replies);
    let deictic_filename_wrapper =
        prompt_looks_like_deictic_filename_wrapper_with_surface(session_snapshot, &surface);
    if has_immediate_anchor
        || has_current_turn_generated_anchor
        || has_current_turn_filename_anchor
        || (surface_has_current_turn_locator(surface) && !deictic_filename_wrapper)
    {
        return None;
    }
    if !deictic_filename_wrapper {
        return None;
    }

    if route_result.wants_file_delivery {
        return Some(FreshDeicticClarifyDecision {
            reason: "fresh_delivery_deictic_requires_locator",
        });
    }
    if semantic_kind_allows_locator_free_fresh_turn(route_result.output_contract.semantic_kind) {
        return None;
    }

    if !route_result.output_contract.requires_content_evidence {
        return None;
    }
    if !matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Path
            | crate::OutputLocatorKind::Filename
            | crate::OutputLocatorKind::Url
    ) {
        return None;
    }

    match route_result.output_contract.response_shape {
        crate::OutputResponseShape::Scalar => Some(FreshDeicticClarifyDecision {
            reason: "fresh_scalar_deictic_requires_locator",
        }),
        crate::OutputResponseShape::Free
        | crate::OutputResponseShape::OneSentence
        | crate::OutputResponseShape::Strict => Some(FreshDeicticClarifyDecision {
            reason: "fresh_content_deictic_requires_locator",
        }),
        crate::OutputResponseShape::FileToken => None,
    }
}

#[allow(dead_code)]
pub(crate) fn fresh_deictic_guard_needs_recent_assistant_probe(
    route_result: &crate::RouteResult,
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    last_turn_full: &str,
) -> bool {
    if route_result.needs_clarify {
        return false;
    }
    let surface = super::surface_signals::analyze_prompt_surface(prompt);
    fresh_deictic_guard_needs_recent_assistant_probe_with_surface(
        route_result,
        prompt,
        session_snapshot,
        last_turn_full,
        &surface,
    )
}

pub(crate) fn fresh_deictic_guard_needs_recent_assistant_probe_with_surface(
    route_result: &crate::RouteResult,
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    last_turn_full: &str,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    if route_result.needs_clarify {
        return false;
    }
    let deictic_filename_wrapper =
        prompt_looks_like_deictic_filename_wrapper_with_surface(session_snapshot, &surface);
    if crate::conversation_state::session_alias_target_for_prompt(prompt, session_snapshot)
        .is_some()
        || context_contains_immediate_locator_anchor(last_turn_full)
        || session_contains_immediate_locator_anchor(session_snapshot)
        || route_has_current_turn_generated_anchor(route_result, surface)
        || route_has_current_turn_filename_anchor(route_result, surface)
        || (surface_has_current_turn_locator(surface) && !deictic_filename_wrapper)
    {
        return false;
    }
    if !deictic_filename_wrapper {
        return false;
    }
    if route_result.wants_file_delivery {
        return true;
    }
    if semantic_kind_allows_locator_free_fresh_turn(route_result.output_contract.semantic_kind) {
        return false;
    }
    route_result.output_contract.requires_content_evidence
        && matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::Path
                | crate::OutputLocatorKind::Filename
                | crate::OutputLocatorKind::Url
        )
        && matches!(
            route_result.output_contract.response_shape,
            crate::OutputResponseShape::Scalar
                | crate::OutputResponseShape::Free
                | crate::OutputResponseShape::OneSentence
        )
}

fn semantic_kind_allows_locator_free_fresh_turn(semantic_kind: crate::OutputSemanticKind) -> bool {
    matches!(semantic_kind, crate::OutputSemanticKind::ServiceStatus)
}

fn route_has_current_turn_generated_anchor(
    route_result: &crate::RouteResult,
    _surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    route_result.is_execute_gate()
        && route_result.output_contract.locator_hint.trim().is_empty()
        && (matches!(
            route_result.output_contract.semantic_kind,
            crate::OutputSemanticKind::RawCommandOutput
        ) || matches!(
            route_result.output_contract.locator_kind,
            crate::OutputLocatorKind::CurrentWorkspace
        ))
}

fn route_has_current_turn_filename_anchor(
    route_result: &crate::RouteResult,
    _surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    let locator_hint = route_result.output_contract.locator_hint.trim();
    matches!(
        route_result.output_contract.locator_kind,
        crate::OutputLocatorKind::Filename
    ) && !locator_hint.is_empty()
        && !locator_hint.contains('/')
        && !locator_hint.contains('\\')
}

fn surface_has_current_turn_locator(
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    surface.has_concrete_locator_hint()
        || surface.has_workspace_single_token_hint()
        || surface.has_single_filename_candidate()
        || surface.single_bare_filename_stem_candidate.is_some()
        || surface.workspace_child_directory_hint.is_some()
        || surface.directory_file_pair.is_some()
        || surface.compare_target_pair.is_some()
        || !surface.filename_candidates.is_empty()
}

pub(crate) fn session_contains_immediate_locator_anchor(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let Some(snapshot) = session_snapshot else {
        return false;
    };
    snapshot
        .active_followup_frame
        .as_ref()
        .is_some_and(|frame| {
            frame
                .bound_target
                .as_deref()
                .is_some_and(|target| !target.trim().is_empty())
        })
        || snapshot
            .active_observed_facts
            .as_ref()
            .is_some_and(|facts| {
                facts
                    .bound_target
                    .as_deref()
                    .is_some_and(|target| !target.trim().is_empty())
                    || !facts.delivery_targets.is_empty()
            })
}

#[allow(dead_code)]
pub(crate) fn prompt_looks_like_deictic_filename_wrapper(
    prompt: &str,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> bool {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    prompt_looks_like_deictic_filename_wrapper_with_surface(session_snapshot, &surface)
}

pub(crate) fn prompt_looks_like_deictic_filename_wrapper_with_surface(
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    if surface.token_count == 0 || surface.has_explicit_path_or_url() {
        return false;
    }
    if !surface.has_deictic_reference() {
        return false;
    }
    let has_explicit_filename_anchor = surface.has_single_filename_candidate()
        || surface.single_bare_filename_stem_candidate.is_some();
    if !has_explicit_filename_anchor {
        return false;
    }
    if !matches!(
        surface.deictic_prompt_shape,
        Some(super::surface_signals::DeicticPromptShape::ObjectTarget)
    ) {
        return false;
    }
    if surface.workspace_child_directory_hint.is_some() || surface.compare_target_pair.is_some() {
        return false;
    }
    if surface.inline_json_shape.is_some()
        || super::surface_signals::workspace_scope_shape_has_reference_scope(
            surface.workspace_scope_prompt_shape,
        )
    {
        return false;
    }
    let _ = session_snapshot;
    surface.has_concrete_locator_hint() || surface.has_generic_or_fileish_reference()
}

#[allow(dead_code)]
pub(crate) fn prompt_looks_like_active_clarify_reply(
    prompt: &str,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
) -> bool {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    prompt_looks_like_active_clarify_reply_with_surface(prompt, active_clarify_state, &surface)
}

pub(crate) fn prompt_looks_like_active_clarify_reply_with_surface(
    prompt: &str,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    let Some(_clarify_state) = active_clarify_state else {
        return false;
    };
    let _ = prompt;
    prompt_looks_like_clarify_target_only_with_surface(surface)
}

#[allow(dead_code)]
fn synthesize_clarify_state_reply_resolution(
    clarify_state: &crate::clarify_state::ClarifyState,
    prompt: &str,
) -> Option<ClarifyFollowupResolution> {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    synthesize_clarify_state_reply_resolution_with_surface(clarify_state, prompt, &surface)
}

fn synthesize_clarify_state_reply_resolution_with_surface(
    clarify_state: &crate::clarify_state::ClarifyState,
    prompt: &str,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> Option<ClarifyFollowupResolution> {
    match clarify_state.missing_slot {
        crate::clarify_state::ClarifyMissingSlot::Locator => {
            if surface.looks_like_locator_only_reply() {
                return Some(ClarifyFollowupResolution::LocatorReplyRewrite(
                    crate::clarify_followup::ClarifyLocatorReplyRewrite {
                        resolved_intent: format!(
                            "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target/content: {}",
                            clarify_state.source_request.trim(),
                            prompt.trim()
                        ),
                        prior_user_text: clarify_state.source_request.trim().to_string(),
                        current_user_text: prompt.trim().to_string(),
                        reason: crate::clarify_followup::ClarifyRewriteReason::ClarifyLocatorReply,
                    },
                ));
            }
            if prompt_looks_like_clarify_target_only_with_surface(surface) {
                return Some(ClarifyFollowupResolution::NormalizerRewrite {
                    rewritten_prompt: format!(
                        "Continue the previous request that was waiting for clarification: {}\nUser now provides the missing target/content: {}",
                        clarify_state.source_request.trim(),
                        prompt.trim()
                    ),
                });
            }
            None
        }
    }
}

pub(crate) fn prompt_looks_like_clarify_target_only_with_surface(
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    if surface.token_count == 0 {
        return false;
    }
    matches!(
        surface.inline_json_shape,
        Some(crate::intent::surface_signals::InlineJsonShape::WholeValue)
    ) || surface.has_any_locator_reference()
}

#[cfg(test)]
mod tests {
    use super::{
        context_contains_immediate_locator_anchor, immediate_prior_turn_was_clarify,
        prompt_looks_like_active_clarify_reply, prompt_looks_like_deictic_filename_wrapper,
        resolve_clarify_followup, resolve_clarify_followup_from_session,
        resolve_fresh_deictic_clarify_guard, ClarifyFollowupResolution,
    };

    #[test]
    fn immediate_last_turn_clarify_placeholder_is_detected() {
        assert!(immediate_prior_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 读一下那个文件里的名字字段，只输出值\nAssistant: [clarification_requested]\n[/TURN]"
        ));
        assert!(!immediate_prior_turn_was_clarify(
            "### LAST_TURN_FULL\n[TURN -1]\nUser: 看看那个重启脚本在不在\nAssistant: 有，路径：scripts/restart_clawd_latest.sh\n[/TURN]"
        ));
    }

    #[test]
    fn deictic_wrapper_ignores_inline_json_and_current_workspace_scope_requests() {
        assert!(!prompt_looks_like_deictic_filename_wrapper(
            r#"把这个 JSON 数组按 score 从高到低排一下，再输出成 markdown 表格：[{"name":"alpha","score":7},{"name":"beta","score":12}]"#,
            None,
        ));
        assert!(!prompt_looks_like_deictic_filename_wrapper(
            "这个目录里有没有那种点开头的隐藏文件？有的话随便举两个例子",
            None,
        ));
        assert!(!prompt_looks_like_deictic_filename_wrapper(
            "先看看当前目录有哪些顶层文件夹，再用一句适合新手的话解释这个仓库大概怎么组织",
            None,
        ));
    }

    #[test]
    fn clarify_followup_rewrites_previous_operation_for_non_locator_reply_target() {
        let out = resolve_clarify_followup(
            "就在 scripts/restart_clawd_latest.sh",
            Some("[LAST_TURN_FULL]\nUser: 把那个重启脚本发给我\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"),
            None,
            None,
            None,
        );
        match out {
            ClarifyFollowupResolution::NormalizerRewrite { rewritten_prompt } => {
                assert!(rewritten_prompt.contains("把那个重启脚本发给我"));
                assert!(rewritten_prompt.contains("就在 scripts/restart_clawd_latest.sh"));
            }
            other => panic!("expected normalizer rewrite, got {other:?}"),
        }
    }

    #[test]
    fn clarify_followup_prefers_locator_reply_rewrite_for_locator_reply() {
        let out = resolve_clarify_followup(
            "scripts/restart_clawd_latest.sh",
            Some("[LAST_TURN_FULL]\nUser: 看看那个重启脚本在不在\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"),
            None,
            None,
            None,
        );
        match out {
            ClarifyFollowupResolution::LocatorReplyRewrite(hit) => {
                assert_eq!(hit.current_user_text, "scripts/restart_clawd_latest.sh");
            }
            other => panic!("expected locator reply rewrite, got {other:?}"),
        }
    }

    #[test]
    fn clarify_followup_ignores_unrelated_new_request() {
        let out = resolve_clarify_followup(
            "今天天气怎么样",
            Some("[LAST_TURN_FULL]\nUser: 把那个 JSON 数组按 score 排一下并转成表格\nAssistant: [clarification_requested]\n[/LAST_TURN_FULL]"),
            None,
            None,
            None,
        );
        assert!(matches!(out, ClarifyFollowupResolution::None));
    }

    #[test]
    fn clarify_followup_prefers_persisted_followup_frame_for_locator_reply() {
        let frame = crate::followup_frame::FollowupFrame {
            source_request: "看一下那个 model io log 最后 4 行，再一句话说有什么现象".to_string(),
            unresolved_slot: Some(crate::followup_frame::FollowupUnresolvedSlot::Locator),
            ..crate::followup_frame::FollowupFrame::default()
        };
        let out = resolve_clarify_followup(
            "/tmp/device_local/logs/model_io.log",
            Some("<none>"),
            Some(&frame),
            None,
            None,
        );
        match out {
            ClarifyFollowupResolution::LocatorReplyRewrite(hit) => {
                assert!(hit.resolved_intent.contains("model io log 最后 4 行"));
                assert!(hit
                    .resolved_intent
                    .contains("/tmp/device_local/logs/model_io.log"));
            }
            other => panic!("expected frame-backed locator reply rewrite, got {other:?}"),
        }
    }

    #[test]
    fn clarify_followup_leaves_persisted_listing_scope_switch_to_normalizer() {
        let frame = crate::followup_frame::FollowupFrame {
            source_request: "先列出 document 目录下前 5 个文件名".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            ordered_entries: vec!["README.md".to_string(), "deploy.md".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        };
        let out = resolve_clarify_followup(
            "那 logs 目录下前 5 个文件名呢",
            Some("<none>"),
            Some(&frame),
            None,
            None,
        );
        assert!(matches!(out, ClarifyFollowupResolution::None));
    }

    #[test]
    fn clarify_followup_leaves_persisted_read_slice_change_to_normalizer() {
        let frame = crate::followup_frame::FollowupFrame {
            source_request: "看看 model_io.log 最后 5 行".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/device_local/logs/model_io.log".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        };
        let out = resolve_clarify_followup("最后 2 行", Some("<none>"), Some(&frame), None, None);
        assert!(matches!(out, ClarifyFollowupResolution::None));
    }

    #[test]
    fn clarify_followup_does_not_hijack_multi_clause_followup() {
        let frame = crate::followup_frame::FollowupFrame {
            source_request: "先列出 document 目录下前 5 个文件名".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::List,
            ordered_entries: vec!["README.md".to_string(), "deploy.md".to_string()],
            ..crate::followup_frame::FollowupFrame::default()
        };
        let out = resolve_clarify_followup(
            "那 logs 目录下前 5 个文件名呢，就第二个",
            Some("<none>"),
            Some(&frame),
            None,
            None,
        );
        assert!(matches!(out, ClarifyFollowupResolution::None));
    }

    #[test]
    fn clarify_followup_uses_active_clarify_state_when_last_turn_is_missing() {
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体要读取的文件名或路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "把那个重启脚本发给我".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        let out = resolve_clarify_followup(
            "就在 scripts/restart_clawd_latest.sh",
            Some("<none>"),
            None,
            Some(&clarify_state),
            None,
        );
        match out {
            ClarifyFollowupResolution::NormalizerRewrite { rewritten_prompt } => {
                assert!(rewritten_prompt.contains("把那个重启脚本发给我"));
                assert!(rewritten_prompt.contains("就在 scripts/restart_clawd_latest.sh"));
            }
            other => panic!("expected clarify-state rewrite, got {other:?}"),
        }
    }

    #[test]
    fn clarify_followup_uses_active_clarify_state_for_locator_reply_rewrite() {
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体要读取的文件名或路径。".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: Some(
                crate::OutputSemanticKind::ExistenceWithPath
                    .as_str()
                    .to_string(),
            ),
            source_request: "看一下那个重启脚本在不在".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        let out = resolve_clarify_followup(
            "scripts/restart_clawd_latest.sh",
            Some("<none>"),
            None,
            Some(&clarify_state),
            None,
        );
        match out {
            ClarifyFollowupResolution::LocatorReplyRewrite(hit) => {
                assert_eq!(hit.prior_user_text, "看一下那个重启脚本在不在");
                assert_eq!(hit.current_user_text, "scripts/restart_clawd_latest.sh");
            }
            other => panic!("expected clarify-state locator reply rewrite, got {other:?}"),
        }
    }

    #[test]
    fn active_clarify_reply_detector_does_not_hard_match_candidate_target_selection() {
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "请提供具体的文件名或路径。".to_string(),
            candidate_targets: vec!["act_plan.log".to_string(), "clawd.log".to_string()],
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "把那个文件发给我".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        assert!(!prompt_looks_like_active_clarify_reply(
            "第二个",
            Some(&clarify_state),
        ));
    }

    #[test]
    fn clarify_followup_from_session_snapshot_leaves_observed_facts_to_normalizer() {
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: Some(crate::observed_facts::ObservedFacts {
                bound_target: Some("/home/guagua/rustclaw/README.md".to_string()),
                output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
                ..crate::observed_facts::ObservedFacts::default()
            }),
        };
        let out = resolve_clarify_followup_from_session(
            "把这个文件再发一次",
            Some("<none>"),
            Some(&snapshot),
        );
        assert!(matches!(out, ClarifyFollowupResolution::None));
    }

    #[test]
    fn immediate_locator_anchor_ignores_older_assistant_replies() {
        assert!(!context_contains_immediate_locator_anchor(
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=好的，我来读取 has_code_block=false\n- turn_id=assistant[-2] short_preview=package.json has_code_block=false",
        ));
        assert!(context_contains_immediate_locator_anchor(
            "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=package.json has_code_block=false\n- turn_id=assistant[-2] short_preview=README.md has_code_block=false",
        ));
    }

    #[test]
    fn fresh_delivery_deictic_without_immediate_anchor_stays_with_normalizer() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "send the referenced file".to_string(),
            needs_clarify: false,
            route_reason: "recent_context_delivery_binding".to_string(),
            route_confidence: Some(0.83),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: true,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::FileToken,
                requires_content_evidence: false,
                delivery_required: true,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::FileSingle,
                semantic_kind: Default::default(),
                locator_hint: "config.toml".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(
            resolve_fresh_deictic_clarify_guard(
                &route,
                "把那个文件发给我",
                None,
                "<none>",
                "<none>",
            )
            .is_none(),
            "generic deictic delivery without an immediate anchor should stay on the normalizer/planner path"
        );
    }

    #[test]
    fn fresh_scalar_deictic_with_immediate_file_anchor_does_not_require_locator() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 package.json 文件中的 name 字段，只输出该字段的值".to_string(),
            needs_clarify: false,
            route_reason: "recent_context_scalar_binding".to_string(),
            route_confidence: Some(0.83),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Scalar,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "package.json".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(
            resolve_fresh_deictic_clarify_guard(
                &route,
                "读一下那个文件里的名字字段，只输出值",
                None,
                "<none>",
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=package.json has_code_block=false\n- turn_id=assistant[-2] short_preview=README.md has_code_block=false",
            )
            .is_none()
        );
    }

    #[test]
    fn fresh_content_deictic_without_immediate_anchor_stays_with_normalizer() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 model_io.log 最后 5 行".to_string(),
            needs_clarify: false,
            route_reason: "memory_established_path_binding".to_string(),
            route_confidence: Some(0.88),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(
            resolve_fresh_deictic_clarify_guard(
                &route,
                "看看那个模型日志最后 5 行",
                None,
                "<none>",
                "<none>",
            )
            .is_none(),
            "generic deictic content requests should not be hard-routed by continuation_resolver"
        );
    }

    #[test]
    fn deictic_filename_wrapper_without_immediate_anchor_requires_locator() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读取 /tmp/device_local/README.md 的开头部分，然后用一句话总结其内容"
                .to_string(),
            needs_clarify: false,
            route_reason: "memory_alias".to_string(),
            route_confidence: Some(0.85),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "/tmp/device_local/README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let out = resolve_fresh_deictic_clarify_guard(
            &route,
            "读一下那个 README 开头，然后一句话总结",
            None,
            "<none>",
            "<none>",
        )
        .expect("deictic filename wrapper should still require clarify");
        assert_eq!(out.reason, "fresh_content_deictic_requires_locator");
    }

    #[test]
    fn explicit_filename_plus_content_slice_does_not_require_locator() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读取 README 开头并用三句话总结".to_string(),
            needs_clarify: false,
            route_reason: "explicit_filename_content_slice".to_string(),
            route_confidence: Some(0.88),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: "README".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(resolve_fresh_deictic_clarify_guard(
            &route,
            "你先翻一下 README 开头那一小段，然后用 3 句话告诉我这个项目大概是干什么的",
            None,
            "<none>",
            "<none>",
        )
        .is_none());
    }

    #[test]
    fn explicit_under_directory_listing_judgment_does_not_require_locator() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "list the 2 most recently modified files under logs and then tell me whether this looks more like runtime logs or test leftovers"
                    .to_string(),
            needs_clarify: false,
            route_reason: "recent_artifacts_judgment".to_string(),
            route_confidence: Some(0.95),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::RecentArtifactsJudgment,
                locator_hint: "logs".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(
            resolve_fresh_deictic_clarify_guard(
                &route,
                "list the 2 most recently modified files under logs and then tell me whether this looks more like runtime logs or test leftovers",
                None,
                "<none>",
                "<none>",
            )
            .is_none(),
            "explicit current-turn child directory should suppress fresh deictic clarify"
        );
    }

    #[test]
    fn service_status_query_does_not_require_locator_even_when_locator_kind_drifted_to_path() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent:
                "list the listening ports on this machine and briefly tell me which ones matter most"
                    .to_string(),
            needs_clarify: false,
            route_reason: "service_status".to_string(),
            route_confidence: Some(0.91),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Path,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ServiceStatus,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(resolve_fresh_deictic_clarify_guard(
            &route,
            route.resolved_intent.as_str(),
            None,
            "<none>",
            "<none>",
        )
        .is_none());
        assert!(!super::fresh_deictic_guard_needs_recent_assistant_probe(
            &route,
            route.resolved_intent.as_str(),
            None,
            "<none>",
        ));
    }

    #[test]
    fn same_turn_command_output_reference_does_not_require_locator() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "执行 pwd，然后用一句话解释这个路径大概是什么".to_string(),
            needs_clarify: false,
            route_reason: String::new(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::CurrentWorkspace,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::None,
                locator_hint: String::new(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(resolve_fresh_deictic_clarify_guard(
            &route,
            route.resolved_intent.as_str(),
            None,
            "<none>",
            "<none>",
        )
        .is_none());
        assert!(!super::fresh_deictic_guard_needs_recent_assistant_probe(
            &route,
            route.resolved_intent.as_str(),
            None,
            "<none>",
        ));
    }

    #[test]
    fn fresh_deictic_probe_skips_recent_assistant_lookup_when_session_anchor_exists() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读取 /tmp/device_local/README.md 的开头部分，然后用一句话总结其内容"
                .to_string(),
            needs_clarify: false,
            route_reason: "memory_alias".to_string(),
            route_confidence: Some(0.85),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "/tmp/device_local/README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                bound_target: Some("/tmp/device_local/README.md".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(!super::fresh_deictic_guard_needs_recent_assistant_probe(
            &route,
            "读一下那个 README 开头，然后一句话总结",
            Some(&snapshot),
            "<none>",
        ));
    }

    #[test]
    fn fresh_deictic_probe_does_not_request_recent_assistant_lookup_for_generic_deictic() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 model_io.log 最后 5 行".to_string(),
            needs_clarify: false,
            route_reason: "memory_established_path_binding".to_string(),
            route_confidence: Some(0.88),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: Default::default(),
                locator_hint: "model_io.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(!super::fresh_deictic_guard_needs_recent_assistant_probe(
            &route,
            "看看那个模型日志最后 5 行",
            None,
            "<none>",
        ));
    }

    #[test]
    fn deictic_filename_wrapper_with_immediate_anchor_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读取 /tmp/device_local/README.md 的开头部分，然后用一句话总结其内容"
                .to_string(),
            needs_clarify: false,
            route_reason: "recent_context_binding".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        assert!(
            resolve_fresh_deictic_clarify_guard(
                &route,
                "读一下那个 README 开头，然后一句话总结",
                None,
                "<none>",
                "### RECENT_ASSISTANT_REPLIES\n- turn_id=assistant[-1] short_preview=README.md has_code_block=false",
            )
            .is_none()
        );
    }

    #[test]
    fn deictic_filename_wrapper_with_session_anchor_does_not_force_clarify() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::ChatAct,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::ChatAct),
            resolved_intent: "读取 /tmp/device_local/README.md 的开头部分，然后用一句话总结其内容"
                .to_string(),
            needs_clarify: false,
            route_reason: "session_context_binding".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::OneSentence,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "README.md".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: None,
            active_followup_frame: Some(crate::followup_frame::FollowupFrame {
                bound_target: Some("/tmp/device_local/README.md".to_string()),
                ..crate::followup_frame::FollowupFrame::default()
            }),
            active_clarify_state: None,
            active_observed_facts: Some(crate::observed_facts::ObservedFacts {
                bound_target: Some("/tmp/device_local/README.md".to_string()),
                ..crate::observed_facts::ObservedFacts::default()
            }),
        };
        assert!(resolve_fresh_deictic_clarify_guard(
            &route,
            "读一下那个 README 开头，然后一句话总结",
            Some(&snapshot),
            "<none>",
            "<none>",
        )
        .is_none());
    }

    #[test]
    fn session_alias_binding_counts_as_immediate_anchor() {
        let route = crate::RouteResult {
            routed_mode: crate::RoutedMode::Act,
            ask_mode: crate::AskMode::from_routed_mode(crate::RoutedMode::Act),
            resolved_intent: "读取 /tmp/device_local/logs/app.log 最后 20 行".to_string(),
            needs_clarify: false,
            route_reason: "session_context_binding".to_string(),
            route_confidence: Some(0.9),
            visible_skill_candidates: Vec::new(),
            risk_ceiling: crate::RiskCeiling::Unknown,
            resume_behavior: crate::ResumeBehavior::None,
            schedule_kind: crate::ScheduleKind::None,
            clarify_question: String::new(),
            schedule_intent: None,
            wants_file_delivery: false,
            should_refresh_long_term_memory: false,
            agent_display_name_hint: String::new(),
            output_contract: crate::IntentOutputContract {
                response_shape: crate::OutputResponseShape::Free,
                requires_content_evidence: true,
                delivery_required: false,
                locator_kind: crate::OutputLocatorKind::Filename,
                delivery_intent: crate::OutputDeliveryIntent::None,
                semantic_kind: crate::OutputSemanticKind::ContentExcerptSummary,
                locator_hint: "app.log".to_string(),
                self_extension: crate::SelfExtensionContract::default(),
            },
        };
        let snapshot = crate::conversation_state::ActiveSessionSnapshot {
            conversation_state: Some(crate::conversation_state::ConversationState {
                alias_bindings: vec![crate::conversation_state::SessionAliasBinding {
                    alias: "那个日志".to_string(),
                    target: "/tmp/device_local/logs/app.log".to_string(),
                    updated_at_ts: 1,
                }],
                ..crate::conversation_state::ConversationState::default()
            }),
            active_followup_frame: None,
            active_clarify_state: None,
            active_observed_facts: None,
        };
        assert!(resolve_fresh_deictic_clarify_guard(
            &route,
            "看一下那个日志最近 20 行",
            Some(&snapshot),
            "<none>",
            "<none>",
        )
        .is_none());
        assert!(!super::fresh_deictic_guard_needs_recent_assistant_probe(
            &route,
            "看一下那个日志最近 20 行",
            Some(&snapshot),
            "<none>",
        ));
    }
}
