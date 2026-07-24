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
    let discovery_response = json!({
        "steps": [
            {
                "type": "call_capability",
                "capability": "mcp.catalog.search",
                "args": {"query": "fixture lookup", "limit": 1}
            }
        ]
    })
    .to_string();
    let execution_response = json!({
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
    let terminal_response = json!({
        "steps": [
            {
                "type": "respond",
                "content": "The MCP fixture returned agent-loop-token from fixture."
            }
        ]
    })
    .to_string();
    let responses = vec![
        recorded_call(1, "single_plan_execution_prompt", &discovery_response),
        recorded_call(2, "loop_incremental_plan_prompt", &execution_response),
        recorded_call(3, "loop_incremental_plan_prompt", &terminal_response),
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
            "UPDATE tasks
             SET status = 'running',
                 lease_owner = ?2,
                 lease_expires_at = 9223372036854775807,
                 claim_attempt = 1,
                 claimed_at = 1
             WHERE task_id = ?1",
            rusqlite::params![task_id, state.worker.worker_id.as_str()],
        )
        .expect("mark fixture task running");
    let task = crate::ClaimedTask {
        claim_attempt: 1,
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

    let initial_observations = [json!({
        "schema_version": 1,
        "owner_layer": "agent_hooks",
        "stage": "pre_compact",
        "decision": "allow",
        "reason_code": "pre_compact_observed",
    })];
    let reply = crate::agent_engine::run_agent_with_tools(
        &state,
        &task,
        user_request,
        user_request,
        None,
        &initial_observations,
    )
    .await
    .expect("ordinary MCP agent loop");

    assert!(!reply.should_fail_task, "error={:?}", reply.error_text);
    assert!(
        reply.text.contains("agent-loop-token"),
        "reply={}",
        reply.text
    );
    assert_eq!(state.task_llm_call_count(&task_id), 3);
    let journal = reply.task_journal.as_ref().expect("task journal");
    assert!(journal.task_observations.iter().any(|observation| {
        observation.get("stage").and_then(Value::as_str) == Some("pre_compact")
    }));
    let journal_plan_steps = journal
        .rounds
        .iter()
        .flat_map(|round| {
            round
                .plan_result
                .iter()
                .flat_map(|plan| plan.steps.iter())
                .map(|step| format!("{}:{}", step.action_type, step.skill))
        })
        .collect::<Vec<_>>();
    eprintln!(
        "LLM actual call sequence={:?}",
        state.task_llm_call_sequence(&task_id)
    );
    eprintln!("journal plan steps={journal_plan_steps:?}");
    assert!(
        journal_plan_steps
            .iter()
            .any(|step| step == "call_capability:mcp.fixture.lookup"),
        "journal_plan_steps={journal_plan_steps:?}"
    );
    assert!(journal.step_results.iter().any(|step| {
        step.skill == "mcp.fixture.lookup"
            && step.status == crate::executor::StepExecutionStatus::Ok
    }));
    let capability_resolution = journal
        .task_observations
        .iter()
        .find(|observation| {
            observation.get("observation_kind").and_then(Value::as_str)
                == Some("capability_resolution")
                && observation
                    .get("requested_capability")
                    .and_then(Value::as_str)
                    == Some("mcp.fixture.lookup")
        })
        .expect("verified capability resolution observation");
    assert_eq!(
        capability_resolution["requested_capability"],
        "mcp.fixture.lookup"
    );
    assert_eq!(
        capability_resolution["resolved_capability"],
        "mcp.fixture.lookup"
    );
    assert_eq!(
        capability_resolution["resolved_tool_or_skill"],
        "tool:mcp.fixture.lookup"
    );
    assert_eq!(capability_resolution["resolution_stage"], "verify");
    assert!(journal.task_observations.iter().any(|observation| {
        observation.get("observation_kind").and_then(Value::as_str)
            == Some("capability_scope_update")
            && observation.get("source").and_then(Value::as_str) == Some("mcp_catalog_search")
            && observation
                .get("loaded_capabilities")
                .and_then(Value::as_array)
                .is_some_and(|capabilities| {
                    capabilities
                        .iter()
                        .any(|capability| capability.as_str() == Some("mcp.fixture.lookup"))
                })
    }));
    let mcp_observation = journal
        .task_observations
        .iter()
        .find(|observation| {
            observation.get("owner_layer").and_then(Value::as_str) == Some("mcp_runtime")
                && observation.get("capability").and_then(Value::as_str)
                    == Some("mcp.fixture.lookup")
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
    assert!(journal.event_stream_snapshot().iter().any(|event| {
        event.get("event_type").and_then(Value::as_str) == Some("tool_finished")
            && event
                .pointer("/payload/requested_capability")
                .and_then(Value::as_str)
                == Some("mcp.fixture.lookup")
            && event
                .pointer("/payload/resolved_capability")
                .and_then(Value::as_str)
                == Some("mcp.fixture.lookup")
    }));
    let replay = crate::task_event_transport::replay_events_after(&state, &task_id, 0)
        .expect("persisted agent-loop event replay");
    let prompt_budget = replay
        .events
        .iter()
        .find(|event| event["event_type"] == "prompt_section_budget")
        .expect("prompt section budget event");
    assert!(prompt_budget
        .pointer("/payload/sections")
        .and_then(Value::as_array)
        .is_some_and(|sections| !sections.is_empty()));
    let tool_surface = replay
        .events
        .iter()
        .find(|event| event["event_type"] == "model_tool_surface_budget")
        .expect("model tool surface budget event");
    assert!(tool_surface
        .pointer("/payload/tool_count")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
    assert!(tool_surface
        .pointer("/payload/serialized_token_estimate")
        .and_then(Value::as_u64)
        .is_some_and(|count| count > 0));
    let budget_decision = replay
        .events
        .iter()
        .find(|event| {
            event["event_type"] == "budget_decision"
                && event["payload"]["planned_action_count"] == 2
        })
        .expect("loop budget decision event");
    assert_eq!(budget_decision["payload"]["planned_action_count"], 2);
    assert_eq!(budget_decision["payload"]["executed_action_count"], 1);
    assert!(budget_decision["payload"]["hard_model_turns"]
        .as_u64()
        .is_some_and(|count| count > 0));
    let replay_json = serde_json::to_string(&replay.events).expect("serialize replay evidence");
    assert!(!replay_json.contains("mcp-agent-loop-user"));
    assert!(!replay_json.contains(user_request));
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
    assert_eq!(audit_count, 2);

    runtime.stop().await;
}
