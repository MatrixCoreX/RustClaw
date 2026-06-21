use super::*;

#[tokio::test]
async fn finalize_loop_reply_keeps_filesystem_mutation_lifecycle_synthesis() {
    let state = test_state();
    let task = claimed_task("task-filesystem-mutation-lifecycle");
    let mut route = free_route_result();
    route.resolved_intent = "filesystem mutation lifecycle".to_string();
    route.output_contract.requires_content_evidence = true;
    route.output_contract.response_shape = OutputResponseShape::OneSentence;
    route.output_contract.semantic_kind = OutputSemanticKind::FilesystemMutationResult;
    route.output_contract.locator_kind = OutputLocatorKind::Path;
    route.output_contract.locator_hint = "tmp/nl_codex_resume_smoke".to_string();
    let ctx = crate::agent_engine::AgentRunContext {
        route_result: Some(route),
        ..Default::default()
    };
    let synthesis = serde_json::json!({
        "schema_version": 1,
        "semantic_kind": "filesystem_mutation_result",
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
            .pointer("/semantic_kind")
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
