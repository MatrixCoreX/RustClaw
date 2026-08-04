use anyhow::{Context, Result};
use clap::ValueEnum;
use serde_json::{json, Value};
use std::path::Path;

use crate::chat_session::{ModelOverride, PermissionMode};
use crate::client;
use crate::events::{task_event_lines, TaskEventLine};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct TaskSubmissionOptions {
    pub(crate) yolo: bool,
    pub(crate) permission_mode: Option<PermissionMode>,
}

pub(crate) struct ThreadAskContext<'a> {
    pub(crate) conversation_id: &'a str,
    pub(crate) session_id: &'a str,
    pub(crate) resume_task_id: Option<&'a str>,
    pub(crate) model_override: Option<&'a ModelOverride>,
    pub(crate) compacted_context_ref: Option<&'a str>,
    pub(crate) goal_ref: Option<&'a str>,
    pub(crate) rewind_anchor: Option<&'a Value>,
    pub(crate) completed_side_effect_refs: &'a [String],
    pub(crate) attachments: &'a [Value],
}

pub(crate) struct TaskStatusView {
    pub(crate) task_id: String,
    pub(crate) status: String,
    pub(crate) raw_data: serde_json::Value,
    pub(crate) result_text: Option<String>,
    pub(crate) error_text: Option<String>,
    pub(crate) events: Vec<TaskEventLine>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskPlanStepView {
    pub(crate) step_id: String,
    pub(crate) title: String,
    pub(crate) status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TaskPlanSnapshotView {
    pub(crate) plan_revision: u64,
    pub(crate) steps: Vec<TaskPlanStepView>,
    pub(crate) completed_count: usize,
    pub(crate) in_progress_step_id: Option<String>,
    pub(crate) raw: Value,
}

#[derive(Default)]
pub(crate) struct TaskResumeRequest<'a> {
    pub(crate) checkpoint_id: Option<&'a str>,
    pub(crate) resume_reason: Option<&'a str>,
    pub(crate) user_message: Option<&'a str>,
    pub(crate) new_constraints: Option<Value>,
    pub(crate) approval_request_id: Option<&'a str>,
    pub(crate) approval_decision: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub(crate) enum ApprovalDecisionArg {
    ApproveOnce,
    AlwaysForScope,
    Deny,
}

impl ApprovalDecisionArg {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ApproveOnce => "approve_once",
            Self::AlwaysForScope => "always_for_scope",
            Self::Deny => "deny",
        }
    }
}

impl TaskStatusView {
    pub(crate) fn is_terminal(&self) -> bool {
        if let Some(state) = self.execution_state() {
            if matches!(state, "completed" | "failed" | "cancelled") {
                return true;
            }
        }
        matches!(
            self.status.as_str(),
            "succeeded" | "failed" | "canceled" | "cancelled" | "timeout"
        )
    }

    pub(crate) fn is_background_waiting(&self) -> bool {
        self.execution_state().is_some_and(|state| {
            matches!(
                state,
                "waiting" | "background" | "needs_user" | "needs_confirmation"
            )
        })
    }

