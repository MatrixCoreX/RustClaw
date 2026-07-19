use serde_json::{json, Value};
use std::collections::BTreeSet;

struct FrontdoorFixture {
    name: &'static str,
    prompt: &'static str,
    payload: Value,
}

fn claimed_task(name: &str, payload: &Value) -> crate::ClaimedTask {
    crate::ClaimedTask {
        claim_attempt: 0,
        task_id: format!("frontdoor-fixture-{name}"),
        user_id: 41,
        chat_id: 73,
        user_key: Some("fixture-user".to_string()),
        channel: "test".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    }
}

fn fixtures() -> Vec<FrontdoorFixture> {
    vec![
        FrontdoorFixture { name: "zh_read", prompt: "读取工作区里的说明文档并总结。", payload: json!({}) },
        FrontdoorFixture { name: "en_write", prompt: "Create a short status note after inspecting the repository.", payload: json!({}) },
        FrontdoorFixture { name: "ja_question", prompt: "このプロジェクトの構成を説明してください。", payload: json!({}) },
        FrontdoorFixture { name: "ko_multi_step", prompt: "설정을 확인하고 문제를 수정한 다음 테스트하세요.", payload: json!({}) },
        FrontdoorFixture { name: "es_clarify_candidate", prompt: "Arregla el archivo que mencionamos antes.", payload: json!({}) },
        FrontdoorFixture { name: "fr_direct_answer", prompt: "Quelle est la différence entre une tâche et un thread ?", payload: json!({}) },
        FrontdoorFixture { name: "de_complex", prompt: "Prüfe den Code, ändere nur die betroffene Stelle und führe Tests aus.", payload: json!({}) },
        FrontdoorFixture { name: "empty_with_attachment", prompt: "", payload: json!({"attachments":[{"kind":"file","path":"docs/input.txt","mime_type":"text/plain","size":12}]}) },
        FrontdoorFixture { name: "text_with_image", prompt: "Compare this image with the current UI.", payload: json!({"attachments":[{"kind":"image","path":"data/ui/example.png","mime_type":"image/png","size":128}]}) },
        FrontdoorFixture { name: "multiple_attachments", prompt: "Summarize both inputs.", payload: json!({"attachments":[{"kind":"file","path":"docs/a.md"},{"kind":"file","path":"docs/b.md"}]}) },
        FrontdoorFixture { name: "explicit_path", prompt: "Inspect the supplied path.", payload: json!({"path":"crates/clawd/src/main.rs"}) },
        FrontdoorFixture { name: "explicit_paths", prompt: "Compare the supplied files.", payload: json!({"paths":["Cargo.toml","README.md"]}) },
        FrontdoorFixture { name: "workspace_path", prompt: "Check this workspace target.", payload: json!({"workspace_path":"crates/claw-core"}) },
        FrontdoorFixture { name: "url_locator", prompt: "Inspect the structured locator.", payload: json!({"locator":"https://example.invalid/status"}) },
        FrontdoorFixture { name: "machine_command_backticks", prompt: "Run `cargo check -p claw-core` and report the observed result.", payload: json!({}) },
        FrontdoorFixture { name: "machine_command_fence", prompt: "```bash\ncargo test -p claw-core\n```", payload: json!({}) },
        FrontdoorFixture { name: "thread_binding", prompt: "Continue the previous task.", payload: json!({"thread_id":"thread-17","session_id":"session-9"}) },
        FrontdoorFixture { name: "resume_binding", prompt: "Resume from the saved checkpoint.", payload: json!({"resume_task_id":"task-old","checkpoint_id":"checkpoint-3"}) },
        FrontdoorFixture { name: "workspace_binding", prompt: "Inspect the bound workspace.", payload: json!({"workspace_id":"workspace-2"}) },
        FrontdoorFixture { name: "permission_profile", prompt: "Apply the requested code change.", payload: json!({"permission_profile":"workspace_write","approval_policy":"on_risk"}) },
        FrontdoorFixture { name: "budget_profile", prompt: "Complete this long-running repository audit.", payload: json!({"budget_profile":"long_tail"}) },
        FrontdoorFixture { name: "source_field", prompt: "Answer using the task context.", payload: json!({"source":"ui_chat"}) },
        FrontdoorFixture { name: "conflicting_semantics", prompt: "Do not execute anything; explain what would happen, then decide whether clarification is needed.", payload: json!({}) },
        FrontdoorFixture { name: "compound_continuation", prompt: "Use the result from the previous turn, update the implementation, run focused tests, and stop if approval is required.", payload: json!({"thread_id":"thread-24","budget_profile":"adaptive"}) },
    ]
}

fn baseline_category(name: &str) -> Option<&'static str> {
    match name {
        "fr_direct_answer" => Some("simple_chat"),
        "zh_read" => Some("file_read"),
        "en_write" => Some("file_write"),
        "ko_multi_step" => Some("multi_step_coding"),
        "es_clarify_candidate" => Some("missing_required_argument"),
        "budget_profile" => Some("background_async_job"),
        _ => None,
    }
}

#[tokio::test]
async fn migration_fixtures_keep_semantics_inside_planner_and_make_zero_frontdoor_llm_calls() {
    let state = crate::AppState::test_default_with_fixture_provider();
    let fixtures = fixtures();
    assert_eq!(fixtures.len(), 24);
    let mut observed_baseline_categories = BTreeSet::new();

    for fixture in fixtures {
        if let Some(category) = baseline_category(fixture.name) {
            observed_baseline_categories.insert(category);
        }
        let task = claimed_task(fixture.name, &fixture.payload);
        let prepared = super::super::prepare_planner_owned_ask_routing(
            &state,
            &task,
            &fixture.payload,
            fixture.prompt,
            "fixture",
        )
        .await
        .unwrap_or_else(|err| panic!("fixture {} failed: {err}", fixture.name));

        assert!(!prepared.planner_user_request.trim().is_empty());
        assert_eq!(prepared.turn_boundary_envelope.task_id, task.task_id);
        assert_eq!(
            prepared.turn_boundary_envelope.raw_chars,
            fixture.prompt.chars().count()
        );
        assert_eq!(
            state.task_llm_call_count(&task.task_id),
            0,
            "{}",
            fixture.name
        );
        assert!(
            state.task_llm_call_sequence(&task.task_id).is_empty(),
            "{}",
            fixture.name
        );
    }
    assert_eq!(
        observed_baseline_categories,
        BTreeSet::from([
            "background_async_job",
            "file_read",
            "file_write",
            "missing_required_argument",
            "multi_step_coding",
            "simple_chat",
        ])
    );
}
