use super::*;

#[tokio::test]
async fn finalize_loop_reply_keeps_filesystem_mutation_lifecycle_synthesis_for_strict_contract() {
    let state = test_state();
    let task = claimed_task("task-filesystem-mutation-lifecycle");
    let mut route = free_route_result();
    route.resolved_intent = "filesystem mutation lifecycle".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Strict;
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/nl_codex_resume_smoke".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let synthesis = serde_json::json!({
        "schema_version": 1,
        "contract_marker": "filesystem_mutation_result",
        "status": "ok",
        "observed_action_count": 5,
        "observed_actions": [
            "make_dir",
            "write_text",
            "append_text",
            "read_range",
            "remove_path"
        ],
        "paths": [
            "tmp/nl_codex_resume_smoke",
            "tmp/nl_codex_resume_smoke/note.txt"
        ],
        "steps": [
            {"step_id": "step_1", "skill": "fs_basic", "status": "ok", "action": "make_dir", "path": "tmp/nl_codex_resume_smoke"},
            {"step_id": "step_2", "skill": "fs_basic", "status": "ok", "action": "write_text", "path": "tmp/nl_codex_resume_smoke/note.txt", "content_bytes": 6},
            {"step_id": "step_3", "skill": "fs_basic", "status": "ok", "action": "append_text", "path": "tmp/nl_codex_resume_smoke/note.txt", "content_bytes": 5},
            {"step_id": "step_4", "skill": "fs_basic", "status": "ok", "action": "read_range", "path": "tmp/nl_codex_resume_smoke/note.txt", "total_lines": 2},
            {"step_id": "step_5", "skill": "fs_basic", "status": "ok", "action": "remove_path", "path": "tmp/nl_codex_resume_smoke", "target_kind": "directory"}
        ],
        "readbacks": [
            {"step_id": "step_4", "path": "tmp/nl_codex_resume_smoke/note.txt", "excerpt": "1|alpha\n2|beta", "total_lines": 2}
        ],
        "final_state": {"cleanup_observed": true}
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(6);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(synthesis.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"make_dir","path":"tmp/nl_codex_resume_smoke"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"tmp/nl_codex_resume_smoke/note.txt","content_bytes":6}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"append_text","path":"tmp/nl_codex_resume_smoke/note.txt","content_bytes":5}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"tmp/nl_codex_resume_smoke/note.txt","excerpt":"1|alpha\n2|beta","total_lines":2}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"remove_path","path":"tmp/nl_codex_resume_smoke","target_kind":"directory"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_6",
        "synthesize_answer",
        &synthesis,
    ));
    let mut backfill_probe = loop_state.clone();
    backfill_delivery_from_last_outputs(&task, &mut backfill_probe, Some(&ctx));
    assert_eq!(
        backfill_probe.delivery_messages.last().map(String::as_str),
        Some(synthesis.as_str())
    );

    let reply = finalize_loop_reply(
        &state,
        &task,
        "run filesystem lifecycle and summarize structured results",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text, synthesis);
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some(synthesis.as_str())
    );
    let payload = serde_json::from_str::<serde_json::Value>(&reply.text).expect("json answer");
    assert_eq!(
        payload
            .pointer("/contract_marker")
            .and_then(serde_json::Value::as_str),
        Some("filesystem_mutation_result")
    );
    assert_eq!(
        payload
            .pointer("/observed_action_count")
            .and_then(serde_json::Value::as_i64),
        Some(5)
    );
    assert!(reply.text.contains("note.txt"));
    assert!(reply.text.contains("alpha"));
    assert!(reply.text.contains("remove_path"));
}

