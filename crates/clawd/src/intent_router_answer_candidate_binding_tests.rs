// Answer-candidate binding tests for intent_router.

#[test]
fn answer_candidate_binding_reports_memory_only_without_phrase_matching() {
    let request = "Return the marker value only.";
    let answer = "client-like-continuous-20260501_054730";
    let memory_only = crate::task_context_builder::RouteContextView {
        memory_context: format!("#### RELEVANT_FACTS\n- remembered marker {answer}"),
        recent_assistant_replies: "<none>".to_string(),
        recent_turns_full: "<none>".to_string(),
        last_turn_full: "<none>".to_string(),
        recent_execution_context: "<none>".to_string(),
        ..Default::default()
    };

    let report = super::analyze_answer_candidate_binding(request, answer, &memory_only)
        .expect("candidate should produce binding report");
    assert!(report.is_memory_only_binding());
    assert!(report.is_distinctive());
    assert!(!report.in_current_request);

    let recent_bound = crate::task_context_builder::RouteContextView {
        memory_context: memory_only.memory_context,
        recent_assistant_replies: format!("已记录。测试编号 `{answer}` 已记住。"),
        ..Default::default()
    };

    let report = super::analyze_answer_candidate_binding(request, answer, &recent_bound)
        .expect("candidate should produce binding report");
    assert!(!report.is_memory_only_binding());
    assert!(report.has_current_or_recent_binding());
}

#[test]
fn recent_execution_bound_answer_candidate_does_not_trigger_active_text_conflict() {
    let recent_execution_bound = super::AnswerCandidateBindingReport {
        candidate: "README.md".to_string(),
        in_current_request: false,
        in_recent_assistant_replies: true,
        in_recent_turns_full: true,
        in_last_turn_full: false,
        in_recent_execution_context: true,
        in_memory_context: false,
    };
    assert!(
        !super::answer_candidate_can_conflict_with_active_text_followup(Some(
            &recent_execution_bound
        ))
    );

    let mut recent_text_only = recent_execution_bound.clone();
    recent_text_only.in_recent_execution_context = false;
    assert!(
        super::answer_candidate_can_conflict_with_active_text_followup(Some(&recent_text_only))
    );
}

#[test]
fn answer_candidate_binding_context_is_structural_not_language_specific() {
    let request = "For this continuous test, remember marker RC-CONT-EN-0428-B. Reply with one short confirmation.";
    let answer = "client-like-continuous-20260501_054730";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older marker {answer}"),
        ..Default::default()
    };

    let report = super::analyze_answer_candidate_binding(request, answer, &route_view)
        .expect("candidate should produce binding report");
    let context = super::answer_candidate_binding_repair_context(&report, true);
    assert!(context.contains("should_refresh_long_term_memory: true"));
    assert!(context.contains("memory_only_binding: true"));
    assert!(context.contains("distinctive_candidate: true"));
}

#[test]
fn memory_only_answer_candidate_clears_when_recent_context_has_conflicting_scalar() {
    let answer = "client-like-continuous-20260516_043255";
    let recent_marker = "RC-CONT-CN-0428-A";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older remembered marker {answer}"),
        recent_assistant_replies: format!("好的，已记住编号 {recent_marker}。"),
        recent_turns_full: format!(
            "### RECENT_TURNS_FULL\n[TURN -1]\nUser: remember {recent_marker}\nAssistant: {recent_marker}\n[/TURN]\n"
        ),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "Return the marker value only.",
        answer,
        &route_view,
    )
    .expect("candidate should produce binding report");
    let conflicts = super::recent_distinctive_scalar_conflict_tokens(&binding, &route_view);
    assert_eq!(conflicts, vec![recent_marker.to_string()]);

    let mut out: super::IntentNormalizerOut = serde_json::from_value(serde_json::json!({
        "resolved_user_intent": "recall marker",
        "answer_candidate": answer,
        "reason": "memory candidate",
        "decision": "direct_answer"
    }))
    .expect("valid normalizer out");
    assert_eq!(
        super::clear_memory_only_answer_candidate_if_recent_context_conflicts(
            &mut out,
            Some(&binding),
            &route_view,
        ),
        Some("memory_only_answer_candidate_recent_scalar_conflict_cleared")
    );
    assert!(out.answer_candidate.is_empty());
    assert!(out
        .reason
        .contains("memory_only_answer_candidate_recent_scalar_conflict_cleared"));
}

