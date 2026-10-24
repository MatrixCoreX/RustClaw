use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, RwLock};

use claw_core::skill_registry::SkillsRegistry;

fn state_with_registry(toml: &str, skills: &[&str]) -> crate::AppState {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-run-skill-finalize-registry-{}-{}",
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
        binding: Default::default(),
        registry: Some(registry),
        skills_list: Arc::new(skills.iter().map(|skill| (*skill).to_string()).collect()),
    })));
    state
}

fn state_with_runtime_registry() -> crate::AppState {
    let registry_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/skills_registry.toml")
        .canonicalize()
        .expect("canonicalize registry path");
    let registry = SkillsRegistry::load_from_path(&registry_path).expect("load registry");
    let enabled: HashSet<String> = registry.enabled_names().into_iter().collect();
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.core.skill_views_snapshot = Arc::new(RwLock::new(Arc::new(crate::SkillViewsSnapshot {
        binding: Default::default(),
        registry: Some(Arc::new(registry)),
        skills_list: Arc::new(enabled),
    })));
    let mut tools = claw_core::config::ToolsConfig::default();
    tools.allow = vec!["skill:run_cmd".to_string()];
    state.skill_rt.tools_policy = Arc::new(crate::ToolsPolicy::from_config(&tools).unwrap());
    state
}

