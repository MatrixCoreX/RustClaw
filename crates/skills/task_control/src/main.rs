use std::io::{self, BufRead, Write};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
}

#[derive(Debug)]
struct SkillOutput {
    text: String,
    extra: Option<Value>,
}

impl SkillOutput {
    fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            extra: None,
        }
    }

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
                        extra: None,
                    },
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    error_text: Some(err),
                    extra: None,
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                error_text: Some(format!("invalid input: {err}")),
                extra: None,
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
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
        "get" | "get_one" | "query_task" | "task_detail" | "detail" => "get",
        "cancel" | "cancel_all" | "stop" | "stop_all" => "cancel_all",
        "cancel_one" | "cancel_index" | "cancel_number" | "stop_one" | "stop_index" => "cancel_one",
        _ => {
            return Err("unsupported action; use list | get | cancel_all | cancel_one".to_string())
        }
    }
    .to_string();
    let index = obj
        .get("index")
        .or_else(|| obj.get("task_number"))
        .or_else(|| obj.get("number"))
        .and_then(|v| v.as_u64().or_else(|| v.as_i64().map(|n| n.max(0) as u64)))
        .map(|v| v as usize);
    if action == "cancel_one" && index.unwrap_or(0) == 0 {
        return Err("cancel_one requires index >= 1".to_string());
    }
    let task_id = obj
        .get("task_id")
        .or_else(|| obj.get("id"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);
    if action == "get" && task_id.is_none() {
        return Err("get requires task_id".to_string());
    }
    Ok(SkillInput {
        action,
        index,
        task_id,
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
                    render_task_list(&tasks),
                    task_list_extra(&tasks),
                ))
            }
            "get" => {
                let task_id = input
                    .task_id
                    .as_deref()
                    .ok_or_else(|| "get requires task_id".to_string())?;
                let detail =
                    fetch_task_detail(&client, &base_url, task_id, user_key.as_deref()).await?;
                Ok(SkillOutput::structured(
                    render_task_detail(&detail),
                    task_detail_extra(task_id, &detail),
                ))
            }
            "cancel_all" => {
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
                    return Ok(SkillOutput::text("当前没有可结束的未完成任务。"));
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
                Ok(SkillOutput::text(render_cancel_all(tasks, canceled)))
            }
            "cancel_one" => {
                let index = input.index.unwrap_or(0);
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
                Ok(SkillOutput::text(render_cancel_one(&task)))
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

fn render_task_list(tasks: &[ActiveTaskItem]) -> String {
    if tasks.is_empty() {
        return "当前没有未完成任务。".to_string();
    }
    let mut lines = vec![format!("当前未完成任务（{} 个）：", tasks.len())];
    for task in tasks {
        lines.push(format!(
            "{}. [{}][{}] {}（已运行 {}s）",
            task.index, task.status, task.kind, task.summary, task.age_seconds
        ));
    }
    lines.join("\n")
}

fn task_list_extra(tasks: &[ActiveTaskItem]) -> Value {
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
    json!({
        "schema_version": 1,
        "action": "list",
        "status": status,
        "count": task_count,
        "task_count": task_count,
        "has_unfinished": task_count > 0,
        "items": items,
        "field_value": {
            "action": "list",
            "status": status,
            "count": task_count,
            "task_count": task_count,
            "has_unfinished": task_count > 0,
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
        "task_id": task_id,
        "db_status": status,
        "lifecycle": lifecycle,
        "field_value": {
            "action": "get",
            "task_id": task_id,
            "db_status": status,
            "lifecycle": detail.get("lifecycle").cloned().unwrap_or(Value::Null),
        },
    })
}

fn render_cancel_all(tasks: Vec<ActiveTaskItem>, canceled: usize) -> String {
    let mut lines = vec![format!("已结束 {} 个任务。", canceled)];
    if canceled > 0 {
        lines.push("本次结束的任务：".to_string());
        for task in tasks.into_iter().take(canceled.max(1)) {
            lines.push(format!(
                "{}. [{}][{}] {}",
                task.index, task.status, task.kind, task.summary
            ));
        }
    }
    lines.join("\n")
}

fn render_cancel_one(task: &ActiveTaskItem) -> String {
    format!(
        "已结束任务 #{}。\n[{}][{}] {}\ntask_id: {}",
        task.index, task.status, task.kind, task.summary, task.task_id
    )
}
