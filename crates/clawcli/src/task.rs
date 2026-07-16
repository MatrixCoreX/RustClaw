use anyhow::{Context, Result};
use serde_json::{json, Value};

use crate::client;
use crate::events::{task_event_lines, TaskEventLine};

pub(crate) struct TaskStatusView {
    pub(crate) task_id: String,
    pub(crate) status: String,
    pub(crate) raw_data: serde_json::Value,
    pub(crate) result_text: Option<String>,
    pub(crate) error_text: Option<String>,
    pub(crate) events: Vec<TaskEventLine>,
}

#[derive(Default)]
pub(crate) struct TaskResumeRequest<'a> {
    pub(crate) checkpoint_id: Option<&'a str>,
    pub(crate) resume_reason: Option<&'a str>,
    pub(crate) user_message: Option<&'a str>,
    pub(crate) new_constraints: Option<Value>,
    pub(crate) approval_request_id: Option<&'a str>,
    pub(crate) approve: bool,
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

pub(crate) fn submit_ask(base_url: &str, key: &str, text: &str) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        json!({
            "text": text
        }),
    )
}

pub(crate) fn submit_resume_ask(
    base_url: &str,
    key: &str,
    task_id: &str,
    text: &str,
) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        json!({
            "text": text,
            "resume_task_id": task_id,
            "resume_trigger": "user_followup"
        }),
    )
}

pub(crate) fn submit_thread_ask(
    base_url: &str,
    key: &str,
    text: &str,
    thread_id: &str,
    session_id: &str,
    resume_task_id: Option<&str>,
) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        threaded_ask_payload(text, thread_id, session_id, resume_task_id),
    )
}

pub(super) fn threaded_ask_payload(
    text: &str,
    thread_id: &str,
    session_id: &str,
    resume_task_id: Option<&str>,
) -> Value {
    let mut payload = json!({
        "text": text,
        "source": "clawcli_chat",
        "thread_id": thread_id,
        "session_id": session_id,
    });
    if let Some(resume_task_id) = resume_task_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let object = payload.as_object_mut().expect("thread payload object");
        object.insert("resume_task_id".to_string(), json!(resume_task_id));
        object.insert("resume_trigger".to_string(), json!("user_followup"));
    }
    payload
}

pub(crate) fn submit_goal_ask(
    base_url: &str,
    key: &str,
    payload: serde_json::Value,
) -> Result<String> {
    submit_ask_with_payload(base_url, key, payload)
}

pub(crate) fn submit_capability(
    base_url: &str,
    key: &str,
    capability: &str,
    args: Value,
) -> Result<String> {
    submit_ask_with_payload(base_url, key, capability_task_payload(capability, args))
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
) -> Result<String> {
    submit_task_with_kind_payload(
        base_url,
        key,
        "run_skill",
        json!({
            "skill_name": skill_name,
            "args": args,
        }),
    )
}

fn submit_ask_with_payload(
    base_url: &str,
    key: &str,
    payload: serde_json::Value,
) -> Result<String> {
    submit_task_with_kind_payload(base_url, key, "ask", payload)
}

fn submit_task_with_kind_payload(
    base_url: &str,
    key: &str,
    kind: &str,
    payload: serde_json::Value,
) -> Result<String> {
    let url = format!("{}/tasks", client::base_v1(base_url));
    let body = json!({
        "user_key": key,
        "channel": "ui",
        "kind": kind,
        "payload": payload
    });
    let resp = client::make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .context("submit task failed")?;
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
        .header("x-rustclaw-key", key)
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
        .header("x-rustclaw-key", key)
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
        if request.approve {
            obj.insert("approve".to_string(), json!(true));
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
        .header("x-rustclaw-key", key)
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
