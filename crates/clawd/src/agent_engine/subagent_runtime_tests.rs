use super::*;
use crate::agent_engine::LoopState;
use std::path::{Path, PathBuf};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "agent-runtime-subagent-runtime-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn insert_running_parent_task(state: &crate::AppState, task: &crate::ClaimedTask) {
    let db = state.core.db.get().expect("get db");
    db.execute(
        "INSERT INTO tasks (
            task_id, user_id, chat_id, user_key, channel, external_user_id,
            external_chat_id, kind, payload_json, status, result_json,
            error_text, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'ask', ?8, 'running', '{}', NULL, '1', '1')",
        rusqlite::params![
            task.task_id,
            task.user_id,
            task.chat_id,
            task.user_key,
            task.channel,
            task.external_user_id,
            task.external_chat_id,
            task.payload_json
        ],
    )
    .expect("insert running parent task");
}

fn child_task_row(state: &crate::AppState, task_id: &str) -> (String, serde_json::Value) {
    let db = state.core.db.get().expect("get db");
    let (status, payload_json): (String, String) = db
        .query_row(
            "SELECT status, payload_json FROM tasks WHERE task_id = ?1 LIMIT 1",
            rusqlite::params![task_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("select child task row");
    (
        status,
        serde_json::from_str(&payload_json).expect("parse child payload"),
    )
}

fn persistent_test_state() -> crate::AppState {
    let mut state = crate::AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state.reload_ctx.config_path_for_reload = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../configs/config.toml")
        .display()
        .to_string();
    state
}

fn install_test_task_budget(loop_state: &mut LoopState) {
    loop_state.task_budget_slice = Some(crate::task_budget_contract::TaskBudgetSlice::new(
        crate::task_budget_contract::TaskBudgetProfile::MultiStepWorkspace,
        3_600_000,
        crate::task_budget_contract::BudgetHardCeilings::default(),
    ));
}

#[test]
fn subagent_action_records_safe_machine_observation() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;

    let stop_signal = record_subagent_action(
        &mut loop_state,
        3,
        2,
        "review",
        "Review the selected files for risk.",
        &[
            "step_1:evidence".to_string(),
            "unsafe natural ref with spaces".to_string(),
        ],
        SubagentActionOptions::default(),
    );

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["owner_layer"], "subagent_runtime");
    assert_eq!(observation["status"], "accepted");
    assert_eq!(observation["role"], "review");
    assert_eq!(observation["role_metadata"]["role_family"], "reviewer");
    assert_eq!(
        observation["role_metadata"]["tool_permission_profile"],
        "read_only"
    );
    assert_eq!(
        observation["role_metadata"]["result_contract_required"],
        true
    );
    assert_eq!(
        observation["timeout_policy"]["policy"],
        "no_operation_deadline"
    );
    assert_eq!(
        observation["timeout_policy"]["runtime_deadline_source"],
        "none"
    );
    assert_eq!(observation["cancellation_policy"]["cancellable"], true);
    assert_eq!(observation["execution_mode"], "inline_readonly_child_run");
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
    assert_eq!(observation["objective_present"], true);
    assert_eq!(observation["context_refs"][0]["ref"], "step_1:evidence");
    assert_eq!(observation["context_refs"][1]["ref"], "");
}

#[test]
fn subagent_action_rejects_unknown_role_as_machine_state() {
    let mut loop_state = LoopState::new();

    let stop_signal = record_subagent_action(
        &mut loop_state,
        1,
        1,
        "unsupported_writer_probe",
        "",
        &[],
        SubagentActionOptions::default(),
    );

    assert_eq!(stop_signal, Some(SUBAGENT_STOP_SIGNAL_INVALID_ROLE));
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["owner_layer"], "subagent_runtime");
    assert_eq!(observation["status"], "rejected");
    assert_eq!(observation["error_code"], "subagent_role_not_allowed");
    assert_eq!(observation["allowed_roles"][0], "observe");
    assert_eq!(observation["allowed_roles"][1], "explorer");
    assert_eq!(observation["allowed_roles"][7], "verifier");
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
}

#[test]
fn subagent_action_from_args_records_child_summary_and_machine_contract() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 4;
    let args = serde_json::json!({
        "role": "test",
        "objective": "Run the scoped verification.",
        "parent_task_id": "task_123",
        "allowed_capabilities": ["filesystem.read", "bad token"],
        "budget": {
            "max_rounds": 1,
            "max_tool_calls": 2,
            "max_context_chars": 4096,
            "runtime_deadline_ms": 2500
        },
        "context_slice": {
            "refs": ["step_1:evidence:1", "unsafe ref"],
            "max_context_chars": 4096
        },
        "result_contract": {
            "status": "enum",
            "evidence_refs": "array"
        }
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 7, 3, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["child_run_id"], "subagent:4:3:test");
    assert_eq!(
        observation["allowed_capabilities"][0]["token"],
        "filesystem.read"
    );
    assert_eq!(observation["allowed_capabilities"][1]["token"], "");
    assert_eq!(observation["budget"]["max_tool_calls"], 2);
    assert_eq!(observation["timeout_policy"]["runtime_deadline_ms"], 2500);
    assert_eq!(
        observation["timeout_policy"]["terminal_status_on_deadline"],
        "timed_out"
    );
    assert_eq!(
        observation["cancellation_policy"]["cancel_scope"],
        "child_run"
    );
    assert_eq!(observation["parent_task_ref"], "task_123");
    assert_eq!(
        observation["context_slice"]["refs"][0]["ref"],
        "step_1:evidence:1"
    );
    assert_eq!(observation["result_contract"]["kind"], "object");
    assert_eq!(
        observation["child_run_summary"]["trace_merge_status"],
        "not_started"
    );
    assert_eq!(observation["child_request"]["state"], "needs_more_evidence");
    assert_eq!(
        observation["child_request"]["role_metadata"]["role_family"],
        "verifier"
    );
    assert_eq!(
        observation["child_request"]["timeout_policy"]["runtime_deadline_ms"],
        2500
    );
    assert_eq!(
        observation["child_request"]["execution_mode"],
        "inline_readonly_child_run"
    );
    assert_eq!(
        observation["child_request"]["request_ref"],
        "subagent:4:3:test"
    );
    assert_eq!(observation["scheduler"]["status"], "waiting_for_evidence");
    assert_eq!(
        observation["scheduler"]["reason_code"],
        "readonly_subagent_context_evidence_required"
    );
    assert_eq!(observation["scheduler"]["lease_required"], false);
    assert_eq!(observation["scheduler"]["checkpoint_required"], false);
    assert_eq!(
        observation["merge_contract"]["strategy"],
        "append_child_trace_summary"
    );
    assert_eq!(
        observation["merge_contract"]["child_trace_merge_status"],
        "not_started"
    );
    assert_eq!(observation["child_result"]["status"], "needs_more_evidence");
    assert_eq!(observation["child_result"]["role_family"], "verifier");
    assert_eq!(
        observation["child_result"]["result_contract_required"],
        true
    );
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_inline_readonly_needs_more_evidence"
    );
    assert_eq!(observation["write_enabled"], false);
}

