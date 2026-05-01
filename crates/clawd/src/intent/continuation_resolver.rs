use crate::clarify_followup::ClarifyLocatorReplyRewrite;

#[derive(Debug, Clone)]
pub(crate) enum ClarifyFollowupResolution {
    None,
    NormalizerRewrite { rewritten_prompt: String },
    LocatorReplyRewrite(ClarifyLocatorReplyRewrite),
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
    if !surface_has_structural_clarify_target_fill(&surface) {
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

#[allow(dead_code)]
pub(crate) fn prompt_can_fill_active_clarify_target(
    prompt: &str,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
) -> bool {
    let surface = super::surface_signals::analyze_prompt_surface(prompt.trim());
    surface_can_fill_active_clarify_target(prompt, active_clarify_state, &surface)
}

pub(crate) fn surface_can_fill_active_clarify_target(
    prompt: &str,
    active_clarify_state: Option<&crate::clarify_state::ClarifyState>,
    surface: &super::surface_signals::PromptSurfaceSignals,
) -> bool {
    let Some(clarify_state) = active_clarify_state else {
        return false;
    };
    let _ = prompt;
    if !clarify_state_has_structural_binding_contract(clarify_state) {
        return false;
    }
    surface_has_structural_clarify_target_fill(surface)
}

fn clarify_state_has_structural_binding_contract(
    clarify_state: &crate::clarify_state::ClarifyState,
) -> bool {
    clarify_state.delivery_required
        || clarify_state.output_shape.is_some()
        || clarify_state.semantic_kind.is_some()
        || !clarify_state.candidate_targets.is_empty()
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
            if !clarify_state_has_structural_binding_contract(clarify_state) {
                return None;
            }
            if surface.is_structural_locator_only_reply() {
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
            if surface_has_structural_clarify_target_fill(surface) {
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

pub(crate) fn surface_has_structural_clarify_target_fill(
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
        immediate_prior_turn_was_clarify, prompt_can_fill_active_clarify_target,
        resolve_clarify_followup, resolve_clarify_followup_from_session, ClarifyFollowupResolution,
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
    fn clarify_followup_does_not_rewrite_persisted_frame_for_unrelated_new_request() {
        let frame = crate::followup_frame::FollowupFrame {
            source_request: "看一下那个 model io log 最后 4 行，再一句话说有什么现象".to_string(),
            unresolved_slot: Some(crate::followup_frame::FollowupUnresolvedSlot::Locator),
            ..crate::followup_frame::FollowupFrame::default()
        };
        let out =
            resolve_clarify_followup("今天天气怎么样", Some("<none>"), Some(&frame), None, None);
        assert!(matches!(out, ClarifyFollowupResolution::None));
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
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
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
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
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
    fn weak_active_clarify_state_does_not_hijack_standalone_locator() {
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: None,
            semantic_kind: None,
            source_request: "logs".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        let out = resolve_clarify_followup(
            "document/",
            Some("<none>"),
            None,
            Some(&clarify_state),
            None,
        );
        assert!(matches!(out, ClarifyFollowupResolution::None));
        assert!(!prompt_can_fill_active_clarify_target(
            "document/",
            Some(&clarify_state),
        ));
    }

    #[test]
    fn active_clarify_reply_detector_does_not_hard_match_candidate_target_selection() {
        let clarify_state = crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
            candidate_targets: vec!["act_plan.log".to_string(), "clawd.log".to_string()],
            delivery_required: true,
            output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
            semantic_kind: None,
            source_request: "把那个文件发给我".to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        };
        assert!(!prompt_can_fill_active_clarify_target(
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
}
