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
    let out = resolve_clarify_followup("今天天气怎么样", Some("<none>"), Some(&frame), None, None);
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
fn persisted_read_frame_reuses_prior_operation_for_locator_only_reply() {
    let frame = crate::followup_frame::FollowupFrame {
        source_request: "去那个配置里找 app.name，只把值给我".to_string(),
        op_kind: crate::followup_frame::FollowupOpKind::Read,
        bound_target: Some("/tmp/device_local/configs/app_config.toml".to_string()),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
        ..crate::followup_frame::FollowupFrame::default()
    };
    let out = resolve_clarify_followup(
        "scripts/nl_tests/fixtures/device_local/configs/app_config.toml",
        Some("<none>"),
        Some(&frame),
        None,
        None,
    );
    match out {
        ClarifyFollowupResolution::LocatorReplyRewrite(hit) => {
            assert_eq!(
                hit.reason,
                crate::clarify_followup::ClarifyRewriteReason::FollowupLocatorReply
            );
            assert!(hit.resolved_intent.contains("app.name"));
            assert!(hit
                .resolved_intent
                .contains("scripts/nl_tests/fixtures/device_local/configs/app_config.toml"));
        }
        other => panic!("expected followup locator reply rewrite, got {other:?}"),
    }
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
fn active_user_input_clarify_state_rewrites_to_generic_waiting_context() {
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::UserInput,
        pending_question: "QUESTION".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: None,
        semantic_kind: None,
        source_request: "Help me draft a proposal".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let out = resolve_clarify_followup(
        "for executives",
        Some("<none>"),
        None,
        Some(&clarify_state),
        None,
    );
    match out {
        ClarifyFollowupResolution::NormalizerRewrite { rewritten_prompt } => {
            assert!(rewritten_prompt.contains("### ACTIVE_CLARIFY_FOLLOWUP"));
            assert!(rewritten_prompt.contains("\"kind\":\"active_clarify_followup\""));
            assert!(rewritten_prompt.contains("\"missing_slot\":\"user_input\""));
            assert!(rewritten_prompt.contains("Help me draft a proposal"));
            assert!(rewritten_prompt.contains("QUESTION"));
            assert!(rewritten_prompt.contains("for executives"));
        }
        other => panic!("expected generic waiting-context rewrite, got {other:?}"),
    }
}

#[test]
fn active_user_input_clarify_state_ignores_empty_reply() {
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::UserInput,
        pending_question: "QUESTION".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: false,
        output_shape: None,
        semantic_kind: None,
        source_request: "Help me draft a proposal".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let out = resolve_clarify_followup("   ", Some("<none>"), None, Some(&clarify_state), None);
    assert!(matches!(out, ClarifyFollowupResolution::None));
}

#[test]
fn active_clarify_state_does_not_treat_deictic_new_request_as_target_fill() {
    let clarify_state = crate::clarify_state::ClarifyState {
        missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
        pending_question: "LOCATOR_CLARIFY_PROMPT".to_string(),
        candidate_targets: Vec::new(),
        delivery_required: true,
        output_shape: Some(crate::OutputResponseShape::FileToken.as_str().to_string()),
        semantic_kind: None,
        source_request: "把那个 release checklist 发给我".to_string(),
        source_task_id: "task-1".to_string(),
        updated_at_ts: 1,
        expires_at_ts: 2,
    };
    let out = resolve_clarify_followup(
        "读一下那个 README 开头 3 行",
        Some("<none>"),
        None,
        Some(&clarify_state),
        None,
    );
    assert!(matches!(out, ClarifyFollowupResolution::None));
    assert!(!prompt_can_fill_active_clarify_target(
        "读一下那个 README 开头 3 行",
        Some(&clarify_state),
    ));
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