#[test]
fn subagent_action_projects_workspace_context_evidence() {
    let temp = TempDirGuard::new("context-evidence");
    std::fs::create_dir_all(temp.path().join("plan")).expect("create plan dir");
    let long_agents = format!(
        "runtime boundary\napi_key = should_not_leak\n{}\nlate runtime boundary\nsecret = should_not_leak_late\n",
        "filler line\n".repeat(300)
    );
    std::fs::write(temp.path().join("AGENTS.md"), long_agents).expect("write agents");
    std::fs::write(
        temp.path().join("plan/current.md"),
        "plan boundary\nsubagent review stays read only\n",
    )
    .expect("write plan");

    let mut loop_state = LoopState::new();
    loop_state.round_no = 8;
    let config = SubagentRuntimeConfig {
        context_evidence_root: Some(temp.path().to_path_buf()),
        ..SubagentRuntimeConfig::default()
    };
    let args = serde_json::json!({
        "role": "review",
        "objective": "runtime_boundary_alignment_audit",
        "context_refs": ["AGENTS.md", "plan/current.md"],
        "context_slice": {
            "max_context_chars": 1024
        },
        "result_contract": {
            "output_format": "machine_json",
            "content_excerpt": "string"
        }
    });

    let stop_signal =
        record_subagent_action_from_args_with_config(&mut loop_state, 10, 1, &args, &config);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["output_format"], "machine_json");
    assert_eq!(observation["action"], "read_text_range");
    assert_eq!(observation["path"], "AGENTS.md");
    assert_eq!(observation["paths"].as_array().unwrap().len(), 2);
    assert_eq!(observation["context_evidence"]["present"], true);
    assert_eq!(observation["context_evidence"]["available_count"], 2);
    assert_eq!(
        observation["context_evidence"]["items"][0]["path"],
        "AGENTS.md"
    );
    assert_eq!(
        observation["context_evidence"]["items"][1]["path"],
        "plan/current.md"
    );
    let excerpt = observation["content_excerpt"].as_str().unwrap();
    assert!(excerpt.contains("runtime boundary"));
    assert!(excerpt.contains("late runtime boundary"));
    assert!(excerpt.contains("plan boundary"));
    assert!(excerpt.contains("[REDACTED_SENSITIVE_LINE]"));
    assert!(!excerpt.contains("should_not_leak"));
    assert!(!excerpt.contains("should_not_leak_late"));
    assert_eq!(
        observation["context_evidence"]["items"][0]["excerpt_strategy"],
        "head_tail"
    );
    assert_eq!(observation["child_result"]["content_excerpt_present"], true);
    assert_eq!(observation["child_request"]["state"], "ready");
    assert_eq!(observation["scheduler"]["status"], "ready_for_model");
    assert_eq!(observation["child_result"]["result_status"], "ready");
}

#[test]
fn persistent_subagent_action_enqueues_child_task_and_sets_waiting_checkpoint() {
    let state = persistent_test_state();
    let admin_key = "rk-persistent-subagent-admin";
    let db = state.core.db.get().expect("get db");
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS auth_keys (
            user_key TEXT PRIMARY KEY,
            role TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL,
            last_used_at TEXT
        );",
    )
    .expect("auth key table");
    db.execute(
        "INSERT INTO auth_keys (user_key, role, enabled, created_at)
         VALUES (?1, 'admin', 1, 'now')",
        rusqlite::params![admin_key],
    )
    .expect("insert admin key");
    drop(db);
    let mut payload = serde_json::json!({
        "text": "parent task",
        "subagent_execution": {
            "runtime_deadline_ms": 2500
        }
    });
    crate::task_execution_policy::stamp_authenticated_submission_policy(
        &mut payload,
        Some(&claw_core::types::AuthIdentity {
            user_key: admin_key.to_string(),
            role: "admin".to_string(),
            user_id: 42,
            chat_id: 7,
        }),
        Some("clawcli"),
        Some("yolo"),
    )
    .expect("stamp parent yolo policy");
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-persistent-subagent-parent".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some(admin_key.to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: payload.to_string(),
    };
    insert_running_parent_task(&state, &task);
    let mut loop_state = LoopState::new();
    loop_state.round_no = 1;
    install_test_task_budget(&mut loop_state);
    let args = serde_json::json!({
        "action": "persistent_child_task",
        "role": "review",
        "objective": "machine_child_objective:persistent-review",
        "context_refs": ["AGENTS.md"],
        "allowed_capabilities": ["filesystem.read_text_range"],
        "required": true,
        "budget": {
            "max_rounds": 3,
            "max_tool_calls": 12,
            "timeout_ms": 180000
        },
        "runtime_deadline": {
            "duration_ms": 120000
        },
        "result_contract": {
            "output_format": "machine_json",
            "required_keys": ["findings", "evidence_refs"]
        }
    });

    let schedule = record_persistent_child_task_from_args(
        &state,
        &task,
        &mut loop_state,
        4,
        1,
        &args,
        &SubagentRuntimeConfig::default(),
    );
    assert!(
        schedule.is_ok(),
        "schedule persistent child task: {:?}",
        loop_state.task_observations
    );
    let stop_signal = schedule.expect("checked persistent child schedule");

    assert_eq!(
        stop_signal,
        Some(subagent_runtime_persistent::SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING)
    );
    let observation = loop_state
        .task_observations
        .last()
        .expect("persistent observation");
    assert_eq!(observation["owner_layer"], "subagent_runtime");
    assert_eq!(observation["status"], "waiting");
    assert_eq!(observation["execution_mode"], "persistent_child_task");
    let child_task_id = observation["child_task_ids"][0]
        .as_str()
        .expect("child task id");
    let (child_status, child_payload) = child_task_row(&state, child_task_id);
    assert_eq!(child_status, "queued");
    assert_eq!(child_payload["task_role"], "subagent_child");
    assert_eq!(child_payload["parent_task_id"], task.task_id);
    assert_eq!(
        child_payload[crate::task_execution_policy::POLICY_PAYLOAD_FIELD]["mode"],
        "yolo"
    );
    assert_eq!(
        child_payload[crate::task_execution_policy::POLICY_PAYLOAD_FIELD]["derivation"],
        "authenticated_parent_task"
    );
    assert_eq!(
        child_payload["child_task_contract"]["permission_profile"],
        "read_only"
    );
    assert_eq!(
        child_payload["child_task_contract"]["budget"]["runtime_deadline_ms"],
        2500
    );
    assert_eq!(
        loop_state
            .task_lifecycle
            .as_ref()
            .and_then(|value| value.get("state"))
            .and_then(serde_json::Value::as_str),
        Some("waiting")
    );
    assert_eq!(
        loop_state
            .task_lifecycle
            .as_ref()
            .and_then(|value| value.get("source"))
            .and_then(serde_json::Value::as_str),
        Some("subagent_child_task_enqueue")
    );
    assert_eq!(
        loop_state
            .task_checkpoint
            .as_ref()
            .and_then(|value| value.get("resume_entrypoint"))
            .and_then(serde_json::Value::as_str),
        Some("next_planner_round")
    );
}