fn demo_capability_contract() -> Value {
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
        claim_attempt: 1,
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
                "command": "sleep 1; echo APP_ASYNC_TEST",
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
            status, result_json, error_text, created_at, updated_at,
            lease_owner, lease_expires_at, claim_attempt, claimed_at
        )
        VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', NULL, NULL, ?8, ?8,
            ?9, 9223372036854775807, ?10, ?8
        )",
        rusqlite::params![
            task.task_id,
            task.user_id,
            task.chat_id,
            task.user_key,
            task.channel,
            task.kind,
            task.payload_json,
            now,
            state.worker.worker_id,
            task.claim_attempt,
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
    let capability_contract = demo_capability_contract();
    let machine_payload = super::run_skill_success_machine_payload();

    super::record_run_skill_task_observation(
        &mut journal,
        "demo",
        "ok",
        &capability_contract,
        &machine_payload,
        Some("done"),
        None,
        Some(&json!({"ok": true})),
        Some(&json!({
            "api_token": token,
            "result": {
                "path": "/tmp/output.txt",
                "exists": true
            },
            "execution_binding": {
                "version": "1.2.3",
                "registry_generation": 7,
                "receipt_digest": "a".repeat(64),
                "policy_digest": "b".repeat(64)
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
    let execution_binding = trace
        .pointer("/task_observations/0/execution_binding")
        .expect("execution binding trace");
    assert_eq!(execution_binding["version"], "1.2.3");
    assert_eq!(execution_binding["registry_generation"], 7);
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
            .pointer("/capability_contract/capability_ref")
            .and_then(Value::as_str),
        Some("demo.preview")
    );
    assert_eq!(
        observation
            .pointer("/capability_contract/effect")
            .and_then(Value::as_str),
        Some("observe")
    );
}

#[test]
fn kb_task_trace_keeps_machine_metrics_without_query_document_or_path_content() {
    let state = state_with_runtime_registry();
    let query = "private quarterly acquisition plan";
    let document_text = "confidential document body for the planned acquisition";
    let private_path = "/home/private/board-plan.pdf";
    let warning = "skip /home/private/broken.pdf: encrypted document";
    let payload = json!({
        "args": {
            "action": "search",
            "namespace": "private-board-documents",
            "query": query,
            "filters": {"path_prefix": private_path}
        }
    });
    let capability_contract = super::run_skill_capability_contract(&state, &payload, "kb");
    assert_eq!(capability_contract["args_shape"]["query"]["redacted"], true);
    assert_eq!(
        capability_contract["args_shape"]["filters"]["path_prefix"]["redacted"],
        true
    );
    let extra = json!({
        "retrieval_trace": {
            "query_digest": "a".repeat(64),
            "selected_hit_ids": ["evidence:sha256:1234"]
        },
        "hits": [{
            "evidence_id": "evidence:sha256:1234",
            "source_ref": "kb://private-board-documents/digest/chunk-1",
            "location": {"path": private_path, "page": 7},
            "text": document_text
        }],
        "warnings": [warning]
    });
    let projected = super::run_skill_trace_safe_json_for_skill("kb", &extra);
    assert_eq!(projected["retrieval_trace"]["query_digest"], "a".repeat(64));
    assert_eq!(
        projected["retrieval_trace"]["selected_hit_ids"][0],
        "evidence:sha256:1234"
    );
    assert_eq!(projected["hits"][0]["text"]["redacted"], true);
    assert_eq!(projected["warnings"]["redacted"], true);

    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-kb", "run_skill", "run_skill:kb");
    super::record_run_skill_task_observation(
        &mut journal,
        "kb",
        "ok",
        &capability_contract,
        &super::run_skill_success_machine_payload(),
        Some(document_text),
        None,
        None,
        Some(&extra),
        None,
        None,
    );

    let trace = journal.to_trace_json();
    let serialized = trace.to_string();
    for private in [
        query,
        document_text,
        private_path,
        warning,
        "private-board-documents",
    ] {
        assert!(!serialized.contains(private), "trace leaked {private}");
    }
    assert!(serialized.contains("\"redacted\":true"));
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
        output_contract_validation: None,
        extra: Some(json!({
            "schema_version": 1,
            "source": "builtin_success_extra",
            "action": "async_start",
            "pending_async_job": {
                "job_id": "local_process:job-1",
                "status": "accepted",
                "poll_after_seconds": 5,
                "expires_at": now + 60,
                "cancel_ref": "local_process:/tmp/agent-runtime-job-1",
                "message_key": "clawd.task.async_job_pending"
            }
        })),
    };

    let verification = super::super::run_skill_permission::verify_direct_run_skill(
        &state,
        &task,
        "run_cmd",
        payload.get("args").cloned().unwrap_or_else(|| json!({})),
    );
    super::finalize_run_skill_result(
        &state,
        &task,
        &payload,
        "run_cmd",
        &verification,
        Ok(outcome),
    )
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
        result["task_checkpoint"]["capability_results"][0]["capability"],
        "run_cmd"
    );
    assert_eq!(
        result["task_checkpoint"]["capability_results"][0]["status"],
        "waiting"
    );
    assert_eq!(
        result["task_lifecycle"]["async_timeout_policy"]["adapter_kind"],
        "local_process_poll"
    );
    assert_eq!(
        result["task_lifecycle"]["async_timeout_policy"]["policy_source"],
        "pending_async_job_contract"
    );
    assert_eq!(
        result["task_journal"]["summary"]["final_stop_signal"],
        "async_job_checkpoint_waiting"
    );
}

#[tokio::test]
async fn direct_run_skill_resource_pressure_requeues_without_terminal_failure() {
    let state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let task = claimed_run_skill_task("task-direct-resource-wait");
    insert_running_task(&state, &task);
    let payload: Value = serde_json::from_str(&task.payload_json).expect("payload");
    let verification = super::super::run_skill_permission::verify_direct_run_skill(
        &state,
        &task,
        "run_cmd",
        payload.get("args").cloned().unwrap_or_else(|| json!({})),
    );
    let error = crate::skills::structured_skill_error_from_parts(
        "run_cmd",
        "resource_admission_unavailable",
        "resource_admission_unavailable",
        Some(std::env::consts::OS),
        Some(json!({
            "message_key": "clawd.execution.resource_admission_unavailable",
            "retryable": true,
            "failure_phase": "pre_dispatch",
            "side_effect_applied": false,
            "wait_reason": "resource_pressure",
            "resource_grant": {"admitted": false},
        })),
    );

    super::finalize_run_skill_result(
        &state,
        &task,
        &payload,
        "run_cmd",
        &verification,
        Err(error),
    )
    .await
    .expect("requeue resource wait");

    let (status, result) = task_status_and_result(&state, &task.task_id);
    assert_eq!(status, "queued");
    assert_eq!(result["status_code"], "resource_admission_wait");
    assert_eq!(result["task_lifecycle"]["state"], "waiting");
    assert_eq!(
        result["task_lifecycle"]["source"],
        "direct_run_skill_resource_admission"
    );
    assert_eq!(
        result["task_lifecycle"]["waiting_reason_code"],
        "resource_pressure"
    );
    assert_eq!(result["task_lifecycle"]["retry_after_seconds"], 15);
    assert_eq!(result["task_lifecycle"]["can_cancel"], true);
    assert!(crate::repo::claim_next_task(&state)
        .expect("poll before due")
        .is_none());
}

#[tokio::test]
async fn direct_run_skill_confirmation_persists_and_consumes_exact_grant() {
    let state = state_with_runtime_registry();
    let task_id = "task-direct-skill-confirmation";
    let task = crate::ClaimedTask {
        claim_attempt: 1,
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
            "args": {"command": "rm -rf target/direct-skill-confirmation-fixture"}
        })
        .to_string(),
    };
    insert_running_task(&state, &task);
    let payload: Value = serde_json::from_str(&task.payload_json).expect("payload");
    let args = payload.get("args").cloned().unwrap_or_else(|| json!({}));
    let verification = super::super::run_skill_permission::verify_direct_run_skill(
        &state,
        &task,
        "run_cmd",
        args.clone(),
    );
    assert!(verification.needs_confirmation());

    super::finalize_run_skill_confirmation_required(
        &state,
        &task,
        &payload,
        "run_cmd",
        &verification,
    )
    .await
    .expect("finalize confirmation");

    let (status, result) = task_status_and_result(&state, task_id);
    assert_eq!(status, "running");
    assert_eq!(result["task_lifecycle"]["state"], "needs_user");
    assert_eq!(
        result["task_lifecycle"]["resume_reason"],
        "confirmation_required"
    );
    assert_eq!(
        result["task_journal"]["summary"]["task_checkpoint"]["resume_entrypoint"],
        "await_user_input"
    );
    assert_eq!(
        result["resume_context"]["approval_request"]["status"],
        "pending"
    );
    assert_eq!(
        result["task_journal"]["summary"]["verify_result"]["permission_decision"]["decision"],
        crate::policy_decision::PolicyDecision::RequireConfirmation.as_token()
    );
    let request_id = result["resume_context"]["approval_request"]["request_id"]
        .as_str()
        .expect("request id");
    crate::repo::decide_task_approval_request_for_actor(
        &state,
        task_id,
        request_id,
        crate::approval_grant::ApprovalDecision::ApproveOnce,
        None,
    )
    .expect("approve request")
    .expect("approval update");
    let (requeued_status, _) = task_status_and_result(&state, task_id);
    assert_eq!(requeued_status, "queued");
    state
        .core
        .db
        .get()
        .expect("db")
        .execute(
            "UPDATE tasks SET status = 'running' WHERE task_id = ?1 AND status = 'queued'",
            rusqlite::params![task_id],
        )
        .expect("simulate resumed worker claim");

    let resumed =
        super::super::run_skill_permission::verify_direct_run_skill(&state, &task, "run_cmd", args);
    assert!(resumed.allowed(), "{:?}", resumed.verify.issues);
    assert_eq!(
        resumed.verify.permission_decision["approval_grant"]["status"],
        crate::repo::TaskApprovalConsumeOutcome::Consumed.as_token()
    );

    let repeated = super::super::run_skill_permission::verify_direct_run_skill(
        &state,
        &task,
        "run_cmd",
        payload.get("args").cloned().unwrap_or_else(|| json!({})),
    );
    assert!(repeated.needs_confirmation());
}