#[test]
fn internal_context_answer_candidate_is_cleared_before_route_output() {
    let mut out: super::IntentNormalizerOut = serde_json::from_value(serde_json::json!({
        "resolved_user_intent": "update alias and acknowledge",
        "answer_candidate": "Acknowledged.\n\n### SESSION_ALIAS_BINDINGS\n- alias: ALPHA_DOC\n  target: scripts/nl_tests/fixtures/device_local/docs/service_notes.md\n\n### ACTIVE_EXECUTION_ANCHOR\nfollowup_bound_target: scripts/nl_tests/fixtures/device_local/docs/service_notes.md",
        "reason": "normalizer_candidate",
        "decision": "direct_answer"
    }))
    .expect("valid normalizer out");

    assert_eq!(
        super::clear_internal_context_answer_candidate(&mut out),
        Some("internal_context_answer_candidate_cleared")
    );
    assert!(out.answer_candidate.is_empty());
    assert!(out
        .reason
        .contains("internal_context_answer_candidate_cleared"));
}

#[test]
fn memory_only_answer_candidate_does_not_clear_for_recent_paths_only() {
    let answer = "client-like-continuous-20260516_043255";
    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older remembered marker {answer}"),
        recent_turns_full: "### RECENT_TURNS_FULL\n[TURN -1]\nUser: read /tmp/report-2026.md\nAssistant: ok\n[/TURN]\n".to_string(),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "Return the marker value only.",
        answer,
        &route_view,
    )
    .expect("candidate should produce binding report");
    assert!(super::recent_distinctive_scalar_conflict_tokens(&binding, &route_view).is_empty());

    let mut out: super::IntentNormalizerOut = serde_json::from_value(serde_json::json!({
        "resolved_user_intent": "recall marker",
        "answer_candidate": answer,
        "reason": "memory candidate",
        "decision": "direct_answer"
    }))
    .expect("valid normalizer out");
    assert_eq!(
        super::clear_memory_only_answer_candidate_if_recent_context_conflicts(
            &mut out,
            Some(&binding),
            &route_view,
        ),
        None
    );
    assert_eq!(out.answer_candidate, answer);
}

#[test]
fn memory_only_answer_candidate_does_not_rebind_structured_id_scalar() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "recent-user-memory-scalar".to_string(),
        user_id: 91,
        chat_id: 202,
        user_key: Some("user:recent-memory-scalar".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"recall marker"}).to_string(),
    };
    let stale = "client-like-continuous-20260516_043255";
    let latest = "RC-CONT-CN-0428-A";
    {
        let db = state.core.db.get().expect("db");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            201,
            task.user_key.as_deref().unwrap(),
            1,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            stale,
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1000,
        )
        .expect("index stale memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            203,
            task.user_key.as_deref().unwrap(),
            2,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            &format!("已记住编号 {latest}。"),
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1010,
        )
        .expect("index latest memory");
    }

    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older remembered marker {stale}"),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "Return the marker value only.",
        stale,
        &route_view,
    )
    .expect("candidate should produce binding report");

    assert_eq!(
        super::latest_user_memory_distinctive_scalar_candidate(&state, &task, &binding),
        None
    );

    let mut out: super::IntentNormalizerOut = serde_json::from_value(serde_json::json!({
        "resolved_user_intent": "recall marker",
        "answer_candidate": stale,
        "reason": "memory candidate",
        "decision": "clarify",
        "needs_clarify": true,
        "clarify_question": "which marker?"
    }))
    .expect("valid normalizer out");
    assert_eq!(
        super::rebind_memory_only_answer_candidate_to_recent_user_memory(
            &state,
            &task,
            &mut out,
            Some(&binding),
        ),
        None
    );
    assert_eq!(out.answer_candidate, stale);
    assert_eq!(out.decision, "clarify");
    assert!(out.needs_clarify);
    assert_eq!(out.clarify_question, "which marker?");
}