#[test]
fn persistent_writer_defaults_to_parent_reviewed_local_worktree() {
    let state = persistent_test_state();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-persistent-writer-parent".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text": "parent task"}).to_string(),
    };
    insert_running_parent_task(&state, &task);
    let mut loop_state = LoopState::new();
    install_test_task_budget(&mut loop_state);
    let args = serde_json::json!({
        "action": "persistent_child_task",
        "role": "writer",
        "objective": "machine_child_objective:isolated-write",
        "context_refs": ["README.md"],
        "allowed_capabilities": ["filesystem.write_text"],
        "required": true,
        "runtime_deadline": {
            "duration_ms": 120000
        },
        "result_contract": {
            "output_format": "machine_json",
            "required_keys": ["artifact_refs", "evidence_refs"]
        }
    });

    let stop_signal = record_persistent_child_task_from_args(
        &state,
        &task,
        &mut loop_state,
        1,
        1,
        &args,
        &SubagentRuntimeConfig::default(),
    )
    .expect("schedule persistent writer");

    assert_eq!(
        stop_signal,
        Some(subagent_runtime_persistent::SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING)
    );
    let observation = loop_state
        .task_observations
        .last()
        .expect("writer observation");
    assert_eq!(observation["write_enabled"], true);
    assert_eq!(observation["write_scope"], "persistent_local_worktree");
    let child_task_id = observation["child_task_ids"][0]
        .as_str()
        .expect("child task id");
    let (_, payload) = child_task_row(&state, child_task_id);
    assert_eq!(payload["child_task_contract"]["role"], "writer");
    assert_eq!(
        payload["child_task_contract"]["permission_profile"],
        "local_worktree"
    );
    assert_eq!(
        payload["child_task_contract"]["scope"]["allowed_capabilities"][0],
        "filesystem.write_text"
    );
    assert_eq!(
        payload["child_task_contract"]["budget"]["runtime_deadline_ms"],
        Value::Null
    );
}