#[test]
fn direct_run_skill_failure_records_error_observation() {
    let mut journal =
        crate::task_journal::TaskJournal::for_task("task-2", "run_skill", "run_skill:demo");
    let capability_contract = demo_capability_contract();
    let machine_payload = super::run_skill_failure_machine_payload("missing required field: path");

    super::record_run_skill_task_observation(
        &mut journal,
        "demo",
        "error",
        &capability_contract,
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
fn resource_admission_failure_remains_retryable() {
    let error = crate::skills::structured_skill_error_from_parts(
        "media_discovery",
        "resource_admission_unavailable",
        "resource_admission_unavailable",
        Some(std::env::consts::OS),
        Some(json!({
            "message_key": "clawd.execution.resource_admission_unavailable",
            "retryable": true,
            "failure_phase": "pre_dispatch",
            "side_effect_applied": false,
            "wait_reason": "resource_pressure",
        })),
    );
    let machine_payload = super::run_skill_failure_machine_payload(&error);
    assert_eq!(machine_payload["retryable"], true);
    assert_eq!(
        machine_payload["error_code"],
        "resource_admission_unavailable"
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
input_schema = { type = "object", properties = { path = { type = "string" }, limit = { type = "integer" } } }
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

#[tokio::test]
async fn host_enrolled_success_avoids_shared_memory_while_forged_ordinary_fields_keep_it() {
    let root=std::env::temp_dir().join(format!("agent-outcome-retention-{}",uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let mut state=state_with_runtime_registry(); state.skill_rt.workspace_root=root.clone();
    state.seed_test_auth_identity("retention-fixture-admin", "admin");
    state.policy.memory.write_filter_enabled = false;
    crate::memory::scope::ensure_memory_scope_schema(&*state.core.db.get().unwrap()).unwrap();
    crate::memory::service::insert_memory_with_kind(&state,42,7,Some("retention-fixture-admin"),"ui",None,
        crate::memory::MEMORY_ROLE_ASSISTANT,"fixture-memory-preflight",256,crate::memory::MemoryWriteKind::AssistantOutcome).unwrap();
    let enrollment=crate::task_result_retention::DeliveryCleanupEnrollment {
        schema_version:1,work_id:format!("nni-work-{}",uuid::Uuid::new_v4().simple()),attempt:1,
        capability:"fixture.download".to_string(),origin:"https://media.example.org".to_string(),
        job_id:uuid::Uuid::new_v4().to_string(),payload_sha256:"a".repeat(64),
    };
    for enrolled in [false,true] {
        let mut task=claimed_run_skill_task(&uuid::Uuid::new_v4().to_string());
        task.user_key = Some("retention-fixture-admin".to_string());
        let payload=json!({"skill_name":"run_cmd","args":{"command":"fixture"},
            "_task_source":"distributed_work","_requested_capability":enrollment.capability,
            "_delivery_cleanup":enrollment});
        task.payload_json=payload.to_string(); insert_running_task(&state,&task);
        let text=if enrolled {"enrolled-outcome-body"} else {"ordinary-outcome-body"};
        if enrolled {
            crate::task_result_retention::register_enrollment(&root,&task.task_id,&enrollment).unwrap();
            state.core.db.get().unwrap().execute("UPDATE tasks SET idempotency_key=?2 WHERE task_id=?1",rusqlite::params![task.task_id,enrollment.idempotency_key()]).unwrap();
        }
        let verification=super::super::run_skill_permission::verify_direct_run_skill(&state,&task,"run_cmd",payload["args"].clone());
        super::finalize_run_skill_result(&state,&task,&payload,"run_cmd",&verification,Ok(crate::skills::SkillRunOutcome {
            text:text.to_string(),notify:Some(false),validation:None,output_contract_validation:None,extra:None,
        })).await.unwrap();
        let (status,result)=task_status_and_result(&state,&task.task_id); assert_eq!(status,"succeeded"); assert_eq!(result["text"],text);
        let count:usize=state.core.db.get().unwrap().query_row("SELECT COUNT(*) FROM memories WHERE content=?1",[text],|r|r.get(0)).unwrap();
        assert_eq!(count,usize::from(!enrolled));
    }
    std::fs::remove_dir_all(root).unwrap();
}
