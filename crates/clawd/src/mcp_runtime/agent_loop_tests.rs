use std::ffi::OsString;
use std::sync::Arc;

use serde_json::{json, Value};

use super::test_support::started_fixture_runtime;
use crate::fixture_replay_e2e::{fixture_env_lock, FixtureEnvGuard};
use crate::providers::fixture_replay::{
    clear_cache_for_test, RecordedCall, FIXTURE_CALLS_FILENAME, FIXTURE_LLM_SEQUENCE_FALLBACK_ENV,
};

struct EnvGuard {
    key: &'static str,
    previous: Option<OsString>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        if let Some(previous) = self.previous.take() {
            std::env::set_var(self.key, previous);
        } else {
            std::env::remove_var(self.key);
        }
    }
}

struct TempFixtureRoot {
    path: std::path::PathBuf,
}

impl TempFixtureRoot {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("rustclaw-mcp-agent-loop-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&path).expect("create fixture root");
        Self { path }
    }
}

impl Drop for TempFixtureRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn recorded_call(index: usize, source: &str, response: &str) -> RecordedCall {
    RecordedCall {
        prompt_hash: format!("{index:016x}"),
        prompt_source: Some(source.to_string()),
        prompt_preview: Some(source.to_string()),
        clean_response: response.to_string(),
        raw_response: Some(response.to_string()),
        usage: None,
    }
}

fn install_sequence_fixture(root: &std::path::Path, case: &str, responses: &[RecordedCall]) {
    let case_dir = root.join(case);
    std::fs::create_dir_all(&case_dir).expect("create fixture case");
    let body = responses
        .iter()
        .map(|response| serde_json::to_string(response).expect("serialize fixture response"))
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(case_dir.join(FIXTURE_CALLS_FILENAME), format!("{body}\n"))
        .expect("write fixture responses");
}

#[tokio::test]
async fn ordinary_agent_loop_executes_safe_mcp_capability_with_event_evidence() {
    let _fixture_lock = fixture_env_lock();
    clear_cache_for_test();
    let fixture_root = TempFixtureRoot::new();
    let case = "ordinary_mcp_agent_loop";
    let user_request =
        "Look up agent-loop-token with the available fixture tool and report the result.";
    let planner_response = json!({
        "steps": [
            {
                "type": "call_capability",
                "capability": "mcp.fixture.lookup",
                "args": {"query": "agent-loop-token"}
            },
            {"type": "synthesize_answer", "evidence_refs": []}
        ]
    })
    .to_string();
    let finalizer_response = json!({
        "answer": "The MCP fixture returned agent-loop-token from fixture.",
        "qualified": true,
        "needs_clarify": false,
        "is_meta_instruction": false,
        "publishable": true,
        "confidence": 0.99,
        "reason": "observed_structured_result"
    })
    .to_string();
    let responses = vec![
        recorded_call(1, "single_plan_execution_prompt", &planner_response),
        recorded_call(2, "observed_answer_fallback_prompt", &finalizer_response),
    ];
    install_sequence_fixture(&fixture_root.path, case, &responses);
    eprintln!("NL CASE: {user_request}");
    for (index, response) in responses.iter().enumerate() {
        eprintln!(
            "LLM return #{} raw_fields={}",
            index + 1,
            serde_json::to_string(response).expect("fixture raw fields")
        );
    }
    let _sequence_guard = EnvGuard::set(FIXTURE_LLM_SEQUENCE_FALLBACK_ENV, "1");
    let _fixture_guard = FixtureEnvGuard::install(&fixture_root.path, case);

    let runtime = started_fixture_runtime().await;
    let mut state = crate::AppState::test_default_with_fixture_provider()
        .with_prompt_layers_installed()
        .with_real_skill_registry()
        .with_real_runtime_policy()
        .with_seeded_db_schema();
    state.core.mcp_runtime = Arc::clone(&runtime);
    let task_id = format!("mcp-agent-loop-{}", uuid::Uuid::new_v4());
    let payload_json = json!({
        "text": user_request,
        "request_id": task_id,
        "thread_id": "mcp_agent_loop_thread",
    })
    .to_string();
    state.seed_ask_task_row(&task_id, 1, 1, &payload_json);
    state
        .core
        .db
        .get()
        .expect("task db")
        .execute(
            "UPDATE tasks SET status = 'running' WHERE task_id = ?1",
            rusqlite::params![task_id],
        )
        .expect("mark fixture task running");
    let task = crate::ClaimedTask {
        task_id: task_id.clone(),
        user_id: 1,
        chat_id: 1,
        user_key: Some("mcp-agent-loop-user".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "ask".to_string(),
        payload_json,
    };

    let reply =
        crate::agent_engine::run_agent_with_tools(&state, &task, user_request, user_request, None)
            .await
            .expect("ordinary MCP agent loop");

    assert!(!reply.should_fail_task, "error={:?}", reply.error_text);
    assert!(
        reply.text.contains("agent-loop-token"),
        "reply={}",
        reply.text
    );
    assert_eq!(state.task_llm_call_count(&task_id), 2);
    let journal = reply.task_journal.as_ref().expect("task journal");
    assert!(journal.rounds.iter().any(|round| {
        round.plan_result.as_ref().is_some_and(|plan| {
            serde_json::from_str::<Value>(&plan.raw_plan_text)
                .ok()
                .and_then(|raw| raw.get("steps").and_then(Value::as_array).cloned())
                .is_some_and(|steps| {
                    steps.iter().any(|step| {
                        step.get("type").and_then(Value::as_str) == Some("call_capability")
                            && step.get("capability").and_then(Value::as_str)
                                == Some("mcp.fixture.lookup")
                    })
                })
        })
    }));
    assert!(journal.step_results.iter().any(|step| {
        step.skill == "mcp.fixture.lookup"
            && step.status == crate::executor::StepExecutionStatus::Ok
    }));
    let mcp_observation = journal
        .task_observations
        .iter()
        .find(|observation| {
            observation.get("owner_layer").and_then(Value::as_str) == Some("mcp_runtime")
        })
        .expect("MCP task observation");
    assert_eq!(mcp_observation["capability"], "mcp.fixture.lookup");
    assert_eq!(mcp_observation["policy_decision"], "allow");
    assert_eq!(mcp_observation["status"], "ok");
    assert!(journal.event_stream_snapshot().iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("mcp_tool_call")
            && event.pointer("/payload/capability").and_then(Value::as_str)
                == Some("mcp.fixture.lookup")
    }));
    let audit_count: u64 = state
        .core
        .audit_db
        .get()
        .expect("audit db")
        .query_row(
            "SELECT COUNT(*) FROM audit_logs WHERE action = 'mcp.tool_call'",
            [],
            |row| row.get(0),
        )
        .expect("MCP audit count");
    assert_eq!(audit_count, 1);

    runtime.stop().await;
}