#[test]
fn ordinary_batch_materializes_all_children_and_queues_capacity_overflow() {
    let state = persistent_test_state();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-durable-readonly-batch-parent".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text": "parent task"}).to_string(),
    };
    insert_running_parent_task(&state, &task);
    let mut loop_state = LoopState::new();
    install_test_task_budget(&mut loop_state);
    let children = (0..16)
        .map(|index| {
            serde_json::json!({
                "node_id": format!("review_{index}"),
                "role": "review",
                "objective": format!("machine_child_objective:review-{index}"),
                "context_refs": ["README.md"],
                "allowed_capabilities": ["filesystem.read_text_range"]
            })
        })
        .collect::<Vec<_>>();
    let args = serde_json::json!({
        "action": "bounded_parallel_readonly",
        "max_parallel": 4,
        "children": children,
    });

    let stop_signal = record_durable_readonly_child_task_from_args(
        &state,
        &task,
        &mut loop_state,
        1,
        1,
        &args,
        &SubagentRuntimeConfig::default(),
    )
    .expect("schedule durable readonly batch");

    assert_eq!(
        stop_signal,
        Some(subagent_runtime_persistent::SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING)
    );
    let observation = loop_state.task_observations.last().expect("observation");
    assert_eq!(observation["child_task_ids"].as_array().unwrap().len(), 16);
    let db = state.core.db.get().expect("get db");
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, &task.task_id)
        .expect("read graph")
        .expect("graph");
    let nodes = graph["nodes"].as_array().expect("nodes");
    assert_eq!(nodes.len(), 16);
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node["readiness"] == "ready")
            .count(),
        4
    );
    assert_eq!(
        nodes
            .iter()
            .filter(|node| node["readiness"] == "blocked_capacity")
            .count(),
        12
    );
    drop(db);
    for child_task_id in observation["child_task_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
    {
        let (_, payload) = child_task_row(&state, child_task_id);
        assert_eq!(
            payload["child_task_contract"]["permission_profile"],
            "read_only"
        );
        assert_eq!(payload["child_task_contract"]["schema_version"], 2);
    }

    let original_child_task_ids = observation["child_task_ids"].clone();
    let replay_signal = record_durable_readonly_child_task_from_args(
        &state,
        &task,
        &mut loop_state,
        2,
        1,
        &serde_json::json!({
            "action": "inline_readonly",
            "role": "review",
            "objective": "machine_child_objective:must-not-respawn",
            "context_refs": ["AGENTS.md"],
            "allowed_capabilities": ["filesystem.read_text_range"]
        }),
        &SubagentRuntimeConfig::default(),
    )
    .expect("reuse pending checkpoint child graph");
    assert_eq!(
        replay_signal,
        Some(subagent_runtime_persistent::SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING)
    );
    let replay_observation = loop_state
        .task_observations
        .last()
        .expect("replay observation");
    assert_eq!(
        replay_observation["child_task_enqueue"]["admission_reused"],
        true
    );
    let mut replay_child_task_ids = replay_observation["child_task_ids"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let mut expected_child_task_ids = original_child_task_ids
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    replay_child_task_ids.sort_unstable();
    expected_child_task_ids.sort_unstable();
    assert_eq!(replay_child_task_ids, expected_child_task_ids);
    let db = state.core.db.get().expect("get db after replay");
    let graph_after_replay = crate::repo::child_task_graph::graph_snapshot(&db, &task.task_id)
        .expect("read graph after replay")
        .expect("graph after replay");
    assert_eq!(graph_after_replay["nodes"].as_array().unwrap().len(), 16);
}

#[test]
fn session_capacity_is_shared_across_parents_and_released_on_terminal() {
    let state = persistent_test_state();
    let mut config = SubagentRuntimeConfig::default();
    config.max_concurrent_threads_per_session = 2;
    config.max_parallel_readonly = 2;
    let make_task = |task_id: &str| crate::ClaimedTask {
        claim_attempt: 0,
        task_id: task_id.to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: serde_json::json!({
            "text": "parent task",
            "thread_id": "shared-thread"
        })
        .to_string(),
    };
    let first = make_task("task-session-z-parent");
    let second = make_task("task-session-a-parent");
    insert_running_parent_task(&state, &first);
    insert_running_parent_task(&state, &second);
    let batch_args = |prefix: &str| {
        let children = [0, 1].map(|index| {
            serde_json::json!({
                "node_id": format!("{prefix}_{index}"),
                "role": "review",
                "objective": format!("machine_child_objective:{prefix}-{index}"),
                "context_refs": ["README.md"],
                "allowed_capabilities": ["filesystem.read_text_range"]
            })
        });
        serde_json::json!({
            "action": "bounded_parallel_readonly",
            "max_parallel": 2,
            "children": children
        })
    };
    for (task, prefix) in [(&first, "first"), (&second, "second")] {
        let mut loop_state = LoopState::new();
        install_test_task_budget(&mut loop_state);
        record_durable_readonly_child_task_from_args(
            &state,
            task,
            &mut loop_state,
            1,
            1,
            &batch_args(prefix),
            &config,
        )
        .expect("schedule shared-session batch");
    }

    let db = state.core.db.get().expect("get db");
    let first_graph = crate::repo::child_task_graph::graph_snapshot(&db, &first.task_id)
        .expect("first graph")
        .expect("first graph exists");
    let second_graph = crate::repo::child_task_graph::graph_snapshot(&db, &second.task_id)
        .expect("second graph")
        .expect("second graph exists");
    assert_eq!(first_graph["session_open_count"], 2);
    assert_eq!(second_graph["session_open_count"], 2);
    assert_eq!(first_graph["main_agent_counted"], false);
    assert!(first_graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|node| node["readiness"] == "ready"));
    assert!(second_graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .all(|node| node["readiness"] == "blocked_capacity"));
    let completed_child = first_graph["nodes"][0]["child_task_id"]
        .as_str()
        .expect("first child");
    crate::repo::child_task_graph::record_child_graph_terminal(
        &db,
        &first.task_id,
        completed_child,
        "succeeded",
        "2099-01-01T00:00:00Z",
    )
    .expect("record terminal");
    let second_after = crate::repo::child_task_graph::graph_snapshot(&db, &second.task_id)
        .expect("second graph after")
        .expect("second graph exists after");
    assert_eq!(
        second_after["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["readiness"] == "ready")
            .count(),
        1
    );
}

#[test]
fn checkpoint_resume_reuses_terminal_child_merge_without_respawning() {
    let state = persistent_test_state();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-terminal-child-resume-parent".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text": "parent task"}).to_string(),
    };
    insert_running_parent_task(&state, &task);
    let args = serde_json::json!({
        "action": "inline_readonly",
        "role": "review",
        "objective": "machine_child_objective:terminal-reuse",
        "context_refs": ["README.md"],
        "allowed_capabilities": ["filesystem.read_text_range"]
    });
    let mut loop_state = LoopState::new();
    install_test_task_budget(&mut loop_state);
    assert_eq!(
        record_durable_readonly_child_task_from_args(
            &state,
            &task,
            &mut loop_state,
            1,
            1,
            &args,
            &SubagentRuntimeConfig::default(),
        )
        .expect("schedule child"),
        Some(subagent_runtime_persistent::SUBAGENT_STOP_SIGNAL_CHILD_TASK_WAITING)
    );
    let child_task_id = loop_state.task_observations.last().unwrap()["child_task_ids"][0]
        .as_str()
        .unwrap()
        .to_string();
    {
        let db = state.core.db.get().expect("get db");
        db.execute(
            "UPDATE tasks
             SET result_json = ?2
             WHERE task_id = ?1",
            rusqlite::params![
                task.task_id,
                serde_json::json!({"child_task_ids": [child_task_id]}).to_string()
            ],
        )
        .expect("persist parent child ids");
        db.execute(
            "UPDATE tasks
             SET status = 'succeeded', result_json = ?2
             WHERE task_id = ?1",
            rusqlite::params![
                child_task_id,
                serde_json::json!({
                    "child_task_result": {
                        "schema_version": 2,
                        "parent_task_id": task.task_id,
                        "child_task_id": child_task_id,
                        "role": "reviewer",
                        "required": true,
                        "status": "succeeded",
                        "result_status": "ok",
                        "findings": {},
                        "finding_refs": [],
                        "evidence_refs": []
                    }
                })
                .to_string()
            ],
        )
        .expect("complete child");
        crate::repo::child_task_graph::record_child_graph_terminal(
            &db,
            &task.task_id,
            &child_task_id,
            "succeeded",
            "2099-01-01T00:00:00Z",
        )
        .expect("record graph terminal");
    }

    assert_eq!(
        record_durable_readonly_child_task_from_args(
            &state,
            &task,
            &mut loop_state,
            2,
            1,
            &args,
            &SubagentRuntimeConfig::default(),
        )
        .expect("reuse terminal child merge"),
        None
    );
    let observation = loop_state
        .task_observations
        .last()
        .expect("merge observation");
    assert_eq!(observation["action"], "subagent_child_task_merge_reused");
    assert_eq!(observation["status"], "ready");
    assert_eq!(observation["child_task_merge"]["terminal_child_count"], 1);
    assert!(loop_state.task_checkpoint.is_none());

    assert_eq!(
        record_durable_readonly_child_task_from_args(
            &state,
            &task,
            &mut loop_state,
            3,
            1,
            &args,
            &SubagentRuntimeConfig::default(),
        )
        .expect("reuse matching terminal invocation after checkpoint is cleared"),
        None
    );
    let replay_observation = loop_state
        .task_observations
        .last()
        .expect("matching invocation replay observation");
    assert_eq!(
        replay_observation["reuse_reason"],
        "existing_parent_child_graph_terminal"
    );
    let db = state.core.db.get().expect("get db after merge");
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, &task.task_id)
        .expect("read graph")
        .expect("graph");
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(graph["nodes"][0]["thread_state"], "closed");
    drop(db);

    let follow_up_args = serde_json::json!({
        "action": "inline_readonly",
        "node_id": "second_review",
        "role": "review",
        "objective": "machine_child_objective:a-genuinely-distinct-follow-up",
        "context_refs": ["README.md"],
        "allowed_capabilities": ["filesystem.read_text_range"]
    });
    assert_eq!(
        record_durable_readonly_child_task_from_args(
            &state,
            &task,
            &mut loop_state,
            4,
            1,
            &follow_up_args,
            &SubagentRuntimeConfig::default(),
        )
        .expect("reuse the parent child graph for a later planner call"),
        None
    );
    let replay_observation = loop_state
        .task_observations
        .last()
        .expect("follow-up replay observation");
    assert_eq!(
        replay_observation["reuse_reason"],
        "existing_parent_child_graph_terminal"
    );
    let db = state.core.db.get().expect("get db after follow-up call");
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, &task.task_id)
        .expect("read graph after follow-up call")
        .expect("graph after follow-up call");
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 1);
}

