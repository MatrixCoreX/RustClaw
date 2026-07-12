use super::{
    IntentOutputContract, OutputDeliveryIntent, OutputLocatorKind, OutputResponseShape,
    OutputSemanticKind, TargetTaskPolicy, TurnType,
};

#[test]
fn active_task_output_table_refinement_emits_loop_context_hint() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize the release checklist".to_string()),
            last_primary_task_output: Some(
                "1. Build\n2. Run tests\n3. Publish release notes".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(
        super::active_task_mutation_loop_context_hint(
            "把结果改成 markdown table 输出",
            "",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            &IntentOutputContract::default(),
            None,
        ),
        Some("active_task_mutation_loop_context")
    );
}

#[test]
fn active_task_correct_emits_loop_context_hint() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Write one deployment note that mentions Python 3.10".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(
        super::active_task_mutation_loop_context_hint(
            "Correction: not Python 3.10, use Python 3.11",
            "",
            Some(&snapshot),
            Some(TurnType::TaskCorrect),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            &IntentOutputContract::default(),
            None,
        ),
        Some("active_task_mutation_loop_context")
    );
}

#[test]
fn active_task_invalid_turn_binding_context_uses_schema_tokens_not_user_phrases() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Write a short release note for RustClaw".to_string()),
            last_primary_task_output: Some(
                "RustClaw v0.1.7 ships with clearer configuration controls.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "Make it easier for non-technical readers.",
    );
    let raw = serde_json::json!({
        "turn_type": "response",
        "target_task_policy": "release_note_rewrite_non_technical"
    })
    .to_string();

    let context =
        super::active_task_invalid_turn_binding_context(&raw, Some(&snapshot), &surface, false)
            .unwrap();

    assert!(context.contains("active_task_invalid_turn_binding"));
    assert!(context.contains("turn_type_invalid: true"));
    assert!(context.contains("target_task_policy_invalid: true"));
}

#[test]
fn active_task_invalid_turn_binding_skips_recent_observed_judgment_contract() {
    let raw = serde_json::json!({
        "reason": "excerpt_kind_judgment",
        "turn_type": "response",
        "target_task_policy": "recent_result_selection",
        "output_contract": {
            "response_shape": "strict",
            "requires_content_evidence": true,
            "delivery_required": false,
            "locator_kind": "none",
            "delivery_intent": "none",
            "semantic_kind": "excerpt_kind_judgment",
            "locator_hint": ""
        }
    })
    .to_string();
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "Compare the previous observed excerpts and return the selected filename.",
    );
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Read service_notes.md first lines".to_string()),
            last_primary_task_output: Some("# Service Notes".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            op_kind: crate::followup_frame::FollowupOpKind::Read,
            bound_target: Some("/tmp/rustclaw/docs/service_notes.md".to_string()),
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(super::active_task_invalid_turn_binding_context(
        &raw,
        Some(&snapshot),
        &surface,
        false
    )
    .is_none());

    let snapshot_without_anchor = crate::conversation_state::ActiveSessionSnapshot {
        active_followup_frame: None,
        ..snapshot
    };
    let context = super::active_task_invalid_turn_binding_context(
        &raw,
        Some(&snapshot_without_anchor),
        &surface,
        false,
    )
    .expect("missing observed anchor should keep conservative token repair");
    assert!(context.contains("active_task_invalid_turn_binding"));
}

#[test]
fn replacement_pairs_remove_conflicting_required_old_literals() {
    let mut state_patch = Some(serde_json::json!({
        "replacement_pairs": [
            {"from": "1. Verify installation and PATH", "to": "1. Check PATH and installation"},
            {"from": "2. Check file ownership and permissions", "to": "2. Check ownership and permissions"}
        ],
        "required_content_literals": [
            "1. Verify installation and PATH",
            "2. Check file ownership and permissions",
            "3. Install missing packages"
        ],
        "forbidden_visible_literals": []
    }));

    let reason = super::repair_state_patch_replacement_literal_conflicts(&mut state_patch);
    let patch = state_patch.expect("patch");

    assert_eq!(
        reason,
        Some("state_patch_replacement_literal_conflict_repair")
    );
    assert_eq!(
        patch["required_content_literals"],
        serde_json::json!([
            "3. Install missing packages",
            "1. Check PATH and installation",
            "2. Check ownership and permissions"
        ])
    );
    assert_eq!(
        patch["forbidden_visible_literals"],
        serde_json::json!([
            "1. Verify installation and PATH",
            "2. Check file ownership and permissions"
        ])
    );
}