    pub(crate) fn pending_approval_request_id(&self) -> Option<&str> {
        let request = self
            .raw_data
            .pointer("/result_json/resume_context/approval_request")?;
        if request.get("status").and_then(Value::as_str) != Some("pending") {
            return None;
        }
        request
            .get("request_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn lifecycle(&self) -> Option<&Value> {
        self.raw_data
            .get("task_lifecycle")
            .or_else(|| self.raw_data.get("lifecycle"))
    }

    pub(crate) fn lifecycle_state(&self) -> Option<&str> {
        self.lifecycle()
            .and_then(|lifecycle| lifecycle.get("state"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn execution_state(&self) -> Option<&str> {
        self.raw_data
            .get("execution_state")
            .or_else(|| {
                self.lifecycle()
                    .and_then(|lifecycle| lifecycle.get("execution_state"))
            })
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn lifecycle_summary_tokens(&self) -> Vec<String> {
        let Some(lifecycle) = self.lifecycle() else {
            return Vec::new();
        };
        let mut tokens = Vec::new();
        for key in [
            "state",
            "execution_state",
            "db_status",
            "state_source",
            "can_poll",
            "can_cancel",
            "checkpoint_id",
            "resume_due",
            "resume_wait_seconds",
            "last_heartbeat_ts",
            "heartbeat_at",
            "lease_owner",
            "lease_expires_at",
            "claim_attempt",
            "attempt_id",
            "claimed_at",
            "resume_entrypoint",
            "resume_directive",
            "resume_reason",
            "waiting_reason_code",
            "reason_code",
            "next_action_kind",
            "next_action_ref",
            "last_successful_evidence_ref",
            "evidence_ref_count",
            "poll_ref",
            "cancel_ref",
            "next_poll_after",
            "poll_after_seconds",
            "async_job_expires_at",
            "async_job_message_key",
            "message_key",
            "terminal_reason",
        ] {
            push_value_token(&mut tokens, key, lifecycle.get(key));
        }
        if let Some(budget) = lifecycle.get("budget") {
            for key in [
                "round",
                "step",
                "llm_calls",
                "tool_calls",
                "elapsed_ms",
                "llm_elapsed_ms",
                "tool_elapsed_ms",
            ] {
                push_value_token(&mut tokens, &format!("budget.{key}"), budget.get(key));
            }
        }
        tokens
    }

    pub(crate) fn task_plan_snapshot(&self) -> Option<TaskPlanSnapshotView> {
        parse_task_plan_snapshot(self.raw_data.get("task_plan")?)
    }
}

fn parse_task_plan_snapshot(value: &Value) -> Option<TaskPlanSnapshotView> {
    if value.get("schema_version").and_then(Value::as_u64) != Some(1)
        || value.get("source").and_then(Value::as_str) != Some("task_plan")
        || value.get("data_only").and_then(Value::as_bool) != Some(true)
    {
        return None;
    }
    let plan_revision = value
        .get("plan_revision")
        .and_then(Value::as_u64)
        .filter(|revision| *revision > 0)?;
    let raw_steps = value
        .get("steps")
        .and_then(Value::as_array)
        .filter(|steps| !steps.is_empty() && steps.len() <= 64)?;
    let mut step_ids = std::collections::HashSet::new();
    let mut steps = Vec::with_capacity(raw_steps.len());
    let mut completed_count = 0usize;
    let mut in_progress_step_id = None;
    for raw_step in raw_steps {
        let step_id = raw_step
            .get("step_id")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|step_id| !step_id.is_empty())?;
        let title = raw_step
            .get("title")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|title| !title.is_empty() && !title.chars().any(char::is_control))?;
        let status = raw_step
            .get("status")
            .and_then(Value::as_str)
            .filter(|status| {
                matches!(
                    *status,
                    "pending" | "in_progress" | "completed" | "cancelled"
                )
            })?;
        if !step_ids.insert(step_id) {
            return None;
        }
        if status == "completed" {
            completed_count = completed_count.saturating_add(1);
        }
        if status == "in_progress" {
            if in_progress_step_id.is_some() {
                return None;
            }
            in_progress_step_id = Some(step_id.to_string());
        }
        steps.push(TaskPlanStepView {
            step_id: step_id.to_string(),
            title: title.to_string(),
            status: status.to_string(),
        });
    }
    Some(TaskPlanSnapshotView {
        plan_revision,
        steps,
        completed_count,
        in_progress_step_id,
        raw: value.clone(),
    })
}

fn push_value_token(parts: &mut Vec<String>, key: &str, value: Option<&Value>) {
    let Some(value) = value else {
        return;
    };
    let token = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    };
    if !token.is_empty() {
        parts.push(format!("{key}={token}"));
    }
}

pub(crate) fn submit_ask(
    base_url: &str,
    key: &str,
    text: &str,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        json!({
            "text": text
        }),
        options,
    )
}

pub(crate) fn submit_auto_review(
    base_url: &str,
    key: &str,
    target_task_id: &str,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_ask_with_payload(base_url, key, auto_review_payload(target_task_id), options)
}

pub(super) fn auto_review_payload(target_task_id: &str) -> Value {
    json!({
        "text": "auto_review",
        "entrypoint": "auto_review",
        "source": "clawcli_machine",
        "execution_profile": "coding",
        "auto_review_once": true,
        "auto_review_blocking": false,
        "review_target_task_id": target_task_id,
    })
}

pub(crate) fn submit_resume_ask(
    base_url: &str,
    key: &str,
    task_id: &str,
    text: &str,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        json!({
            "text": text,
            "resume_task_id": task_id,
            "resume_trigger": "user_followup"
        }),
        options,
    )
}