#[test]
fn persistent_subagent_batch_materializes_declared_dag_and_child_policy() {
    let state = persistent_test_state();
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-persistent-dag-parent".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text": "parent task"}).to_string(),
    };
    insert_running_parent_task(&state, &task);
    let mut loop_state = LoopState::new();
    install_test_task_budget(&mut loop_state);
    let args = serde_json::json!({
        "action": "persistent_child_task",
        "max_parallel": 2,
        "children": [
            {
                "node_id": "writer",
                "role": "writer",
                "objective": "machine_child_objective:write",
                "context_refs": ["plan/current.md"],
                "allowed_capabilities": ["filesystem.write_text"],
                "owned_paths": ["crates/runtime"]
            },
            {
                "node_id": "reviewer",
                "role": "reviewer",
                "objective": "machine_child_objective:review",
                "context_refs": ["plan/current.md"],
                "allowed_capabilities": ["filesystem.read_text_range"],
                "depends_on": [{"node_id": "writer", "required": true}]
            }
        ]
    });

    record_persistent_child_task_from_args(
        &state,
        &task,
        &mut loop_state,
        1,
        1,
        &args,
        &SubagentRuntimeConfig::default(),
    )
    .expect("schedule DAG");
    let db = state.core.db.get().expect("get db");
    let graph = crate::repo::child_task_graph::graph_snapshot(&db, &task.task_id)
        .expect("read graph")
        .expect("graph");
    let nodes = graph["nodes"].as_array().expect("nodes");
    let writer = nodes
        .iter()
        .find(|node| node["role"] == "writer")
        .expect("writer");
    let reviewer = nodes
        .iter()
        .find(|node| node["role"] == "reviewer")
        .expect("reviewer");
    assert_eq!(writer["readiness"], "ready");
    assert_eq!(writer["owned_paths"], json!(["crates/runtime"]));
    assert_eq!(
        writer["tool_policy"]["allowed_capabilities"],
        json!(["filesystem.write_text"])
    );
    assert_eq!(reviewer["readiness"], "blocked_dependency");
    assert_eq!(reviewer["model_policy"], json!({"model_class": "default"}));
    assert_eq!(graph["edges"][0]["edge_kind"], "declared_dependency");
    assert_eq!(
        graph["edges"][0]["predecessor_task_id"],
        writer["child_task_id"]
    );
    assert_eq!(
        graph["edges"][0]["successor_task_id"],
        reviewer["child_task_id"]
    );
}

#[test]
fn subagent_model_child_result_merges_into_runtime_observation() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 3;
    let args = serde_json::json!({
        "role": "review",
        "objective": "machine_boundary_review",
        "context_refs": [],
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 4, 1, &args);
    assert!(stop_signal.is_none());
    let merged = apply_model_assisted_child_result_for_test(
        &mut loop_state,
        4,
        1,
        serde_json::json!({
            "schema_version": 1,
            "owner_layer": "subagent_model_child",
            "output_format": "machine_json",
            "status": "completed",
            "findings": [{"code": "boundary_consistent"}],
            "evidence_refs": ["AGENTS.md"],
            "confidence": 0.77
        }),
    );

    assert!(merged);
    let observation = &loop_state.task_observations[0];
    assert_eq!(
        observation["execution_mode"],
        "agent_loop_readonly_child_run"
    );
    assert_eq!(observation["action"], "subagent_agent_loop_child");
    assert_eq!(observation["model_assisted"], true);
    assert_eq!(observation["agent_loop_assisted"], true);
    assert_eq!(observation["status"], "completed");
    assert_eq!(observation["delegated_terminal_evidence"], true);
    assert_eq!(observation["child_result"]["model_assisted"], true);
    assert_eq!(observation["child_request"]["state"], "completed");
    assert_eq!(observation["scheduler"]["status"], "inline_completed");
    assert_eq!(
        observation["merge_contract"]["child_trace_merge_status"],
        "merged"
    );
    assert_eq!(
        observation["child_run_summary"]["trace_merge_status"],
        "merged"
    );
    assert_eq!(observation["child_result"]["status"], "completed");
    assert_eq!(observation["child_result"]["result_status"], "completed");
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_inline_readonly_completed"
    );
    assert_eq!(
        observation["child_model_result"]["findings"][0]["code"],
        "boundary_consistent"
    );
}

#[test]
fn subagent_batch_model_results_replace_parent_scaffolding() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 2;
    let args = json!({
        "children": [
            {
                "role": "review",
                "objective": "review_runtime",
                "context_refs": ["AGENTS.md"],
                "allowed_capabilities": ["filesystem.read_text_range"]
            },
            {
                "role": "test",
                "objective": "review_tests",
                "context_refs": ["crates/clawd/src/verifier.rs"],
                "allowed_capabilities": ["filesystem.read_text_range"],
                "required": false
            }
        ]
    });
    assert!(record_subagent_action_from_args(&mut loop_state, 4, 1, &args).is_none());

    let merged = apply_model_assisted_batch_results_for_test(
        &mut loop_state,
        4,
        1,
        vec![
            (
                "subagent-batch:2:1:1:review".to_string(),
                true,
                json!({
                    "schema_version": 1,
                    "owner_layer": "subagent_model_child",
                    "output_format": "machine_json",
                    "status": "completed",
                    "role": "review",
                    "findings": [{"code": "runtime_consistent"}],
                    "evidence_refs": ["AGENTS.md"],
                    "confidence": 0.9
                }),
            ),
            (
                "subagent-batch:2:1:2:test".to_string(),
                false,
                json!({
                    "schema_version": 1,
                    "owner_layer": "subagent_model_child",
                    "output_format": "machine_json",
                    "status": "failed",
                    "role": "test",
                    "findings": [],
                    "evidence_refs": [],
                    "confidence": 0.0,
                    "error_code": "test_evidence_unavailable"
                }),
            ),
        ],
    );

    assert!(merged);
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["model_assisted"], true);
    assert_eq!(observation["status"], "partial");
    assert_eq!(observation["delegated_terminal_evidence"], true);
    assert_eq!(observation["aggregation"]["completed_count"], 1);
    assert_eq!(observation["aggregation"]["required_failed_count"], 0);
    assert_eq!(observation["aggregation"]["optional_failed_count"], 1);
    assert_eq!(
        observation["child_results"][0]["findings"][0]["code"],
        "runtime_consistent"
    );
    assert_eq!(
        observation["child_results"][0]["model_result"]["owner_layer"],
        "subagent_model_child"
    );
}

#[test]
fn subagent_model_child_parser_ignores_visible_thinking_and_nested_json() {
    let raw = r#"<think>notes with a nested but irrelevant object {"id":"F0","summary":"not result"} and refs ["/tmp/a"].</think>
{"schema_version":1,"owner_layer":"subagent_model_child","output_format":"machine_json","status":"completed","role":"review","findings":[{"code":"boundary_consistent","summary":"policy and plan align"}],"evidence_refs":["AGENTS.md","plan/current.md"],"confidence":0.82}"#;

    let parsed = parse_child_model_result_for_test(raw);

    assert_eq!(parsed["owner_layer"], "subagent_model_child");
    assert_eq!(parsed["output_format"], "machine_json");
    assert_eq!(parsed["status"], "completed");
    assert_eq!(parsed["findings"][0]["code"], "boundary_consistent");
    assert_eq!(parsed["evidence_refs"][1], "plan/current.md");
}