#[test]
fn replacement_pairs_seed_required_new_literals_even_without_old_required_conflict() {
    let mut state_patch = Some(serde_json::json!({
        "replacement_pairs": [
            {"from": "Python 3.10", "to": "Python 3.11"}
        ],
        "required_content_literals": [],
        "forbidden_visible_literals": []
    }));

    let reason = super::repair_state_patch_replacement_literal_conflicts(&mut state_patch);
    let patch = state_patch.expect("patch");

    assert_eq!(
        reason,
        Some("state_patch_replacement_literal_conflict_repair")
    );
    assert_eq!(
        patch["required_content_literals"],
        serde_json::json!(["Python 3.11"])
    );
    assert_eq!(
        patch["forbidden_visible_literals"],
        serde_json::json!(["Python 3.10"])
    );
}

#[test]
fn nested_primary_task_update_replacement_pairs_seed_required_and_forbidden_literals() {
    let mut state_patch = Some(serde_json::json!({
        "primary_task_update": {
            "active_task_ref": "last_primary_task_prompt",
            "revision": "replace runtime version",
            "replacement_pairs": {"from": "Python 3.10", "to": "Python 3.11"}
        }
    }));

    let reason = super::repair_state_patch_replacement_literal_conflicts(&mut state_patch);
    let patch = state_patch.expect("patch");

    assert_eq!(
        reason,
        Some("state_patch_replacement_literal_conflict_repair")
    );
    assert_eq!(
        patch["required_content_literals"],
        serde_json::json!(["Python 3.11"])
    );
    assert_eq!(
        patch["forbidden_visible_literals"],
        serde_json::json!(["Python 3.10"])
    );
}

#[test]
fn active_task_mutation_with_content_evidence_stays_executable() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize this repository".to_string()),
            last_primary_task_output: Some("It has a web UI and backend services.".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        semantic_kind: OutputSemanticKind::WorkspaceProjectSummary,
        ..IntentOutputContract::default()
    };
    assert_eq!(
        super::active_task_mutation_loop_context_hint(
            "Focus only on the UI part",
            "workspace_project_summary",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            &contract,
            None,
        ),
        None
    );
}

#[test]
fn unresolved_deictic_observation_clarify_is_not_downgraded_to_direct_answer() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
            last_primary_task_output: Some("需要一个具体文件目标。".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::None,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };
    let state_patch = serde_json::json!({
        "deictic_reference": {"target": "unresolved_prior_object"}
    });
    assert_eq!(
        super::active_task_scope_update_loop_context_hint(
            "看看那个文件最后 5 行",
            Some(&snapshot),
            Some(TurnType::TaskScopeUpdate),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            true,
            &contract,
            Some(&state_patch),
        ),
        None
    );
}

#[test]
fn structured_deictic_unresolved_target_blocks_non_chinese_pronoun_fallback_gap() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface("それの最後の2行を見せて");
    let contract = IntentOutputContract {
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let patch = serde_json::json!({
        "deictic_reference": {"target": "unresolved_prior_object"}
    });

    assert!(super::unresolved_deictic_observable_target_should_clarify(
        &surface,
        &contract,
        Some(&patch),
    ));
    assert!(!super::active_task_turn_can_reuse_semantic_patch(
        &surface,
        Some(&patch),
    ));
}

#[test]
fn structured_deictic_resolved_target_overrides_local_pronoun_fallback() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface("看看那个最后 5 行");
    let contract = IntentOutputContract {
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };
    let patch = serde_json::json!({
        "deictic_reference": {"target": "current_action_result"}
    });

    assert!(!super::unresolved_deictic_observable_target_should_clarify(
        &surface,
        &contract,
        Some(&patch),
    ));
}