pub(crate) fn submit_exec_ask(
    base_url: &str,
    key: &str,
    text: &str,
    profile: Option<&str>,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_exec_ask_with_resume(base_url, key, text, None, profile, options)
}

pub(crate) fn submit_exec_resume_ask(
    base_url: &str,
    key: &str,
    task_id: &str,
    text: &str,
    profile: Option<&str>,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_exec_ask_with_resume(base_url, key, text, Some(task_id), profile, options)
}

fn submit_exec_ask_with_resume(
    base_url: &str,
    key: &str,
    text: &str,
    resume_task_id: Option<&str>,
    profile: Option<&str>,
    options: TaskSubmissionOptions,
) -> Result<String> {
    let current_working_directory = if profile.map(str::trim) == Some("coding") {
        Some(std::env::current_dir().context("workspace_current_directory_unavailable")?)
    } else {
        None
    };
    submit_ask_with_payload(
        base_url,
        key,
        exec_ask_payload(
            text,
            resume_task_id,
            profile,
            current_working_directory.as_deref(),
        ),
        options,
    )
}

pub(super) fn exec_ask_payload(
    text: &str,
    resume_task_id: Option<&str>,
    profile: Option<&str>,
    current_working_directory: Option<&Path>,
) -> Value {
    let profile = profile.map(str::trim).filter(|profile| !profile.is_empty());
    let mut payload = json!({
        "text": text,
        "entrypoint": "exec",
        "source": "clawcli_machine",
    });
    let object = payload.as_object_mut().expect("exec payload object");
    if let Some(profile) = profile {
        object.insert("execution_profile".to_string(), json!(profile));
    }
    if profile == Some("coding") {
        if let Some(cwd) = current_working_directory {
            object.insert(
                "workspace_context".to_string(),
                json!({
                    "schema_version": 1,
                    "current_working_directory": cwd.display().to_string(),
                }),
            );
        }
    }
    if let Some(task_id) = resume_task_id
        .map(str::trim)
        .filter(|task_id| !task_id.is_empty())
    {
        object.insert("resume_task_id".to_string(), json!(task_id));
        object.insert("resume_trigger".to_string(), json!("user_followup"));
    }
    payload
}

pub(crate) fn submit_thread_ask(
    base_url: &str,
    key: &str,
    text: &str,
    context: ThreadAskContext<'_>,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_ask_with_payload(base_url, key, threaded_ask_payload(text, context), options)
}

pub(crate) fn submit_conversation_compaction(
    base_url: &str,
    key: &str,
    conversation_id: &str,
    session_id: &str,
    resume_task_id: Option<&str>,
    options: TaskSubmissionOptions,
) -> Result<String> {
    let mut payload = json!({
        "entrypoint": "compact_conversation",
        "source": "clawcli_machine",
        "conversation_id": conversation_id,
        "thread_id": conversation_id,
        "session_id": session_id,
    });
    if let Some(task_id) = resume_task_id {
        payload["resume_task_id"] = json!(task_id);
    }
    submit_ask_with_payload(base_url, key, payload, options)
}

pub(super) fn threaded_ask_payload(text: &str, context: ThreadAskContext<'_>) -> Value {
    let mut payload = json!({
        "text": text,
        "source": "clawcli_chat",
        "conversation_id": context.conversation_id,
        "thread_id": context.conversation_id,
        "session_id": context.session_id,
    });
    let object = payload.as_object_mut().expect("thread payload object");
    if let Some(resume_task_id) = context
        .resume_task_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        object.insert("resume_task_id".to_string(), json!(resume_task_id));
        object.insert("resume_trigger".to_string(), json!("user_followup"));
    }
    if let Some(selection) = context.model_override {
        object.insert(
            "model_selection".to_string(),
            json!({
                "provider": selection.provider,
                "model": selection.model,
            }),
        );
    }
    if let Some(reference) = context.compacted_context_ref {
        object.insert("compacted_context_ref".to_string(), json!(reference));
    }
    if let Some(reference) = context.goal_ref {
        object.insert("goal_ref".to_string(), json!(reference));
    }
    if let Some(anchor) = context.rewind_anchor {
        object.insert(
            "session_rewind".to_string(),
            json!({
                "schema_version": 1,
                "anchor": anchor,
                "completed_side_effect_refs": context.completed_side_effect_refs,
            }),
        );
    }
    if !context.attachments.is_empty() {
        object.insert(
            "attachments".to_string(),
            Value::Array(context.attachments.to_vec()),
        );
    }
    payload
}