#[test]
fn subagent_model_child_parser_rejects_partial_nested_array_as_result() {
    let raw = r#"<think>incomplete top-level result follows</think>
{"schema_version":1,"owner_layer":"subagent_model_child","output_format":"machine_json","status":"completed","role":"review","findings":[{"code":"boundary_consistent","summary":"truncated"}],"evidence_refs":["AGENTS.md","plan/current.md"]"#;

    let parsed = parse_child_model_result_for_test(raw);

    assert_eq!(parsed["status"], "failed");
    assert_eq!(parsed["error_code"], "subagent_child_json_parse_failed");
}

#[test]
fn subagent_model_child_parser_rejects_unstructured_completion() {
    let parsed = parse_child_model_result_for_test(
        r#"{"status":"completed","summary":"looks good","evidence_refs":[]}"#,
    );

    assert_eq!(parsed["status"], "failed");
    assert_eq!(
        parsed["error_code"],
        "subagent_child_result_contract_invalid"
    );
    assert_eq!(parsed["findings"], json!([]));
}

#[test]
fn subagent_child_loop_wraps_satisfied_custom_result_contract() {
    let raw = r##"{"first_line_text":"# Agent Runtime","names_agent-runtime":true,"evidence_ref":"README.md"}"##;
    let parsed = parse_child_loop_result_for_test(
        raw,
        "reviewer",
        &json!(["README.md"]),
        &json!({
            "output_format": "machine_json",
            "require_evidence": true,
            "required_keys": ["first_line_text", "names_agent-runtime", "evidence_ref"]
        }),
    );

    assert_eq!(parsed["status"], "completed");
    assert_eq!(parsed["role"], "reviewer");
    assert_eq!(parsed["result"]["first_line_text"], "# Agent Runtime");
    assert_eq!(parsed["result"]["names_agent-runtime"], true);
    assert_eq!(parsed["evidence_refs"], json!(["README.md"]));
}

#[test]
fn subagent_child_loop_rejects_missing_custom_result_key() {
    let raw = r##"{"first_line_text":"# Agent Runtime","evidence_ref":"README.md"}"##;
    let parsed = parse_child_loop_result_for_test(
        raw,
        "reviewer",
        &json!(["README.md"]),
        &json!({
            "required_keys": ["first_line_text", "names_agent-runtime", "evidence_ref"]
        }),
    );

    assert_eq!(parsed["status"], "failed");
    assert_eq!(
        parsed["error_code"],
        "subagent_child_result_contract_invalid"
    );
}

#[test]
fn subagent_new_role_tokens_preserve_readonly_policy() {
    let mut loop_state = LoopState::new();

    let stop_signal = record_subagent_action(
        &mut loop_state,
        1,
        1,
        "worker",
        "Collect bounded evidence.",
        &[],
        SubagentActionOptions::default(),
    );

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["role"], "worker");
    assert_eq!(observation["role_metadata"]["role_family"], "worker");
    assert_eq!(
        observation["role_metadata"]["default_scope"],
        "read_only_worker"
    );
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
    assert_eq!(
        observation["cancellation_policy"]["cancel_status"],
        "cancelled"
    );
}

#[test]
fn subagent_runtime_config_separates_join_wait_from_legacy_timeout() {
    let mut loop_state = LoopState::new();
    let config = SubagentRuntimeConfig {
        role_definitions: crate::agent_runtime_contract::default_subagent_role_definitions(),
        enabled: true,
        max_concurrent_threads_per_session: 3,
        join_wait_ms: 30_000,
        max_spawn_depth: 2,
        interrupt_message: true,
        legacy_config_key_used: true,
        max_running_threads_global: Some(3),
        max_parallel_readonly: 3,
        default_timeout_ms: Some(15_000),
        context_evidence_root: None,
        resolved_model_policies: std::collections::BTreeMap::new(),
    };

    let stop_signal = record_subagent_action_with_config(
        &mut loop_state,
        2,
        1,
        "explorer",
        "Collect read-only evidence.",
        &[],
        SubagentActionOptions::default(),
        &config,
    );

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["runtime_config"]["max_parallel_readonly"], 3);
    assert_eq!(
        observation["runtime_config"]["max_concurrent_threads_per_session"],
        3
    );
    assert_eq!(observation["budget"]["runtime_deadline_ms"], Value::Null);
    assert_eq!(observation["budget"]["join_wait_ms"], 30_000);
    assert_eq!(
        observation["timeout_policy"]["policy"],
        "no_operation_deadline"
    );
    assert_eq!(
        observation["timeout_policy"]["runtime_deadline_source"],
        "none"
    );
    assert_eq!(observation["scheduler"]["max_parallel_readonly"], 3);
    assert_eq!(
        observation["child_request"]["runtime_config"]["legacy_default_timeout_ms"],
        15_000
    );
}

#[test]
fn subagent_config_v2_prefers_new_keys_and_v1_reader_is_traceable() {
    let temp = TempDirGuard::new("subagent-config-v2");
    let path = temp.path().join("agent_guard.toml");
    std::fs::write(
        &path,
        r#"
[agent.subagents]
enabled = true
max_concurrent_threads_per_session = 8
join_wait_ms = 45000
max_spawn_depth = 3
interrupt_message = false
"#,
    )
    .expect("write v2 config");
    let v2 = load_subagent_runtime_config_from_path(&path);
    assert_eq!(v2.max_concurrent_threads_per_session, 8);
    assert_eq!(v2.max_parallel_readonly, 8);
    assert_eq!(v2.join_wait_ms, 45_000);
    assert_eq!(v2.max_spawn_depth, 3);
    assert!(!v2.interrupt_message);
    assert!(!v2.legacy_config_key_used);
    assert_eq!(v2.default_timeout_ms, None);

    std::fs::write(
        &path,
        r#"
[agent.subagents]
max_parallel_readonly = 6
default_timeout_ms = 180000
"#,
    )
    .expect("write v1 config");
    let v1 = load_subagent_runtime_config_from_path(&path);
    assert_eq!(v1.max_concurrent_threads_per_session, 6);
    assert_eq!(v1.max_parallel_readonly, 6);
    assert_eq!(v1.join_wait_ms, 30_000);
    assert_eq!(v1.default_timeout_ms, Some(180_000));
    assert!(v1.legacy_config_key_used);
}

#[test]
fn subagent_runtime_config_rejects_undefined_role_as_machine_state() {
    let mut loop_state = LoopState::new();
    let config = SubagentRuntimeConfig {
        role_definitions: crate::agent_runtime_contract::default_subagent_role_definitions()
            .into_iter()
            .filter(|definition| definition.token == "observe")
            .collect(),
        enabled: true,
        max_concurrent_threads_per_session: 1,
        join_wait_ms: 30_000,
        max_spawn_depth: 2,
        interrupt_message: true,
        legacy_config_key_used: true,
        max_running_threads_global: Some(1),
        max_parallel_readonly: 1,
        default_timeout_ms: Some(5_000),
        context_evidence_root: None,
        resolved_model_policies: std::collections::BTreeMap::new(),
    };

    let stop_signal = record_subagent_action_with_config(
        &mut loop_state,
        2,
        1,
        "review",
        "Review evidence.",
        &[],
        SubagentActionOptions::default(),
        &config,
    );

    assert_eq!(stop_signal, Some(SUBAGENT_STOP_SIGNAL_INVALID_ROLE));
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["status"], "rejected");
    assert_eq!(observation["error_code"], "subagent_role_not_allowed");
    assert_eq!(observation["allowed_roles"][0], "observe");
    assert_eq!(observation["runtime_config"]["inline_write_enabled"], false);
    assert_eq!(
        observation["runtime_config"]["persistent_worktree_write_enabled"],
        true
    );
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
}

