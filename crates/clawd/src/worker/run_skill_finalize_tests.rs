use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

use claw_core::skill_registry::SkillsRegistry;

fn state_with_registry(toml: &str, skills: &[&str]) -> crate::AppState {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-run-skill-finalize-registry-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&root).expect("create temp registry dir");
    let path = root.join("skills_registry.toml");
    std::fs::write(&path, toml).expect("write registry");
    let registry = Arc::new(SkillsRegistry::load_from_path(&path).expect("load registry"));
    let _ = std::fs::remove_dir_all(root);
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(crate::SkillViewsSnapshot {
        registry: Some(registry),
        skills_list: Arc::new(skills.iter().map(|skill| (*skill).to_string()).collect()),
    })));
    state
}

fn demo_task_contract() -> Value {
    json!({
        "schema_version": 1,
        "source": "run_skill",
        "skill_name": "demo",
        "canonical_skill_name": "demo",
        "action": "preview",
        "effect": "observe",
        "risk_level": "low",
        "required_args": ["path"],
        "optional_args": [],
        "expected_evidence": ["text"],
        "delivery_shape": "text",
        "capability_ref": "demo.preview",
        "planner_kind": "skill",
        "idempotent": true,
        "dedup_scope": "args",
        "args_shape": {"action": "preview", "path": "/tmp/input"},
    })
}

fn claimed_run_skill_task(task_id: &str) -> crate::ClaimedTask {
    crate::ClaimedTask {
        task_id: task_id.to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("anon:42:7".to_string()),
        channel: "ui".to_string(),
        external_user_id: None,
        external_chat_id: None,
        kind: "run_skill".to_string(),
        payload_json: json!({
            "skill_name": "run_cmd",
            "args": {
                "command": "sleep 1; echo RUSTCLAW_ASYNC_TEST",
                "async_start": true,
            }
        })
        .to_string(),
    }
}

fn insert_running_task(state: &crate::AppState, task: &crate::ClaimedTask) {
    let db = state.core.db.get().expect("get db");
    let now = crate::now_ts();
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, kind, payload_json,
            status, result_json, error_text, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', NULL, NULL, ?8, ?8)",
        rusqlite::params![
            task.task_id,
            task.user_id,
            task.chat_id,
            task.user_key,
            task.channel,
            task.kind,
            task.payload_json,
            now,
        ],
    )
    .expect("insert running task");
}

fn task_status_and_result(state: &crate::AppState, task_id: &str) -> (String, serde_json::Value) {
    let db = state.core.db.get().expect("get db");
    let (status, raw_result): (String, String) = db
        .query_row(
            "SELECT status, result_json FROM tasks WHERE task_id = ?1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select task result");
    (
        status,
        serde_json::from_str(&raw_result).expect("parse result_json"),
    )
}

#[test]
fn direct_run_skill_observation_records_redacted_extra_evidence() {
    let token = "sk-test_abcdefghijklmnopqrstuvwxyz1234567890";
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-1", "run_skill", "run_skill:demo");
    let task_contract = demo_task_contract();
    let machine_payload = super::run_skill_success_machine_payload();

    super::record_run_skill_task_observation(
        &mut journal,
        "demo",
        "ok",
        &task_contract,
        &machine_payload,
        Some("done"),
        None,
        Some(&json!({"ok": true})),
        Some(&json!({
            "api_token": token,
            "result": {
                "path": "/tmp/output.txt",
                "exists": true
            }
        })),
        Some(true),
        Some(&json!({
            "schema_version": 1,
            "source": "skills_registry",
            "skill": "demo",
            "eligible": false,
            "admission_version": "external-v1"
        })),
    );

    let trace = journal.to_trace_json();
    let trace_text = trace.to_string();
    assert!(!trace_text.contains(token));
    assert_eq!(
        trace
            .get("task_observations")
            .and_then(Value::as_array)
            .map(Vec::len),
        Some(1)
    );

    let items = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("observed_evidence"))
        .and_then(|evidence| evidence.get("items"))
        .and_then(Value::as_array)
        .expect("observed evidence items");

    let token_item = items
        .iter()
        .find(|item| item.get("field").and_then(Value::as_str) == Some("extra.api_token"))
        .expect("extra api token item");
    assert_eq!(
        token_item.get("redacted").and_then(Value::as_bool),
        Some(true)
    );
    let admission = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("external_skill_admission"))
        .expect("external skill admission trace");
    assert_eq!(
        admission.get("eligible").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        trace
            .get("task_observations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|entry| entry.get("execution_surface_owner"))
            .and_then(Value::as_str),
        Some("single_step_skill_compat")
    );
    let observation = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("task observation");
    assert_eq!(
        observation.get("source").and_then(Value::as_str),
        Some("run_skill")
    );
    assert_eq!(
        observation.get("status_code").and_then(Value::as_str),
        Some("ok")
    );
    assert_eq!(
        observation.get("message_key").and_then(Value::as_str),
        Some("clawd.run_skill.ok")
    );
    assert_eq!(
        observation
            .pointer("/task_contract/capability_ref")
            .and_then(Value::as_str),
        Some("demo.preview")
    );
    assert_eq!(
        observation
            .pointer("/task_contract/effect")
            .and_then(Value::as_str),
        Some("observe")
    );
}