#[test]
fn scope_refinement_repair_keeps_unresolved_deictic_observation_clarify() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我整理一个方案".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let mut turn_type = Some(TurnType::TaskRequest);
    let mut target_task_policy = Some(TargetTaskPolicy::Standalone);
    let mut finalize_style = crate::ActFinalizeStyle::Plain;
    let mut needs_clarify = true;
    let mut contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        delivery_required: false,
        locator_kind: OutputLocatorKind::None,
        delivery_intent: OutputDeliveryIntent::None,
        semantic_kind: OutputSemanticKind::None,
        locator_hint: String::new(),
        self_extension: Default::default(),
    };

    let reason = super::apply_active_task_scope_refinement_repair(
        "看看那个文件最后 5 行",
        "",
        Some(&snapshot),
        &mut turn_type,
        &mut target_task_policy,
        false,
        &mut finalize_style,
        &mut needs_clarify,
        super::ScheduleKind::None,
        false,
        &mut contract,
        None,
        false,
    );

    assert_eq!(reason, None);
    assert_eq!(turn_type, Some(TurnType::TaskRequest));
    assert_eq!(target_task_policy, Some(TargetTaskPolicy::Standalone));
    assert!(needs_clarify);
    assert!(contract.requires_content_evidence);
}

#[test]
fn active_task_output_refinement_clarify_is_resolved() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("Summarize this repository".to_string()),
            last_primary_task_output: Some(
                "The UI is a web-based frontend for RustClaw.".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(
        super::active_task_append_loop_context_hint(
            "Output a two-row markdown table",
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            true,
            &IntentOutputContract::default(),
            None,
        ),
        Some("active_task_append_loop_context")
    );
}

#[test]
fn active_task_append_clarify_without_output_is_resolved() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我写个方案".to_string()),
            last_primary_task_output: None,
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(
        super::active_task_append_loop_context_hint(
            "控制在 80 字内，只输出正文",
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            true,
            &IntentOutputContract::default(),
            None,
        ),
        Some("active_task_append_loop_context")
    );
}

#[test]
fn missing_active_text_append_requires_loop_context() {
    let mut needs_clarify = true;
    let mut clarify_question = "who is the beginner?".to_string();
    let mut finalize = crate::ActFinalizeStyle::Plain;
    let mut contract = super::IntentOutputContract::default();

    let reason = super::apply_missing_active_task_reuse_clarify(
        "make it beginner friendly",
        "",
        None,
        Some(super::TurnType::TaskAppend),
        Some(super::TargetTaskPolicy::ReuseActive),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize,
        &mut contract,
    );

    assert_eq!(reason, Some("missing_active_task_reuse_loop_needs_context"));
    assert!(needs_clarify);
    assert!(clarify_question.is_empty());
}

#[test]
fn missing_active_text_append_keeps_file_locator_clarify() {
    let mut needs_clarify = false;
    let mut clarify_question = String::new();
    let mut finalize = crate::ActFinalizeStyle::Plain;
    let mut contract = super::IntentOutputContract::default();

    let reason = super::apply_missing_active_task_reuse_clarify(
        "README.md",
        "",
        None,
        Some(super::TurnType::TaskAppend),
        Some(super::TargetTaskPolicy::ReuseActive),
        None,
        &mut needs_clarify,
        &mut clarify_question,
        &mut finalize,
        &mut contract,
    );

    assert_eq!(reason, Some("missing_active_task_reuse_requires_clarify"));
    assert!(needs_clarify);
}

#[test]
fn active_task_append_clarify_keeps_file_locator_guard() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
            last_primary_task_output: None,
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    assert_eq!(
        super::active_task_append_loop_context_hint(
            "README.md",
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            false,
            true,
            &IntentOutputContract::default(),
            None,
        ),
        None
    );
}

#[test]
fn bare_path_correction_can_fill_active_observable_task() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "读一下 configs/config.toml 里的名字字段，只输出值".to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskCorrect),
        Some(TargetTaskPolicy::ReuseActive),
        &contract,
    ));
}

#[test]
fn bare_path_clarify_with_observable_scalar_contract_can_fill_active_task() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some(
                "Extract the name field from the package file and output only the value"
                    .to_string(),
            ),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        None,
        None,
        &contract,
    ));
}