#[test]
fn subagent_batch_records_bounded_parallel_aggregation() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 5;
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "collect_file_refs",
                "context_refs": ["step_1:evidence"],
                "allowed_capabilities": ["filesystem.find_entries"],
                "findings": [
                    {
                        "kind": "file_ref",
                        "status": "found",
                        "message_key": "subagent.file_ref_found",
                        "confidence": 0.82,
                        "evidence_refs": ["step_1:evidence"],
                        "text": "ignored user-visible prose"
                    }
                ]
            },
            {
                "role": "verifier",
                "objective": "verify_contract",
                "required": true,
                "budget": {
                    "runtime_deadline_ms": 3200
                },
                "context_slice": {
                    "refs": ["step_2:evidence"],
                    "max_context_chars": 2048
                },
                "result_contract": {
                    "status": "enum",
                    "evidence_refs": "array"
                },
                "findings": [
                    {
                        "kind": "contract",
                        "status": "ok",
                        "code": "verified",
                        "evidence_refs": ["step_2:evidence"],
                        "error_text": "ignored user-visible prose"
                    }
                ]
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 9, 2, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(
        observation["execution_mode"],
        "bounded_parallel_readonly_child_runs"
    );
    assert_eq!(
        observation["aggregation"]["execution_mode"],
        "bounded_parallel_readonly_child_runs"
    );
    assert_eq!(observation["team_spec"]["spec_kind"], "agent_team_spec");
    assert_eq!(observation["team_spec"]["team_id"], "subagent-batch:5:2");
    assert_eq!(observation["team_spec"]["max_parallel"], 4);
    assert_eq!(observation["team_spec"]["write_permission"], "read_only");
    assert_eq!(
        observation["team_spec"]["conflict_policy"],
        "parent_loop_resolution_required"
    );
    assert_eq!(
        observation["team_spec"]["children"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        observation["team_lifecycle_events"][0]["event_type"],
        "agent_team_started"
    );
    assert!(observation["team_lifecycle_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "subagent_finished"));
    assert_eq!(
        observation["team_lifecycle_events"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()["event_type"],
        "agent_team_aggregated"
    );
    assert_eq!(
        observation["scheduler"]["status"],
        "bounded_parallel_completed"
    );
    assert_eq!(
        observation["scheduler"]["reason_code"],
        "bounded_parallel_readonly_execution"
    );
    assert_eq!(observation["aggregation"]["status"], "completed");
    assert_eq!(observation["aggregation"]["child_count"], 2);
    assert_eq!(observation["aggregation"]["completed_count"], 2);
    assert_eq!(
        observation["aggregation"]["finding_refs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(observation["aggregation"]["finding_count"], 2);
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["reported_count"],
        1
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["missing_count"],
        1
    );
    assert_eq!(observation["aggregation"]["conflict_count"], 0);
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_status"],
        "ready_to_synthesize"
    );
    assert_eq!(
        observation["aggregation"]["recommended_next_action"],
        "synthesize_from_child_findings"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["kind"],
        "file_ref"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["confidence"],
        0.82
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["message_key"],
        "subagent.file_ref_found"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["evidence_refs"][0],
        "step_1:evidence"
    );
    assert_eq!(
        observation["child_results"][0]["findings"][0]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|key| key["key"] == "text"),
        false
    );
    assert_eq!(
        observation["child_results"][1]["findings"][0]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|key| key["key"] == "error_text"),
        false
    );
    assert_eq!(
        observation["child_requests"][1]["timeout_policy"]["runtime_deadline_ms"],
        3200
    );
    assert_eq!(
        observation["child_requests"][1]["timeout_policy"]["terminal_status_on_deadline"],
        "timed_out"
    );
    assert_eq!(
        observation["child_requests"][1]["cancellation_policy"]["cancel_scope"],
        "child_run"
    );
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_parallel_readonly_completed"
    );
    assert_eq!(observation["write_enabled"], false);
    assert_eq!(observation["external_publish_enabled"], false);
}

#[test]
fn subagent_batch_records_conflicting_findings_for_parent_decision() {
    let mut loop_state = LoopState::new();
    loop_state.round_no = 6;
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "inspect_policy_a",
                "findings": [
                    {
                        "kind": "risk_review",
                        "status": "pass",
                        "code": "policy_state",
                        "conflict_group": "policy_state",
                        "confidence": 0.91,
                        "evidence_refs": ["step_1:evidence"]
                    }
                ]
            },
            {
                "role": "review",
                "objective": "inspect_policy_b",
                "findings": [
                    {
                        "kind": "risk_review",
                        "status": "fail",
                        "code": "policy_state",
                        "conflict_group": "policy_state",
                        "confidence": 0.73,
                        "evidence_refs": ["step_2:evidence"]
                    }
                ]
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 11, 4, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["aggregation"]["status"], "completed");
    assert_eq!(observation["aggregation"]["conflict_count"], 1);
    assert_eq!(
        observation["aggregation"]["conflict_summary"]["conflict_groups"][0]["group_ref"],
        "policy_state"
    );
    assert_eq!(
        observation["aggregation"]["conflict_summary"]["conflict_groups"][0]["status_count"],
        2
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["reported_count"],
        2
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["min"],
        0.73
    );
    assert_eq!(
        observation["aggregation"]["confidence_summary"]["max"],
        0.91
    );
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_owner"],
        "parent_agent_loop"
    );
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_required"],
        true
    );
    assert_eq!(
        observation["aggregation"]["main_thread_decision"]["decision_status"],
        "needs_conflict_resolution"
    );
    assert_eq!(
        observation["aggregation"]["recommended_next_action"],
        "resolve_child_conflicts"
    );
    assert!(observation["team_lifecycle_events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|event| event["event_type"] == "agent_team_conflict_detected"));
    assert_eq!(observation["child_run_summary"]["conflict_count"], 1);
}

#[test]
fn subagent_batch_isolates_optional_child_failures_and_parallel_limit() {
    let mut loop_state = LoopState::new();
    let config = SubagentRuntimeConfig {
        role_definitions: crate::agent_runtime_contract::default_subagent_role_definitions(),
        enabled: true,
        max_concurrent_threads_per_session: 1,
        join_wait_ms: 30_000,
        max_spawn_depth: 2,
        interrupt_message: true,
        legacy_config_key_used: true,
        max_running_threads_global: Some(1),
        max_parallel_readonly: 1,
        default_timeout_ms: Some(10_000),
        context_evidence_root: None,
        resolved_model_policies: std::collections::BTreeMap::new(),
    };
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "scheduled_optional_child"
            },
            {
                "role": "unsupported_writer_probe",
                "objective": "invalid_optional_child"
            },
            {
                "role": "worker",
                "objective": "over_parallel_budget_optional_child"
            }
        ]
    });

    let stop_signal =
        record_subagent_action_from_args_with_config(&mut loop_state, 3, 1, &args, &config);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["aggregation"]["status"], "partial");
    assert_eq!(observation["aggregation"]["completed_count"], 1);
    assert_eq!(observation["aggregation"]["rejected_count"], 1);
    assert_eq!(observation["aggregation"]["skipped_count"], 1);
    assert_eq!(observation["aggregation"]["optional_failed_count"], 2);
    assert_eq!(observation["aggregation"]["required_failed_count"], 0);
    assert_eq!(
        observation["child_results"][1]["error_code"],
        "subagent_role_not_allowed"
    );
    assert_eq!(
        observation["child_results"][2]["error_code"],
        "subagent_parallel_limit_exceeded"
    );
    assert_eq!(observation["failure_isolated"], true);
}

