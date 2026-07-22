use super::*;

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.task_control.execution_failed");
    assert_eq!(extra["retryable"], false);
}

fn sample_task(index: usize, task_id: &str, status: &str) -> ActiveTaskItem {
    ActiveTaskItem {
        index,
        task_id: task_id.to_string(),
        kind: "ask".to_string(),
        status: status.to_string(),
        summary: "task-summary".to_string(),
        age_seconds: 12,
    }
}

#[test]
fn list_extra_is_machine_renderable_contract() {
    let tasks = vec![sample_task(
        1,
        "00000000-0000-4000-8000-000000000001",
        "running",
    )];

    let extra = task_list_extra(&tasks);

    assert_eq!(extra.get("action").and_then(Value::as_str), Some("list"));
    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("task_control.list.ok")
    );
    assert_eq!(extra.get("task_count").and_then(Value::as_u64), Some(1));
    assert_eq!(
        extra.pointer("/items/0/task_id").and_then(Value::as_str),
        Some("00000000-0000-4000-8000-000000000001")
    );
    assert!(extra.to_string().starts_with('{'));
}

#[test]
fn empty_list_extra_uses_message_key_not_sentence_contract() {
    let extra = task_list_extra(&[]);

    assert_eq!(extra.get("status").and_then(Value::as_str), Some("empty"));
    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("task_control.list.empty")
    );
    assert_eq!(
        extra
            .pointer("/field_value/has_unfinished")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(extra.get("states").and_then(Value::as_str), Some("none"));
    assert_eq!(extra.get("can_poll").and_then(Value::as_bool), Some(false));
    assert_eq!(
        extra.get("checkpoint_id_present").and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn cancel_all_result_extra_is_structured_for_success_and_empty() {
    let tasks = vec![
        sample_task(1, "00000000-0000-4000-8000-000000000001", "running"),
        sample_task(2, "00000000-0000-4000-8000-000000000002", "queued"),
    ];

    let success = cancel_all_result_extra(&tasks, 2);
    assert_eq!(
        success.get("message_key").and_then(Value::as_str),
        Some("task_control.cancel_all.ok")
    );
    assert_eq!(
        success.get("canceled_count").and_then(Value::as_u64),
        Some(2)
    );
    assert_eq!(
        success
            .pointer("/field_value/task_ids/1")
            .and_then(Value::as_str),
        Some("00000000-0000-4000-8000-000000000002")
    );

    let empty = cancel_all_result_extra(&[], 0);
    assert_eq!(empty.get("status").and_then(Value::as_str), Some("empty"));
    assert_eq!(
        empty.get("message_key").and_then(Value::as_str),
        Some("task_control.cancel_all.empty")
    );
}

#[test]
fn cancel_one_result_extra_carries_task_identity() {
    let task = sample_task(3, "00000000-0000-4000-8000-000000000003", "running");

    let extra = cancel_one_result_extra(&task);

    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("task_control.cancel_one.ok")
    );
    assert_eq!(
        extra
            .pointer("/field_value/task_id")
            .and_then(Value::as_str),
        Some("00000000-0000-4000-8000-000000000003")
    );
    assert_eq!(
        extra.pointer("/field_value/index").and_then(Value::as_u64),
        Some(3)
    );
}

#[test]
fn parse_input_reports_machine_error_for_missing_cancel_index() {
    let err = parse_input(&json!({"action":"cancel_one"})).expect_err("missing index");

    assert_eq!(err, "cancel_one_missing_index");
}

#[test]
fn parse_input_accepts_resume_and_pause_machine_actions() {
    let resume = parse_input(&json!({
        "action": "resume",
        "task_id": "00000000-0000-4000-8000-000000000010",
        "checkpoint_id": "ckpt-1",
        "resume_reason": "manual_resume",
        "user_message": "continue",
        "new_constraints": {"max_rounds": 2}
    }))
    .expect("resume input");

    assert_eq!(resume.action, "resume");
    assert_eq!(
        resume.task_id.as_deref(),
        Some("00000000-0000-4000-8000-000000000010")
    );
    assert_eq!(resume.checkpoint_id.as_deref(), Some("ckpt-1"));
    assert_eq!(resume.resume_reason.as_deref(), Some("manual_resume"));
    assert!(resume.user_message.is_some());
    assert!(resume.new_constraints.is_some());

    let pause = parse_input(&json!({
        "action": "pause",
        "task_id": "00000000-0000-4000-8000-000000000011",
        "pause_seconds": 120
    }))
    .expect("pause input");

    assert_eq!(pause.action, "pause");
    assert_eq!(pause.pause_seconds, Some(120));

    let preview = parse_input(&json!({
        "action": "preview_resume",
        "task_id": "00000000-0000-4000-8000-000000000012",
        "checkpoint_id": "ckpt-preview"
    }))
    .expect("resume preview input");

    assert_eq!(preview.action, "preview_resume");
    assert_eq!(preview.checkpoint_id.as_deref(), Some("ckpt-preview"));

    let provider_failure = parse_input(&json!({
        "action": "preview_provider_failure",
        "failure_class": "quota_exhausted"
    }))
    .expect("provider failure preview input");

    assert_eq!(provider_failure.action, "preview_provider_failure");
    assert_eq!(
        provider_failure.failure_class.as_deref(),
        Some("quota_exhausted")
    );

    let coding_repair = parse_input(&json!({
        "action": "preview_coding_repair"
    }))
    .expect("coding repair preview input");

    assert_eq!(coding_repair.action, "preview_coding_repair");
}

#[test]
fn session_alias_action_requires_and_preserves_structured_values() {
    let input = parse_input(&json!({
        "action": "bind_session_alias",
        "alias": "甲文件",
        "target": "scripts/nl_tests/fixtures/device_local/docs/service_notes.md"
    }))
    .expect("session alias input");

    assert_eq!(input.action, "bind_session_alias");
    assert_eq!(input.alias.as_deref(), Some("甲文件"));
    assert_eq!(
        input.alias_target.as_deref(),
        Some("scripts/nl_tests/fixtures/device_local/docs/service_notes.md")
    );

    assert_eq!(
        parse_input(&json!({"action":"bind_session_alias", "target":"note.md"}))
            .expect_err("alias required"),
        "bind_session_alias_missing_alias"
    );
    assert_eq!(
        parse_input(&json!({"action":"bind_session_alias", "alias":"note"}))
            .expect_err("target required"),
        "bind_session_alias_missing_target"
    );
}

#[test]
fn session_alias_extra_is_machine_state_evidence() {
    let extra = session_alias_binding_extra("note file", "document/note.md");

    assert_eq!(extra["action"], "bind_session_alias");
    assert_eq!(extra["status"], "ok");
    assert_eq!(extra["session_alias_bindings"][0]["alias"], "note file");
    assert_eq!(
        extra["session_alias_bindings"][0]["target"],
        "document/note.md"
    );
}

#[test]
fn resume_and_pause_dry_run_extras_are_machine_contracts() {
    let cancel_extra = cancel_dry_run_extra("cancel_one", Some("task-1"));
    assert_eq!(cancel_extra["dry_run"], true);
    assert_eq!(cancel_extra["field_value"]["dry_run"], true);

    let resume = parse_input(&json!({
        "action": "resume",
        "task_id": "00000000-0000-4000-8000-000000000010",
        "checkpoint_id": "ckpt-1",
        "resume_reason": "manual_resume",
        "dry_run": true
    }))
    .expect("resume input");
    let resume_extra = resume_dry_run_extra(&resume);

    assert_eq!(
        resume_extra.get("message_key").and_then(Value::as_str),
        Some("task_control.resume.dry_run")
    );
    assert_eq!(resume_extra["dry_run"], true);
    assert_eq!(resume_extra["field_value"]["dry_run"], true);
    assert_eq!(
        resume_extra
            .pointer("/field_value/checkpoint_id")
            .and_then(Value::as_str),
        Some("ckpt-1")
    );
    assert_eq!(
        resume_extra
            .pointer("/result_projection_fields/resume_due")
            .and_then(Value::as_bool),
        Some(true)
    );

    let pause = parse_input(&json!({
        "action": "pause",
        "task_id": "00000000-0000-4000-8000-000000000011",
        "pause_seconds": 120,
        "dry_run": true
    }))
    .expect("pause input");
    let pause_extra = pause_dry_run_extra(&pause);

    assert_eq!(
        pause_extra.get("message_key").and_then(Value::as_str),
        Some("task_control.pause.dry_run")
    );
    assert_eq!(pause_extra["dry_run"], true);
    assert_eq!(pause_extra["field_value"]["dry_run"], true);
    assert_eq!(
        pause_extra
            .pointer("/field_value/pause_seconds")
            .and_then(Value::as_u64),
        Some(120)
    );
    assert_eq!(
        pause_extra
            .pointer("/result_projection_fields/resume_due")
            .and_then(Value::as_bool),
        Some(false)
    );
}

#[test]
fn resume_preview_is_read_only_and_exposes_entrypoint_and_lease_contract() {
    let preview = parse_input(&json!({
        "action": "preview_resume",
        "task_id": "00000000-0000-4000-8000-000000000024",
        "checkpoint_id": "ckpt-tier1"
    }))
    .expect("resume preview input");
    let extra = resume_preview_extra(&preview);

    assert_eq!(extra["action"], "preview_resume");
    assert_eq!(extra["status"], "dry_run");
    assert_eq!(extra["dry_run"], true);
    assert_eq!(extra["would_mutate"], false);
    assert_eq!(extra["resume_entrypoint"], "checkpoint_declared");
    assert_eq!(extra["lease"]["mode"], "renewable");
    assert_eq!(extra["lease"]["heartbeat_renewal"], true);
    assert_eq!(
        extra["field_value"]["checkpoint_id"],
        serde_json::json!("ckpt-tier1")
    );
    assert_eq!(extra["field_value"]["lease_required"], true);
    assert_eq!(extra["field_value"]["dry_run"], true);
}

#[test]
fn provider_failure_preview_uses_shared_read_only_wait_contract() {
    let policy = claw_core::provider_failure_policy::ProviderFailureClass::QuotaExhausted.policy();
    let extra = provider_failure_preview_extra(policy);

    assert_eq!(extra["action"], "preview_provider_failure");
    assert_eq!(extra["status"], "dry_run");
    assert_eq!(extra["would_mutate"], false);
    assert_eq!(extra["failure_class"], "quota_exhausted");
    assert_eq!(extra["provider_retryable"], false);
    assert_eq!(extra["provider_blocker"], true);
    assert_eq!(extra["retry_policy"], "background_wait");
    assert_eq!(extra["retry_after_seconds"], 10_800);
    assert_eq!(extra["waiting_state"], "waiting");
    assert_eq!(extra["checkpoint"]["required"], true);
    assert_eq!(
        extra["checkpoint"]["resume_reason"],
        "provider_blocker_wait_background"
    );
    assert_eq!(
        extra["checkpoint"]["resume_entrypoint"],
        "next_planner_round"
    );
}

#[test]
fn coding_repair_preview_is_structured_and_side_effect_free() {
    let extra = coding_repair_preview_extra();

    assert_eq!(extra["action"], "preview_coding_repair");
    assert_eq!(extra["status"], "dry_run");
    assert_eq!(extra["synthetic"], true);
    assert_eq!(extra["would_mutate"], false);
    assert_eq!(extra["would_execute_command"], false);
    assert_eq!(
        extra["checkpoint"]["checkpoint_ref"],
        "dry_run:checkpoint:pre_patch"
    );
    assert_eq!(extra["diff"]["patch_ref"], "dry_run:patch:repair_attempt_1");
    assert_eq!(extra["failed_verification"]["status"], "failed");
    assert_eq!(extra["repair_attempt"]["attempt"], 1);
    assert_eq!(extra["passing_verification"]["status"], "passed");
    assert_eq!(
        extra["rewind_references"][0],
        "dry_run:checkpoint:pre_patch"
    );
}

#[test]
fn task_control_by_id_result_extra_projects_lifecycle() {
    let response = json!({
        "task_id": "00000000-0000-4000-8000-000000000012",
        "status": "running",
        "lifecycle": {
            "state": "background",
            "checkpoint_id": "ckpt-2",
            "can_poll": true
        }
    });

    let extra =
        task_control_by_id_result_extra("resume", "00000000-0000-4000-8000-000000000012", response);

    assert_eq!(
        extra.get("message_key").and_then(Value::as_str),
        Some("task_control.resume.ok")
    );
    assert_eq!(
        extra
            .pointer("/field_value/lifecycle/checkpoint_id")
            .and_then(Value::as_str),
        Some("ckpt-2")
    );
}

#[test]
fn list_with_first_detail_projects_requested_lifecycle_fields() {
    let tasks = vec![sample_task(
        1,
        "00000000-0000-4000-8000-000000000020",
        "running",
    )];
    let detail = json!({
        "task_id": "00000000-0000-4000-8000-000000000020",
        "status": "running",
        "lifecycle": {
            "state": "background",
            "checkpoint_id": "ckpt-20",
            "can_poll": true,
            "can_cancel": false
        }
    });

    let extra = task_list_with_first_detail_extra(&tasks, Some(&detail));

    assert_eq!(
        extra.get("state").and_then(Value::as_str),
        Some("background")
    );
    assert_eq!(extra.get("can_poll").and_then(Value::as_bool), Some(true));
    assert_eq!(
        extra.get("can_cancel").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        extra.get("checkpoint_id_present").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        extra
            .pointer("/field_value/checkpoint_id_present")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        extra
            .pointer("/field_value/lifecycle_present_fields/has_state")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(extra
        .pointer("/field_value/lifecycle_field_presence")
        .is_none());
}