#[test]
fn memory_only_answer_candidate_does_not_rebind_marker_to_recent_hostname() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "recent-hostname-no-marker-rebind".to_string(),
        user_id: 94,
        chat_id: 208,
        user_key: Some("user:recent-hostname-no-marker-rebind".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"recall marker"}).to_string(),
    };
    let marker = "RC-CONT-EN-0428-B";
    {
        let db = state.core.db.get().expect("db");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            task.chat_id,
            task.user_key.as_deref().unwrap(),
            1,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            "/home/guagua/rustclaw\nguagua\nThinkPad-X1",
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1010,
        )
        .expect("index hostname memory");
    }

    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("remembered marker {marker}"),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "Return the marker value only.",
        marker,
        &route_view,
    )
    .expect("candidate should produce binding report");

    assert_eq!(
        super::latest_user_memory_distinctive_scalar_candidate(&state, &task, &binding),
        None
    );
}

#[test]
fn memory_only_answer_candidate_rebinds_locator_only_to_latest_locator() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "recent-user-memory-locator".to_string(),
        user_id: 92,
        chat_id: 204,
        user_key: Some("user:recent-memory-locator".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"recall note file"}).to_string(),
    };
    let stale = "scripts/nl_tests/fixtures/device_local/docs/service_notes.md";
    let latest = "scripts/nl_tests/fixtures/device_local/docs/release_checklist.md";
    {
        let db = state.core.db.get().expect("db");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            301,
            task.user_key.as_deref().unwrap(),
            1,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            stale,
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1000,
        )
        .expect("index stale path memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            302,
            task.user_key.as_deref().unwrap(),
            2,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            &format!("note file -> {latest}"),
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1010,
        )
        .expect("index latest path memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            303,
            task.user_key.as_deref().unwrap(),
            3,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            "remembered marker RC-CONT-EN-0428-B",
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1020,
        )
        .expect("index unrelated marker memory");
    }

    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older note file mapping {stale}"),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "What file does the note file refer to now?",
        stale,
        &route_view,
    )
    .expect("candidate should produce binding report");

    assert_eq!(
        super::latest_user_memory_distinctive_scalar_candidate(&state, &task, &binding).as_deref(),
        Some(latest)
    );
}

#[test]
fn memory_only_answer_candidate_does_not_rebind_locator_to_marker() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = crate::ClaimedTask {
        task_id: "recent-user-memory-locator-no-cross-class".to_string(),
        user_id: 93,
        chat_id: 206,
        user_key: Some("user:recent-memory-locator-no-cross-class".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text":"recall note file"}).to_string(),
    };
    let stale = "scripts/nl_tests/fixtures/device_local/docs/service_notes.md";
    {
        let db = state.core.db.get().expect("db");
        crate::memory::indexing::ensure_retrieval_schema(&db).expect("retrieval schema");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            401,
            task.user_key.as_deref().unwrap(),
            1,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            stale,
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1000,
        )
        .expect("index stale path memory");
        crate::memory::indexing::index_memory_row(
            &db,
            task.user_id,
            402,
            task.user_key.as_deref().unwrap(),
            2,
            crate::memory::MEMORY_ROLE_ASSISTANT,
            "remembered marker RC-CONT-EN-0428-B",
            crate::memory::MEMORY_TYPE_ASSISTANT_REPLY,
            0.8,
            false,
            1010,
        )
        .expect("index marker memory");
    }

    let route_view = crate::task_context_builder::RouteContextView {
        memory_context: format!("older note file mapping {stale}"),
        ..Default::default()
    };
    let binding = super::analyze_answer_candidate_binding(
        "What file does the note file refer to now?",
        stale,
        &route_view,
    )
    .expect("candidate should produce binding report");

    assert_eq!(
        super::latest_user_memory_distinctive_scalar_candidate(&state, &task, &binding),
        None
    );
}