pub(crate) fn submit_goal_ask(
    base_url: &str,
    key: &str,
    payload: serde_json::Value,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_ask_with_payload(base_url, key, payload, options)
}

pub(crate) fn submit_capability(
    base_url: &str,
    key: &str,
    capability: &str,
    args: Value,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        capability_task_payload(capability, args),
        options,
    )
}

pub(super) fn capability_task_payload(capability: &str, args: Value) -> Value {
    json!({
        "entrypoint": "run_capability",
        "capability": capability,
        "args": args,
        "source": "clawcli_machine",
    })
}

pub(crate) fn submit_run_skill(
    base_url: &str,
    key: &str,
    skill_name: &str,
    args: Value,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_task_with_kind_payload(
        base_url,
        key,
        "run_skill",
        json!({
            "skill_name": skill_name,
            "args": args,
        }),
        options,
    )
}

fn submit_ask_with_payload(
    base_url: &str,
    key: &str,
    payload: serde_json::Value,
    options: TaskSubmissionOptions,
) -> Result<String> {
    submit_task_with_kind_payload(base_url, key, "ask", payload, options)
}

fn submit_task_with_kind_payload(
    base_url: &str,
    key: &str,
    kind: &str,
    payload: serde_json::Value,
    options: TaskSubmissionOptions,
) -> Result<String> {
    let url = format!("{}/tasks", client::base_v1(base_url));
    let body = json!({
        "user_key": key,
        "channel": "ui",
        "kind": kind,
        "payload": payload
    });
    let request = client::make_client()?
        .post(&url)
        .header("x-agent-key", key)
        .header("x-agent-client", "clawcli")
        .header("content-type", "application/json")
        .json(&body);
    let requested_mode = if options.yolo {
        Some("yolo")
    } else {
        options.permission_mode.map(PermissionMode::as_token)
    };
    let request = match requested_mode {
        Some(mode) => request.header("x-agent-execution-mode", mode),
        None => request,
    };
    let resp = request.send().context("submit task failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse submit response")?;
    if !status.is_success() {
        anyhow::bail!("submit returned {}: {:?}", status, body.get("error"));
    }
    let task_id = body
        .get("data")
        .and_then(|d| d.get("task_id"))
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("response missing data.task_id"))?;
    Ok(task_id.to_string())
}

pub(crate) fn get_task_status(base_url: &str, key: &str, task_id: &str) -> Result<TaskStatusView> {
    let url = format!("{}/tasks/{}", client::base_v1(base_url), task_id);
    let resp = client::make_client()?
        .get(&url)
        .header("x-agent-key", key)
        .send()
        .context("get task failed")?;
    let status_code = resp.status();
    let body: serde_json::Value = resp.json().context("parse get task response")?;
    if !status_code.is_success() {
        anyhow::bail!("get task returned {}: {:?}", status_code, body.get("error"));
    }
    let data = body
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("response missing data"))?;
    let status = data
        .get("status")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let result_json = data.get("result_json");
    let result_text = result_json.and_then(result_text_from_result_json);
    let error_text = data
        .get("error_text")
        .and_then(|e| e.as_str())
        .map(String::from);
    let events = task_event_lines(data);
    Ok(TaskStatusView {
        task_id: task_id.to_string(),
        status,
        raw_data: data.clone(),
        result_text,
        error_text,
        events,
    })
}

fn result_text_from_result_json(value: &Value) -> Option<String> {
    value
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|arr| {
            let lines: Vec<String> = arr
                .iter()
                .filter_map(|m| {
                    m.get("text")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .or_else(|| m.as_str().map(String::from))
                })
                .collect();
            (!lines.is_empty()).then(|| lines.join("\n\n"))
        })
        .or_else(|| value.get("text").and_then(Value::as_str).map(String::from))
        .or_else(|| {
            async_final_result_value(value)
                .and_then(|final_result| {
                    final_result
                        .get("output")
                        .or_else(|| final_result.get("stdout"))
                        .and_then(Value::as_str)
                })
                .map(String::from)
        })
}

pub(crate) fn async_final_result_value(value: &Value) -> Option<&Value> {
    value
        .pointer("/final_result_json")
        .or_else(|| {
            value.pointer("/task_lifecycle/resume_executor_result_projection/final_result_json")
        })
        .or_else(|| value.pointer("/lifecycle/resume_executor_result_projection/final_result_json"))
        .filter(|value| value.is_object())
}