#[test]
fn bare_path_active_clarify_state_can_fill_standalone_task_request() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: None,
        active_followup_frame: None,
        active_clarify_state: Some(crate::clarify_state::ClarifyState {
            missing_slot: crate::clarify_state::ClarifyMissingSlot::Locator,
            pending_question: "Provide the file path".to_string(),
            candidate_targets: Vec::new(),
            delivery_required: false,
            output_shape: Some(OutputResponseShape::Scalar.as_str().to_string()),
            semantic_kind: Some(OutputSemanticKind::StructuredKeys.as_str().to_string()),
            source_request: "Find the name field in the package file and output only the value"
                .to_string(),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
        }),
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        requires_content_evidence: true,
        locator_kind: OutputLocatorKind::Path,
        locator_hint: "scripts/nl_tests/fixtures/device_local/package.json".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::Standalone),
        &contract,
    ));
}

#[test]
fn bare_filename_task_request_can_replace_active_existence_check() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("看看那个重启脚本在不在".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "restart_clawd_latest.sh".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        Some(TargetTaskPolicy::ReplaceActive),
        &contract,
    ));
}

#[test]
fn bare_path_with_executable_contract_can_fill_active_log_tail() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我看一下那个日志最近 20 行".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::Filename,
        locator_hint: "logs/clawd.log".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        None,
        None,
        &contract,
    ));
}

#[test]
fn bare_filename_can_replace_active_delivery_target() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("send the selected file".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: Some(crate::followup_frame::FollowupFrame {
            source_request: "send the selected file".to_string(),
            op_kind: crate::followup_frame::FollowupOpKind::Delivery,
            bound_target: Some("/tmp/old.md".to_string()),
            output_shape: Some(OutputResponseShape::FileToken.as_str().to_string()),
            source_task_id: "task-1".to_string(),
            updated_at_ts: 1,
            expires_at_ts: 2,
            ..crate::followup_frame::FollowupFrame::default()
        }),
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::FileToken,
        requires_content_evidence: true,
        delivery_required: true,
        delivery_intent: OutputDeliveryIntent::FileSingle,
        locator_kind: OutputLocatorKind::CurrentWorkspace,
        locator_hint: "README.md".to_string(),
        ..IntentOutputContract::default()
    };

    assert!(super::bare_path_only_input_can_fill_active_observable_task(
        Some(&snapshot),
        Some(TurnType::TaskRequest),
        None,
        &contract,
    ));
}

#[test]
fn bare_path_without_observable_contract_still_needs_action_clarify() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("帮我检查这个文件".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };

    assert!(
        !super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            &IntentOutputContract::default(),
        )
    );
}

#[test]
fn bare_path_semantic_kind_alone_does_not_fill_active_observable_task() {
    let snapshot = crate::conversation_state::ActiveSessionSnapshot {
        conversation_state: Some(crate::conversation_state::ConversationState {
            last_primary_task_prompt: Some("检查上一个目标".to_string()),
            ..crate::conversation_state::ConversationState::default()
        }),
        active_followup_frame: None,
        active_clarify_state: None,
        active_observed_facts: None,
    };
    let contract = IntentOutputContract {
        response_shape: OutputResponseShape::Free,
        requires_content_evidence: true,
        semantic_kind: OutputSemanticKind::ContentExcerptSummary,
        locator_kind: OutputLocatorKind::None,
        locator_hint: String::new(),
        ..IntentOutputContract::default()
    };

    assert!(
        !super::bare_path_only_input_can_fill_active_observable_task(
            Some(&snapshot),
            Some(TurnType::TaskAppend),
            Some(TargetTaskPolicy::ReuseActive),
            &contract,
        )
    );
}

#[test]
fn workspace_scope_listing_shape_is_not_treated_as_fileish_cue() {
    let surface =
        crate::intent::surface_signals::analyze_prompt_surface("看看当前目录有哪些顶层文件夹");
    assert!(!super::prompt_has_concrete_fileish_cue(&surface));
}

#[test]
fn simple_command_shape_is_not_treated_as_fileish_cue() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface("执行 pwd");
    assert!(!super::prompt_has_concrete_fileish_cue(&surface));
}

#[test]
fn locator_target_pair_still_counts_as_fileish_cue() {
    let surface = crate::intent::surface_signals::analyze_prompt_surface(
        "比较 README.md 和 AGENTS.md 哪个更大",
    );
    assert!(super::prompt_has_concrete_fileish_cue(&surface));
}