#[tokio::test]
async fn direct_run_skill_async_start_publishes_waiting_checkpoint() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = claimed_run_skill_task("task-direct-async-start");
    insert_running_task(&state, &task);
    let payload: Value = serde_json::from_str(&task.payload_json).expect("payload");
    let now = crate::now_ts_u64() as i64;
    let outcome = crate::skills::SkillRunOutcome {
        text: json!({
            "job_id": "local_process:job-1",
            "message_key": "clawd.task.async_job_started",
            "status": "accepted",
        })
        .to_string(),
        notify: Some(false),
        validation: None,
        extra: Some(json!({
            "schema_version": 1,
            "source": "builtin_success_extra",
            "action": "async_start",
            "pending_async_job": {
                "job_id": "local_process:job-1",
                "status": "accepted",
                "poll_after_seconds": 5,
                "expires_at": now + 60,
                "cancel_ref": "local_process:/tmp/rustclaw-job-1",
                "message_key": "clawd.task.async_job_pending"
            }
        })),
    };

    super::finalize_run_skill_result(&state, &task, &payload, "run_cmd", Ok(outcome))
        .await
        .expect("finalize");

    let (status, result) = task_status_and_result(&state, &task.task_id);
    assert_eq!(status, "running");
    assert_eq!(result["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        result["task_lifecycle"]["source"],
        "direct_run_skill_async_start_adapter"
    );
    assert_eq!(
        result["task_checkpoint"]["resume_entrypoint"],
        serde_json::to_value(crate::task_lifecycle::ResumeEntrypoint::PollAsyncJob)
            .expect("resume entrypoint")
    );
    assert_eq!(
        result["task_checkpoint"]["pending_async_job"]["job_id"],
        "local_process:job-1"
    );
    assert_eq!(
        result["task_journal"]["summary"]["final_stop_signal"],
        "async_job_checkpoint_waiting"
    );
}

#[test]
fn direct_run_skill_failure_records_error_observation() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-2", "run_skill", "run_skill:demo");
    let task_contract = demo_task_contract();
    let machine_payload = super::run_skill_failure_machine_payload("missing required field: path");

    super::record_run_skill_task_observation(
        &mut journal,
        "demo",
        "error",
        &task_contract,
        &machine_payload,
        None,
        Some("missing required field: path"),
        None,
        None,
        None,
        None,
    );

    let trace = journal.to_trace_json();
    let observed = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(|entry| entry.get("observed_evidence"))
        .expect("observed evidence");
    assert_eq!(
        observed.get("source").and_then(Value::as_str),
        Some("step_output")
    );
    assert_eq!(
        trace
            .get("task_observations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|entry| entry.get("source"))
            .and_then(Value::as_str),
        Some("run_skill")
    );
    assert_eq!(
        trace
            .get("task_observations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|entry| entry.get("execution_surface_owner"))
            .and_then(Value::as_str),
        Some("single_step_skill_compat")
    );
    let observation = trace
        .get("task_observations")
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .expect("task observation");
    assert_eq!(
        observation.get("error_code").and_then(Value::as_str),
        Some("skill_execution_failed")
    );
    assert_eq!(
        observation.get("status_code").and_then(Value::as_str),
        Some("skill_execution_failed")
    );
    assert_eq!(
        observation.get("message_key").and_then(Value::as_str),
        Some("clawd.run_skill.execution_failed")
    );
    assert_eq!(
        observation.get("retryable").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn run_skill_contract_uses_registry_capability_metadata() {
    let token = "sk-test_abcdefghijklmnopqrstuvwxyz1234567890";
    let state = state_with_registry(
        r#"
[[skills]]
name = "demo"
enabled = true
kind = "runner"
planner_kind = "skill"
output_kind = "file"
risk_level = "medium"
side_effect = true
idempotent = false
dedup_scope = "action"
planner_capabilities = [
  { name = "demo.preview", action = "preview", effect = "observe", required = ["path"], optional = ["limit"], risk_level = "low", idempotent = true, dedup_scope = "args" },
]
"#,
        &["demo"],
    );
    let payload = json!({
        "args": {
            "action": "preview",
            "path": "/tmp/input",
            "api_token": token,
            "limit": 3
        }
    });

    let contract = super::run_skill_capability_contract(&state, &payload, "demo");

    assert_eq!(
        contract.get("source").and_then(Value::as_str),
        Some("run_skill")
    );
    assert_eq!(
        contract.get("capability_ref").and_then(Value::as_str),
        Some("demo.preview")
    );
    assert_eq!(
        contract.get("effect").and_then(Value::as_str),
        Some("observe")
    );
    assert_eq!(
        contract.get("risk_level").and_then(Value::as_str),
        Some("low")
    );
    assert_eq!(
        contract.get("delivery_shape").and_then(Value::as_str),
        Some("file")
    );
    assert_eq!(
        contract.get("idempotent").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        contract.get("dedup_scope").and_then(Value::as_str),
        Some("args")
    );
    assert_eq!(
        contract
            .get("required_args")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(Value::as_str),
        Some("path")
    );
    assert!(!contract.to_string().contains(token));
    assert_eq!(
        contract
            .pointer("/args_shape/api_token")
            .and_then(Value::as_str),
        Some("[REDACTED]")
    );
}