pub(crate) fn cancel_task_by_id(
    base_url: &str,
    key: &str,
    task_id: &str,
) -> Result<serde_json::Value> {
    let url = format!("{}/tasks/cancel-by-task-id", client::base_v1(base_url));
    let payload = json!({
        "task_id": task_id,
    });
    let resp = client::make_client()?
        .post(&url)
        .header("x-agent-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("cancel task by id failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse cancel task response")?;
    if !status.is_success() {
        anyhow::bail!("cancel-task returned {}: {:?}", status, body.get("error"));
    }
    Ok(body)
}

pub(crate) fn resume_task_by_id(
    base_url: &str,
    key: &str,
    task_id: &str,
    request: TaskResumeRequest<'_>,
) -> Result<serde_json::Value> {
    let payload = resume_task_payload(task_id, request);
    task_control_by_id(
        base_url,
        key,
        "/tasks/resume-by-task-id",
        "resume-task",
        payload,
    )
}

fn resume_task_payload(task_id: &str, request: TaskResumeRequest<'_>) -> Value {
    let mut payload = json!({ "task_id": task_id });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(checkpoint_id) = non_empty_token(request.checkpoint_id) {
            obj.insert("checkpoint_id".to_string(), json!(checkpoint_id));
        }
        if let Some(resume_reason) = non_empty_token(request.resume_reason) {
            obj.insert("resume_reason".to_string(), json!(resume_reason));
        }
        if let Some(user_message) = non_empty_token(request.user_message) {
            obj.insert("user_message".to_string(), json!(user_message));
        }
        if let Some(new_constraints) = request.new_constraints {
            obj.insert("new_constraints".to_string(), new_constraints);
        }
        if let Some(approval_request_id) = non_empty_token(request.approval_request_id) {
            obj.insert(
                "approval_request_id".to_string(),
                json!(approval_request_id),
            );
        }
        if let Some(approval_decision) = non_empty_token(request.approval_decision) {
            obj.insert("approval_decision".to_string(), json!(approval_decision));
        }
    }
    payload
}

pub(crate) fn update_goal_by_task_id(
    base_url: &str,
    key: &str,
    task_id: &str,
    operation: &str,
    goal: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let mut payload = json!({
        "task_id": task_id,
        "operation": operation,
    });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(goal) = goal {
            obj.insert("goal".to_string(), goal);
        }
    }
    task_control_by_id(
        base_url,
        key,
        "/tasks/goal-by-task-id",
        "goal-control",
        payload,
    )
}

pub(crate) fn pause_task_by_id(
    base_url: &str,
    key: &str,
    task_id: &str,
    pause_seconds: u64,
) -> Result<serde_json::Value> {
    task_control_by_id(
        base_url,
        key,
        "/tasks/pause-by-task-id",
        "pause-task",
        json!({
            "task_id": task_id,
            "pause_seconds": pause_seconds,
        }),
    )
}

pub(crate) fn stop_child_tasks_by_parent(
    base_url: &str,
    key: &str,
    parent_task_id: &str,
) -> Result<serde_json::Value> {
    task_control_by_id(
        base_url,
        key,
        "/tasks/stop-child-tasks-by-parent",
        "stop-subagents",
        json!({ "parent_task_id": parent_task_id }),
    )
}

pub(crate) fn close_child_task_by_id(
    base_url: &str,
    key: &str,
    parent_task_id: &str,
    child_task_id: &str,
) -> Result<serde_json::Value> {
    task_control_by_id(
        base_url,
        key,
        "/tasks/close-child-by-task-id",
        "close-subagent",
        json!({
            "parent_task_id": parent_task_id,
            "child_task_id": child_task_id,
        }),
    )
}

fn non_empty_token(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn task_control_by_id(
    base_url: &str,
    key: &str,
    path: &str,
    operation: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value> {
    let url = format!("{}{}", client::base_v1(base_url), path);
    let resp = client::make_client()?
        .post(&url)
        .header("x-agent-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .with_context(|| format!("{operation} request failed"))?;
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .with_context(|| format!("parse {operation} response"))?;
    if !status.is_success() {
        anyhow::bail!("{operation} returned {}: {:?}", status, body.get("error"));
    }
    Ok(body)
}

#[cfg(test)]
#[path = "task_tests.rs"]
mod tests;