#[test]
fn subagent_batch_required_child_failure_stops_parent_loop() {
    let mut loop_state = LoopState::new();
    let args = serde_json::json!({
        "children": [
            {
                "role": "explorer",
                "objective": "optional_success"
            },
            {
                "role": "unsupported_writer_probe",
                "objective": "required_invalid_child",
                "required": true
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 5, 1, &args);

    assert_eq!(
        stop_signal,
        Some(SUBAGENT_STOP_SIGNAL_REQUIRED_CHILD_FAILED)
    );
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["status"], "failed");
    assert_eq!(
        observation["aggregation"]["status"],
        "failed_required_child"
    );
    assert_eq!(observation["aggregation"]["required_failed_count"], 1);
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_required_child_failed"
    );
    assert_eq!(observation["failure_isolated"], false);
}

#[test]
fn subagent_batch_expected_required_child_failure_dry_run_is_delivered() {
    let mut loop_state = LoopState::new();
    let args = serde_json::json!({
        "dry_run": true,
        "expected_failure": true,
        "children": [
            {
                "role": "explorer",
                "objective": "readonly_probe"
            },
            {
                "role": "unsupported_required_probe",
                "objective": "required_failure_probe",
                "required": true
            }
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 5, 1, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["status"], "accepted");
    assert_eq!(observation["result_status"], "completed_expected_failure");
    assert_eq!(
        observation["outcome_code"],
        "subagent_expected_required_child_failure_observed"
    );
    assert_eq!(observation["dry_run"], true);
    assert_eq!(observation["expected_failure"], true);
    assert_eq!(observation["expected_failure_delivery"], true);
    assert_eq!(observation["actual_required_child_failed"], true);
    assert_eq!(observation["actual_failure_isolated"], false);
    assert_eq!(observation["failure_isolated"], true);
    assert_eq!(
        observation["aggregation"]["status"],
        "failed_required_child"
    );
    assert_eq!(
        observation["child_result"]["outcome_code"],
        "subagent_required_child_failed"
    );
    assert_eq!(
        observation["scheduler"]["status"],
        "expected_required_child_failure_observed"
    );
    assert_eq!(
        observation["merge_contract"]["parent_result_status"],
        "completed_expected_failure"
    );
}

#[test]
fn persistent_child_specs_keep_twenty_nodes_and_allocate_each_budget_slice() {
    let task = crate::ClaimedTask {
        claim_attempt: 0,
        task_id: "task-persistent-twenty-nodes".to_string(),
        user_id: 42,
        chat_id: 7,
        user_key: Some("test-key".to_string()),
        channel: "ui".to_string(),
        external_user_id: Some("ui-user".to_string()),
        external_chat_id: Some("ui-chat".to_string()),
        kind: "ask".to_string(),
        payload_json: serde_json::json!({"text": "parent task"}).to_string(),
    };
    let children = (0..20)
        .map(|index| {
            serde_json::json!({
                "node_id": format!("node_{index}"),
                "role": "explorer",
                "objective": format!("machine_child_objective:{index}"),
                "context_refs": ["AGENTS.md"],
                "allowed_capabilities": ["filesystem.read_text_range"]
            })
        })
        .collect::<Vec<_>>();
    let args = serde_json::json!({
        "action": "persistent_child_task",
        "children": children
    });
    let mut specs = super::subagent_runtime_persistent::persistent_child_specs(
        &task,
        &args,
        &SubagentRuntimeConfig::default(),
    )
    .expect("materialize every child spec");
    assert_eq!(specs.len(), 20);

    let mut loop_state = LoopState::new();
    install_test_task_budget(&mut loop_state);
    let allocations = super::subagent_runtime_persistent::allocate_persistent_child_budgets(
        &mut loop_state,
        &mut specs,
    )
    .expect("allocate every child budget");

    assert_eq!(allocations.len(), 20);
    assert_eq!(
        loop_state
            .task_budget_slice
            .as_ref()
            .expect("budget slice")
            .allocations
            .len(),
        20
    );
    assert!(specs.iter().all(|spec| spec
        .scope
        .get("budget_allocation_id")
        .and_then(serde_json::Value::as_str)
        .is_some()));
}

#[test]
fn persistent_subagent_registry_action_selects_persistent_runtime() {
    assert!(
        super::subagent_runtime_persistent::persistent_child_task_requested(
            &serde_json::json!({"action": "persistent_child_task"})
        )
    );
    assert!(
        !super::subagent_runtime_persistent::persistent_child_task_requested(
            &serde_json::json!({"execution_mode": "persistent_child_task"})
        )
    );
}

#[test]
fn explicit_inline_registry_action_does_not_fall_into_batch_dispatch() {
    let mut loop_state = LoopState::new();
    let args = serde_json::json!({
        "action": "inline_readonly",
        "role": "review",
        "objective": "inspect_runtime_boundary",
        "children": [
            {"role": "test", "objective": "must_not_replace_single_child"}
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 1, 1, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(observation["execution_mode"], "inline_readonly_child_run");
    assert_eq!(observation["role"], "review");
    assert_eq!(observation["objective_present"], true);
    assert!(observation.get("aggregation").is_none());
}

#[test]
fn explicit_batch_registry_action_uses_bounded_batch_dispatch() {
    let mut loop_state = LoopState::new();
    let args = serde_json::json!({
        "action": "bounded_parallel_readonly",
        "children": [
            {"role": "review", "objective": "inspect_runtime_boundary"},
            {"role": "test", "objective": "inspect_test_boundary"}
        ]
    });

    let stop_signal = record_subagent_action_from_args(&mut loop_state, 1, 1, &args);

    assert!(stop_signal.is_none());
    let observation = &loop_state.task_observations[0];
    assert_eq!(
        observation["execution_mode"],
        "bounded_parallel_readonly_child_runs"
    );
    assert_eq!(observation["aggregation"]["child_count"], 2);
}