#[tokio::test]
async fn finalize_loop_reply_keeps_generic_lifecycle_shape_synthesis_as_structured_delivery() {
    let state = test_state();
    let task = claimed_task("task-generic-filesystem-lifecycle");
    let mut route = free_route_result();
    route.resolved_intent = "scratch filesystem lifecycle structured result".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/nl_codex_resume_smoke".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let synthesis = serde_json::json!({
        "schema_version": 1,
        "final_answer_shape": "lifecycle_result",
        "final_answer_shape_class": "verdict",
        "status": "ok",
        "observed_action_count": 5,
        "observed_actions": [
            "make_dir",
            "write_text",
            "append_text",
            "read_range",
            "remove_path"
        ],
        "paths": [
            "tmp/nl_codex_resume_smoke",
            "tmp/nl_codex_resume_smoke/note.txt"
        ],
        "steps": [
            {"step_id": "step_1", "skill": "fs_basic", "status": "ok", "action": "make_dir", "path": "tmp/nl_codex_resume_smoke"},
            {"step_id": "step_2", "skill": "fs_basic", "status": "ok", "action": "write_text", "path": "tmp/nl_codex_resume_smoke/note.txt", "content_bytes": 6},
            {"step_id": "step_3", "skill": "fs_basic", "status": "ok", "action": "append_text", "path": "tmp/nl_codex_resume_smoke/note.txt", "content_bytes": 4},
            {"step_id": "step_4", "skill": "fs_basic", "status": "ok", "action": "read_range", "path": "tmp/nl_codex_resume_smoke/note.txt", "total_lines": 2},
            {"step_id": "step_5", "skill": "fs_basic", "status": "ok", "action": "remove_path", "path": "tmp/nl_codex_resume_smoke", "target_kind": "directory"}
        ],
        "readbacks": [
            {"step_id": "step_4", "path": "tmp/nl_codex_resume_smoke/note.txt", "excerpt": "1|alpha\n2|beta", "total_lines": 2}
        ],
        "final_state": {"cleanup_observed": true}
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(6);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(synthesis.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "fs_basic",
        r#"{"extra":{"action":"make_dir","path":"tmp/nl_codex_resume_smoke"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "fs_basic",
        r#"{"extra":{"action":"write_text","path":"tmp/nl_codex_resume_smoke/note.txt","content_bytes":6}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_3",
        "fs_basic",
        r#"{"extra":{"action":"append_text","path":"tmp/nl_codex_resume_smoke/note.txt","content_bytes":4}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_4",
        "fs_basic",
        r#"{"extra":{"action":"read_range","path":"tmp/nl_codex_resume_smoke/note.txt","excerpt":"1|alpha\n2|beta","total_lines":2}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_5",
        "fs_basic",
        r#"{"extra":{"action":"remove_path","path":"tmp/nl_codex_resume_smoke","target_kind":"directory"}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_6",
        "synthesize_answer",
        &synthesis,
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "run generic filesystem lifecycle and summarize structured results",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(reply.text, synthesis);
    assert_eq!(
        reply.messages.last().map(String::as_str),
        Some(synthesis.as_str())
    );
    let payload = serde_json::from_str::<serde_json::Value>(&reply.text).expect("json answer");
    assert_eq!(
        payload
            .pointer("/final_answer_shape")
            .and_then(serde_json::Value::as_str),
        Some("lifecycle_result")
    );
    assert!(reply.text.contains("alpha"));
    assert!(reply.text.contains("beta"));
}

#[test]
fn generic_free_execute_route_accepts_complete_lifecycle_synthesis() {
    let mut route = free_route_result();
    route.resolved_intent = "scratch filesystem lifecycle structured result".to_string();
    route.output_contract.requires_content_evidence = false;
    route.output_contract.response_shape = OutputResponseShape::Free;
    route.output_contract.semantic_kind = OutputSemanticKind::None;
    route.output_contract.locator_kind = OutputLocatorKind::None;
    let synthesis = serde_json::json!({
        "schema_version": 1,
        "final_answer_shape": "lifecycle_result",
        "final_answer_shape_class": "verdict",
        "status": "ok",
        "observed_action_count": 5,
        "observed_actions": [
            "make_dir",
            "write_text",
            "append_text",
            "read_range",
            "remove_path"
        ],
        "steps": [
            {"step_id": "step_1", "skill": "fs_basic", "status": "ok", "action": "make_dir"}
        ],
        "final_state": {"cleanup_observed": true}
    })
    .to_string();

    assert!(
        crate::finalize::loop_reply::route_accepts_filesystem_mutation_synthesis(
            &route, &synthesis
        )
    );
}

#[tokio::test]
async fn finalize_loop_reply_uses_status_line_for_visible_filesystem_mutation_success() {
    let state = test_state();
    let task = claimed_task("task-kb-visible-filesystem-mutation");
    let mut route = free_route_result();
    route.resolved_intent = "kb ingest status".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace;
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let synthesis = serde_json::json!({
        "schema_version": 1,
        "contract_marker": "filesystem_mutation_result",
        "capability": "kb",
        "status": "ok",
        "effective_status": "ok",
        "effective_success": true,
        "idempotent_success": true,
        "result_kinds": ["already_indexed"],
        "observed_actions": ["ingest"],
        "namespaces": ["demo_docs_nl"],
        "paths": ["README.md"],
        "steps": [{
            "step_id": "step_1",
            "skill": "kb",
            "status": "ok",
            "effective_status": "ok",
            "effective_success": true,
            "idempotent_success": true,
            "action": "ingest",
            "namespace": "demo_docs_nl",
            "path": "README.md",
            "paths": ["README.md"],
            "result_kind": "already_indexed",
            "stats": {
                "ingested_docs": 0,
                "total_docs": 1,
                "total_chunks": 59,
                "unified_index_rows": 59,
                "unified_index_synced": true
            }
        }]
    })
    .to_string();
    let mut loop_state = crate::agent_engine::LoopState::new(3);
    loop_state.has_tool_or_skill_output = true;
    loop_state.last_publishable_synthesis_output = Some(synthesis.clone());
    loop_state.executed_step_results.push(ok_step_result(
        "step_1",
        "kb",
        r#"{"status":"ok","extra":{"action":"ingest","namespace":"demo_docs_nl","path":"README.md","effective_status":"ok","result_kind":"already_indexed","effective_success":true,"idempotent_success":true,"stats":{"total_chunks":59}}}"#,
    ));
    loop_state.executed_step_results.push(ok_step_result(
        "step_2",
        "synthesize_answer",
        &synthesis,
    ));

    let reply = finalize_loop_reply(
        &state,
        &task,
        "ingest README.md into demo_docs_nl",
        loop_state,
        Some(&ctx),
    )
    .await
    .expect("finalize should succeed");

    assert!(!reply.should_fail_task, "reply: {}", reply.text);
    assert_eq!(
        reply.text,
        "status=ok effective_status=ok result_kind=already_indexed action=ingest path=README.md namespace=demo_docs_nl total_chunks=59"
    );
    assert_eq!(
        reply
            .task_journal
            .as_ref()
            .and_then(|journal| journal.final_status),
        Some(crate::task_journal::TaskJournalFinalStatus::Success)
    );
}
