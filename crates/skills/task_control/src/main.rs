use std::io::{self, BufRead, Write};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod response_contract;

use response_contract::*;

const SKILL_NAME: &str = "task_control";

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    user_id: i64,
    chat_id: i64,
    #[serde(default)]
    user_key: Option<String>,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    error_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct ApiResponse {
    ok: bool,
    data: Option<Value>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ActiveTasksData {
    count: usize,
    tasks: Vec<ActiveTaskItem>,
}

#[derive(Debug, Clone, Deserialize)]
struct ActiveTaskItem {
    index: usize,
    task_id: String,
    kind: String,
    status: String,
    summary: String,
    age_seconds: i64,
}

#[derive(Debug)]
struct SkillInput {
    action: String,
    index: Option<usize>,
    task_id: Option<String>,
    checkpoint_id: Option<String>,
    resume_reason: Option<String>,
    user_message: Option<String>,
    new_constraints: Option<Value>,
    pause_seconds: Option<u64>,
    dry_run: bool,
}

#[derive(Debug)]
struct SkillOutput {
    text: String,
    extra: Option<Value>,
}

impl SkillOutput {
    fn structured(text: impl Into<String>, extra: Value) -> Self {
        Self {
            text: text.into(),
            extra: Some(extra),
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match parse_input(&req.args).and_then(|input| execute(&req, input)) {
                Ok(fut) => match fut.await {
                    Ok(output) => Resp {
                        request_id: req.request_id,
                        status: "ok".to_string(),
                        text: output.text,
                        error_text: None,
                        extra: output.extra,
                    },
                    Err(err) => Resp {
                        request_id: req.request_id,
                        status: "error".to_string(),
                        text: String::new(),
                        error_text: Some(err),
                        extra: Some(error_extra("execution_failed")),
                    },
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                    extra: Some(error_extra("invalid_input")),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
                extra: Some(error_extra("invalid_input")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn parse_input(args: &Value) -> Result<SkillInput, String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("list")
        .trim()
        .to_ascii_lowercase();
    let action = match action.as_str() {
        "list" | "query" | "status" => "list",
        "list_with_first_detail" | "list_and_get_first" | "sample_detail" => {
            "list_with_first_detail"
        }
        "get" | "get_one" | "query_task" | "task_detail" | "detail" => "get",
        "cancel" | "cancel_all" | "stop" | "stop_all" => "cancel_all",
        "cancel_one" | "cancel_index" | "cancel_number" | "stop_one" | "stop_index" => "cancel_one",
        "preview_resume" | "resume_preview" => "preview_resume",
        "resume" | "resume_task" | "continue_task" => "resume",
        "pause" | "pause_task" | "delay_task" => "pause",
        _ => return Err("unsupported_action".to_string()),
    }
    .to_string();
    let dry_run = obj
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let index = obj
        .get("index")
        .or_else(|| obj.get("task_number"))
        .or_else(|| obj.get("number"))
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|n| n.max(0) as u64)))
        .map(|v| v as usize);
    if action == "cancel_one" && index.unwrap_or(0) == 0 && !dry_run {
        return Err("cancel_one_missing_index".to_string());
    }
    let task_id = obj
        .get("task_id")
        .or_else(|| obj.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let checkpoint_id = obj
        .get("checkpoint_id")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let resume_reason = obj
        .get("resume_reason")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let user_message = obj
        .get("user_message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    let new_constraints = obj
        .get("new_constraints")
        .filter(|value| value.is_object())
        .cloned();
    let pause_seconds = obj
        .get("pause_seconds")
        .or_else(|| obj.get("seconds"))
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|n| n.max(0) as u64)))
        .filter(|value| *value > 0);
    if action == "get" && task_id.is_none() {
        return Ok(SkillInput {
            action,
            index,
            task_id,
            checkpoint_id,
            resume_reason,
            user_message,
            new_constraints,
            pause_seconds,
            dry_run,
        });
    }
    Ok(SkillInput {
        action,
        index,
        task_id,
        checkpoint_id,
        resume_reason,
        user_message,
        new_constraints,
        pause_seconds,
        dry_run,
    })
}

fn execute(
    req: &Req,
    input: SkillInput,
) -> Result<impl std::future::Future<Output = Result<SkillOutput, String>>, String> {
    let base_url = clawd_base_url();
    let timeout_secs = task_control_timeout_seconds();
    let request_id = req.request_id.clone();
    let user_id = req.user_id;
    let chat_id = req.chat_id;
    let user_key = effective_user_key(req);
    Ok(async move {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .build()
            .map_err(|e| format!("build http client failed: {e}"))?;
        match input.action.as_str() {
            "list" => {
                let tasks = fetch_active_tasks(
                    &client,
                    &base_url,
                    user_id,
                    chat_id,
                    &request_id,
                    user_key.as_deref(),
                )
                .await?;
                Ok(SkillOutput::structured(
                    task_list_extra(&tasks).to_string(),
                    task_list_extra(&tasks),
                ))
            }
            "list_with_first_detail" => {
                let tasks = fetch_active_tasks(
                    &client,
                    &base_url,
                    user_id,
                    chat_id,
                    &request_id,
                    user_key.as_deref(),
                )
                .await?;
                let detail = if let Some(first) = tasks.first() {
                    Some(
                        fetch_task_detail(&client, &base_url, &first.task_id, user_key.as_deref())
                            .await?,
                    )
                } else {
                    None
                };
                let extra = task_list_with_first_detail_extra(&tasks, detail.as_ref());
                Ok(SkillOutput::structured(extra.to_string(), extra))
            }
            "get" => {
                let Some(task_id) = input.task_id.as_deref() else {
                    let extra = task_detail_input_status_extra("missing_task_id", None);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                };
                if !is_task_id_shape(task_id) {
                    let extra = task_detail_input_status_extra("invalid_task_id", Some(task_id));
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                let detail =
                    fetch_task_detail(&client, &base_url, task_id, user_key.as_deref()).await?;
                Ok(SkillOutput::structured(
                    render_task_detail(&detail),
                    task_detail_extra(task_id, &detail),
                ))
            }
            "cancel_all" => {
                if input.dry_run {
                    let extra = cancel_dry_run_extra("cancel_all", None);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                let tasks = fetch_active_tasks(
                    &client,
                    &base_url,
                    user_id,
                    chat_id,
                    &request_id,
                    user_key.as_deref(),
                )
                .await?;
                if tasks.is_empty() {
                    let extra = cancel_all_result_extra(&tasks, 0);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                let canceled = cancel_all_tasks(
                    &client,
                    &base_url,
                    user_id,
                    chat_id,
                    &request_id,
                    user_key.as_deref(),
                )
                .await?;
                let extra = cancel_all_result_extra(&tasks, canceled);
                Ok(SkillOutput::structured(extra.to_string(), extra))
            }
            "cancel_one" => {
                let index = input.index.unwrap_or(0);
                if input.dry_run {
                    let extra = cancel_dry_run_extra("cancel_one", None);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                let task = cancel_one_task(
                    &client,
                    &base_url,
                    user_id,
                    chat_id,
                    &request_id,
                    index,
                    user_key.as_deref(),
                )
                .await?;
                let extra = cancel_one_result_extra(&task);
                Ok(SkillOutput::structured(extra.to_string(), extra))
            }
            "resume" => {
                let Some(task_id) = input.task_id.as_deref() else {
                    let extra = task_control_input_status_extra("resume", "missing_task_id", None);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                };
                if !is_task_id_shape(task_id) {
                    let extra =
                        task_control_input_status_extra("resume", "invalid_task_id", Some(task_id));
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                if input.dry_run {
                    let extra = resume_dry_run_extra(&input);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                let value = resume_task_by_id(
                    &client,
                    &base_url,
                    task_id,
                    input.checkpoint_id.as_deref(),
                    input.resume_reason.as_deref(),
                    input.user_message.as_deref(),
                    input.new_constraints.clone(),
                    user_key.as_deref(),
                )
                .await?;
                let extra = task_control_by_id_result_extra("resume", task_id, value);
                Ok(SkillOutput::structured(extra.to_string(), extra))
            }
            "preview_resume" => {
                let Some(task_id) = input.task_id.as_deref() else {
                    let extra =
                        task_control_input_status_extra("preview_resume", "missing_task_id", None);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                };
                if !is_task_id_shape(task_id) {
                    let extra = task_control_input_status_extra(
                        "preview_resume",
                        "invalid_task_id",
                        Some(task_id),
                    );
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                let extra = resume_preview_extra(&input);
                Ok(SkillOutput::structured(extra.to_string(), extra))
            }
            "pause" => {
                let Some(task_id) = input.task_id.as_deref() else {
                    let extra = task_control_input_status_extra("pause", "missing_task_id", None);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                };
                if !is_task_id_shape(task_id) {
                    let extra =
                        task_control_input_status_extra("pause", "invalid_task_id", Some(task_id));
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                if input.dry_run {
                    let extra = pause_dry_run_extra(&input);
                    return Ok(SkillOutput::structured(extra.to_string(), extra));
                }
                let pause_seconds = input.pause_seconds.unwrap_or(3600);
                let value = pause_task_by_id(
                    &client,
                    &base_url,
                    task_id,
                    pause_seconds,
                    user_key.as_deref(),
                )
                .await?;
                let extra = task_control_by_id_result_extra("pause", task_id, value);
                Ok(SkillOutput::structured(extra.to_string(), extra))
            }
            _ => Err("unsupported action".to_string()),
        }
    })
}

fn clawd_base_url() -> String {
    std::env::var("CLAWD_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "http://127.0.0.1:8787".to_string())
}

fn task_control_timeout_seconds() -> u64 {
    std::env::var("TASK_CONTROL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .or_else(|| {
            std::env::var("SKILL_TIMEOUT_SECONDS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
        })
        .unwrap_or(30)
}

async fn fetch_active_tasks(
    client: &reqwest::Client,
    base_url: &str,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: &str,
    user_key: Option<&str>,
) -> Result<Vec<ActiveTaskItem>, String> {
    let mut req = client.post(format!(
        "{}/v1/tasks/active",
        base_url.trim_end_matches('/')
    ));
    if let Some(key) = user_key {
        req = req.header("x-rustclaw-key", key);
    }
    let resp = req
        .json(&json!({
            "user_id": user_id,
            "chat_id": chat_id,
            "exclude_task_id": exclude_task_id,
        }))
        .send()
        .await
        .map_err(|e| format!("request active tasks failed: {e}"))?;
    parse_api_response::<ActiveTasksData>(resp).await.map(|v| {
        debug_assert_eq!(v.count, v.tasks.len());
        v.tasks
    })
}

async fn fetch_task_detail(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    user_key: Option<&str>,
) -> Result<Value, String> {
    let mut req = client.get(format!(
        "{}/v1/tasks/{}",
        base_url.trim_end_matches('/'),
        task_id
    ));
    if let Some(key) = user_key {
        req = req.header("x-rustclaw-key", key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("request task detail failed: {e}"))?;
    parse_api_response::<Value>(resp).await
}

async fn cancel_all_tasks(
    client: &reqwest::Client,
    base_url: &str,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: &str,
    user_key: Option<&str>,
) -> Result<usize, String> {
    let mut req = client.post(format!(
        "{}/v1/tasks/cancel",
        base_url.trim_end_matches('/')
    ));
    if let Some(key) = user_key {
        req = req.header("x-rustclaw-key", key);
    }
    let resp = req
        .json(&json!({
            "user_id": user_id,
            "chat_id": chat_id,
            "exclude_task_id": exclude_task_id,
        }))
        .send()
        .await
        .map_err(|e| format!("request cancel tasks failed: {e}"))?;
    let value = parse_api_response::<Value>(resp).await?;
    Ok(value.get("canceled").and_then(|v| v.as_u64()).unwrap_or(0) as usize)
}

async fn cancel_one_task(
    client: &reqwest::Client,
    base_url: &str,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: &str,
    index: usize,
    user_key: Option<&str>,
) -> Result<ActiveTaskItem, String> {
    let mut req = client.post(format!(
        "{}/v1/tasks/cancel-one",
        base_url.trim_end_matches('/')
    ));
    if let Some(key) = user_key {
        req = req.header("x-rustclaw-key", key);
    }
    let resp = req
        .json(&json!({
            "user_id": user_id,
            "chat_id": chat_id,
            "index": index,
            "exclude_task_id": exclude_task_id,
        }))
        .send()
        .await
        .map_err(|e| format!("request cancel one task failed: {e}"))?;
    let value = parse_api_response::<Value>(resp).await?;
    serde_json::from_value(
        value
            .get("task")
            .cloned()
            .ok_or_else(|| "cancel-one response missing task".to_string())?,
    )
    .map_err(|e| format!("decode canceled task failed: {e}"))
}

async fn resume_task_by_id(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    checkpoint_id: Option<&str>,
    resume_reason: Option<&str>,
    user_message: Option<&str>,
    new_constraints: Option<Value>,
    user_key: Option<&str>,
) -> Result<Value, String> {
    let mut payload = json!({ "task_id": task_id });
    if let Some(obj) = payload.as_object_mut() {
        insert_optional_token(obj, "checkpoint_id", checkpoint_id);
        insert_optional_token(obj, "resume_reason", resume_reason);
        insert_optional_token(obj, "user_message", user_message);
        if let Some(new_constraints) = new_constraints {
            obj.insert("new_constraints".to_string(), new_constraints);
        }
    }
    post_task_control_by_id(
        client,
        base_url,
        "/v1/tasks/resume-by-task-id",
        payload,
        user_key,
    )
    .await
}

async fn pause_task_by_id(
    client: &reqwest::Client,
    base_url: &str,
    task_id: &str,
    pause_seconds: u64,
    user_key: Option<&str>,
) -> Result<Value, String> {
    post_task_control_by_id(
        client,
        base_url,
        "/v1/tasks/pause-by-task-id",
        json!({
            "task_id": task_id,
            "pause_seconds": pause_seconds,
        }),
        user_key,
    )
    .await
}

async fn post_task_control_by_id(
    client: &reqwest::Client,
    base_url: &str,
    path: &str,
    payload: Value,
    user_key: Option<&str>,
) -> Result<Value, String> {
    let mut req = client.post(format!("{}{}", base_url.trim_end_matches('/'), path));
    if let Some(key) = user_key {
        req = req.header("x-rustclaw-key", key);
    }
    let resp = req
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("request task control by id failed: {e}"))?;
    parse_api_response::<Value>(resp).await
}

fn insert_optional_token(obj: &mut serde_json::Map<String, Value>, key: &str, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        obj.insert(key.to_string(), json!(value));
    }
}

fn effective_user_key(req: &Req) -> Option<String> {
    req.user_key
        .clone()
        .or_else(|| {
            req.context
                .as_ref()
                .and_then(|ctx| ctx.get("user_key"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        })
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

async fn parse_api_response<T: for<'de> Deserialize<'de>>(
    resp: reqwest::Response,
) -> Result<T, String> {
    let status = resp.status();
    let api: ApiResponse = resp
        .json()
        .await
        .map_err(|e| format!("decode api response failed: {e}"))?;
    if !status.is_success() || !api.ok {
        return Err(api
            .error
            .unwrap_or_else(|| format!("request failed with status {status}")));
    }
    let data = api
        .data
        .ok_or_else(|| "api response missing data".to_string())?;
    serde_json::from_value(data).map_err(|e| format!("decode api data failed: {e}"))
}

fn task_list_extra(tasks: &[ActiveTaskItem]) -> Value {
    let states = task_list_states_surface(tasks);
    let items: Vec<Value> = tasks
        .iter()
        .map(|task| {
            json!({
                "index": task.index,
                "task_id": task.task_id,
                "kind": task.kind,
                "status": task.status,
                "summary": task.summary,
                "age_seconds": task.age_seconds,
            })
        })
        .collect();
    let task_count = tasks.len();
    let status = if task_count == 0 { "empty" } else { "ok" };
    let message_key = if task_count == 0 {
        "task_control.list.empty"
    } else {
        "task_control.list.ok"
    };
    json!({
        "schema_version": 1,
        "action": "list",
        "status": status,
        "message_key": message_key,
        "count": task_count,
        "task_count": task_count,
        "has_unfinished": task_count > 0,
        "states": states,
        "can_poll": task_count > 0,
        "can_cancel": task_count > 0,
        "checkpoint_id_present": false,
        "items": items,
        "field_value": {
            "action": "list",
            "status": status,
            "message_key": message_key,
            "count": task_count,
            "task_count": task_count,
            "has_unfinished": task_count > 0,
            "states": states,
            "can_poll": task_count > 0,
            "can_cancel": task_count > 0,
            "checkpoint_id_present": false,
        },
    })
}

fn task_list_states_surface(tasks: &[ActiveTaskItem]) -> String {
    let mut states = Vec::new();
    for task in tasks {
        let status = task.status.trim();
        if status.is_empty() || states.iter().any(|existing| existing == status) {
            continue;
        }
        states.push(status.to_string());
    }
    if states.is_empty() {
        "none".to_string()
    } else {
        states.join(",")
    }
}

fn task_list_with_first_detail_extra(tasks: &[ActiveTaskItem], detail: Option<&Value>) -> Value {
    let list = task_list_extra(tasks);
    let selected_task_id = tasks.first().map(|task| task.task_id.as_str());
    let detail_extra = detail.map(|detail| {
        task_detail_extra(
            detail
                .get("task_id")
                .and_then(Value::as_str)
                .or(selected_task_id)
                .unwrap_or_default(),
            detail,
        )
    });
    let lifecycle = detail_extra
        .as_ref()
        .and_then(|value| value.get("lifecycle"))
        .cloned()
        .unwrap_or(Value::Null);
    let db_status = detail_extra
        .as_ref()
        .and_then(|value| value.get("db_status"))
        .cloned()
        .unwrap_or(Value::Null);
    let lifecycle_present_fields = json!({
        "has_state": lifecycle.get("state").is_some(),
        "has_can_poll": lifecycle.get("can_poll").is_some(),
        "has_can_cancel": lifecycle.get("can_cancel").is_some(),
        "has_last_heartbeat_ts": lifecycle.get("last_heartbeat_ts").is_some(),
        "has_checkpoint_id": lifecycle.get("checkpoint_id").is_some(),
        "has_db_status": !db_status.is_null(),
    });
    let detail_available = detail_extra.is_some();
    let count = tasks.len();
    let state = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .or_else(|| {
            detail_extra
                .as_ref()
                .and_then(|value| value.get("status"))
                .and_then(Value::as_str)
        })
        .filter(|value| !value.trim().is_empty())
        .unwrap_or(if count == 0 { "none" } else { "unknown" })
        .to_string();
    let can_poll = lifecycle
        .get("can_poll")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let can_cancel = lifecycle
        .get("can_cancel")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let checkpoint_id_present = lifecycle
        .get("checkpoint_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|value| !value.is_empty());
    json!({
        "schema_version": 1,
        "action": "list_with_first_detail",
        "status": if count == 0 { "empty" } else { "ok" },
        "message_key": if count == 0 { "task_control.list.empty" } else { "task_control.list_with_first_detail.ok" },
        "count": count,
        "selected_task_id": selected_task_id,
        "state": state.clone(),
        "can_poll": can_poll,
        "can_cancel": can_cancel,
        "checkpoint_id_present": checkpoint_id_present,
        "list": list,
        "detail": detail_extra.unwrap_or(Value::Null),
        "field_value": {
            "action": "list_with_first_detail",
            "status": if count == 0 { "empty" } else { "ok" },
            "message_key": if count == 0 { "task_control.list.empty" } else { "task_control.list_with_first_detail.ok" },
            "count": count,
            "selected_task_id": selected_task_id,
            "state": state,
            "can_poll": can_poll,
            "can_cancel": can_cancel,
            "checkpoint_id_present": checkpoint_id_present,
            "detail_available": detail_available,
            "list_item_fields": ["index", "task_id", "kind", "status", "summary", "age_seconds"],
            "db_status": db_status,
            "lifecycle": lifecycle,
            "lifecycle_present_fields": lifecycle_present_fields,
        },
    })
}

fn render_task_detail(detail: &Value) -> String {
    serde_json::to_string(&task_detail_extra(
        detail
            .get("task_id")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        detail,
    ))
    .unwrap_or_else(|_| "{\"action\":\"get\",\"status\":\"decode_error\"}".to_string())
}

fn task_detail_extra(task_id: &str, detail: &Value) -> Value {
    let status = detail
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let lifecycle = detail.get("lifecycle").cloned().unwrap_or(Value::Null);
    json!({
        "schema_version": 1,
        "action": "get",
        "status": if status.is_empty() { "unknown" } else { status },
        "message_key": "task_control.get.ok",
        "task_id": task_id,
        "db_status": status,
        "lifecycle": lifecycle.clone(),
        "field_value": {
            "action": "get",
            "message_key": "task_control.get.ok",
            "task_id": task_id,
            "db_status": status,
            "lifecycle": detail.get("lifecycle").cloned().unwrap_or(Value::Null),
        },
    })
}

fn is_task_id_shape(task_id: &str) -> bool {
    task_id.len() == 36
        && task_id.char_indices().all(|(idx, ch)| match idx {
            8 | 13 | 18 | 23 => ch == '-',
            _ => ch.is_ascii_hexdigit(),
        })
}
