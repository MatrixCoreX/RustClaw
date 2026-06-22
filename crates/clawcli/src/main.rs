//! clawcli — 终端 CLI，与 clawd 交互，默认从数据库读取 admin key（或 RUSTCLAW_ADMIN_KEY）。

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use reqwest::blocking::Client;
use serde_json::json;
use std::path::{Path, PathBuf};

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8787";
const V1: &str = "/v1";
const DEFAULT_SQLITE_PATH: &str = "data/rustclaw.db";
const CONFIG_REL: &str = "configs/config.toml";

/// 从当前目录向上查找包含 configs/config.toml 的工作区根目录；或使用环境变量 RUSTCLAW_WORKSPACE
fn find_workspace_root() -> Option<std::path::PathBuf> {
    if let Ok(s) = std::env::var("RUSTCLAW_WORKSPACE") {
        let p = Path::new(s.trim());
        if !p.as_os_str().is_empty() && p.join(CONFIG_REL).exists() {
            return Some(p.to_path_buf());
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(CONFIG_REL).exists() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
        if dir.as_os_str().is_empty() {
            return None;
        }
    }
}

/// 从工作区 configs/config.toml 读取 [database].sqlite_path，缺省为 data/rustclaw.db（相对工作区根）
fn sqlite_path_from_config() -> Option<std::path::PathBuf> {
    let root = find_workspace_root()?;
    let config_path = root.join(CONFIG_REL);
    let raw = std::fs::read_to_string(&config_path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    let path_str = value.get("database")?.get("sqlite_path")?.as_str()?.trim();
    if path_str.is_empty() {
        return Some(root.join(DEFAULT_SQLITE_PATH));
    }
    let p = Path::new(path_str);
    if p.is_absolute() {
        Some(p.to_path_buf())
    } else {
        Some(root.join(p))
    }
}

/// 从数据库读取一个已启用的 admin key；无则返回 None。
fn admin_key_from_db() -> Option<String> {
    let db_path = sqlite_path_from_config()
        .or_else(|| find_workspace_root().map(|root| root.join(DEFAULT_SQLITE_PATH)))?;
    let db = rusqlite::Connection::open(&db_path).ok()?;
    let mut stmt = db
        .prepare("SELECT user_key FROM auth_keys WHERE role = 'admin' AND enabled = 1 LIMIT 1")
        .ok()?;
    let mut rows = stmt.query([]).ok()?;
    let row = rows.next().ok()??;
    let user_key: String = row.get(0).ok()?;
    if user_key.trim().is_empty() {
        return None;
    }
    Some(user_key)
}

/// 解析 admin key：环境变量 RUSTCLAW_ADMIN_KEY > 数据库 auth_keys（role=admin, enabled=1）
fn default_admin_key() -> Option<String> {
    if let Ok(s) = std::env::var("RUSTCLAW_ADMIN_KEY") {
        let t = s.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    admin_key_from_db()
}

/// 需要 key 时的统一错误说明（含工作区/数据库提示）
fn key_required_error() -> anyhow::Error {
    let hint = if find_workspace_root().is_none() {
        "未找到工作区（当前目录及上级无 configs/config.toml）。请在项目根目录执行，或设置 RUSTCLAW_WORKSPACE。"
    } else {
        "数据库 auth_keys 中无启用的 admin key。请先启动 clawd 生成初始 key。"
    };
    anyhow::anyhow!("需要 key：用 -k/--key，或设置 RUSTCLAW_ADMIN_KEY。{}", hint)
}

#[derive(Parser)]
#[command(name = "clawcli")]
#[command(about = "Terminal CLI to interact with clawd")]
#[command(subcommand_required = false)]
struct Cli {
    #[arg(long, default_value = DEFAULT_BASE_URL)]
    base_url: String,

    /// Admin key（不传则用 RUSTCLAW_ADMIN_KEY 或从数据库 auth_keys 读取）
    #[arg(short, long)]
    key: Option<String>,

    #[command(subcommand)]
    cmd: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// 对话式交互（默认）：输入一句发送给 clawd，等待结果后继续输入
    Chat,

    /// GET /v1/health
    Health,

    /// POST /v1/tasks — 提交 ask 任务，payload 为 {"text": "..."}
    Submit {
        #[arg(short, long)]
        text: String,
    },

    /// POST /v1/tasks — 以 user_followup 方式续接一个已有任务；payload fields: resume_task_id, resume_trigger=user_followup
    Resume {
        /// resume_task_id：要续接的已有任务 ID
        task_id: String,
        /// resume_directive：本次续接提交的文本内容
        #[arg(short, long)]
        text: String,
    },

    /// GET /v1/tasks/:task_id
    Get {
        task_id: String,
        #[arg(long)]
        events: bool,
        #[arg(long = "event-type")]
        event_types: Vec<String>,
        #[arg(long)]
        events_output: Option<PathBuf>,
    },

    /// POST /v1/tasks/active — 列出活跃任务
    Active {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        exclude_task_id: Option<String>,
    },

    /// POST /v1/tasks/cancel — 取消任务
    Cancel {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        exclude_task_id: Option<String>,
    },

    /// POST /v1/admin/reload-skills — 重载技能视图（需 admin key）
    ReloadSkills,
}

fn base_v1(base_url: &str) -> String {
    let u = base_url.trim_end_matches('/');
    format!("{u}{V1}")
}

fn make_client() -> Result<Client> {
    Ok(Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}

fn run_health(base_url: &str, key: Option<&str>) -> Result<()> {
    let url = format!("{}/health", base_v1(base_url));
    let mut req = Client::new().get(&url);
    if let Some(k) = key {
        req = req.header("x-rustclaw-key", k);
    }
    let resp = req.send().context("request failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse health response")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("health returned {}", status);
    }
    Ok(())
}

/// 提交 ask 任务，返回 task_id（从 data.task_id 取）
fn submit_ask(base_url: &str, key: &str, text: &str) -> Result<String> {
    submit_ask_with_payload(
        base_url,
        key,
        json!({
            "text": text
        }),
    )
}

fn submit_resume_ask(base_url: &str, key: &str, task_id: &str, text: &str) -> Result<String> {
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

fn submit_ask_with_payload(
    base_url: &str,
    key: &str,
    payload: serde_json::Value,
) -> Result<String> {
    let url = format!("{}/tasks", base_v1(base_url));
    let body = json!({
        "user_key": key,
        "channel": "ui",
        "kind": "ask",
        "payload": payload
    });
    let resp = make_client()?
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

fn run_submit(base_url: &str, key: &str, text: &str) -> Result<()> {
    let task_id = submit_ask(base_url, key, text)?;
    println!("task_id: {}", task_id);
    Ok(())
}

fn run_resume(base_url: &str, key: &str, resume_task_id: &str, text: &str) -> Result<()> {
    let task_id = submit_resume_ask(base_url, key, resume_task_id, text)?;
    println!("task_id: {}", task_id);
    println!("resume_task_id: {}", resume_task_id);
    Ok(())
}

struct TaskStatusView {
    status: String,
    result_text: Option<String>,
    error_text: Option<String>,
    events: Vec<TaskEventLine>,
}

struct TaskEventLine {
    event_type: String,
    line: String,
}

/// 拉取任务详情。
/// result_text：优先用 result_json.messages（多条拼成一段，不丢），无则用 result_json.text。
fn get_task_status(base_url: &str, key: &str, task_id: &str) -> Result<TaskStatusView> {
    let url = format!("{}/tasks/{}", base_v1(base_url), task_id);
    let resp = make_client()?
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
    let result_text = result_json
        .and_then(|v| v.get("messages").and_then(|m| m.as_array()))
        .and_then(|arr| {
            let lines: Vec<String> = arr
                .iter()
                .filter_map(|m| {
                    m.get("text")
                        .and_then(|t| t.as_str())
                        .map(String::from)
                        .or_else(|| m.as_str().map(String::from))
                })
                .collect();
            if lines.is_empty() {
                None
            } else {
                Some(lines.join("\n\n"))
            }
        })
        .or_else(|| {
            result_json.and_then(|v| v.get("text").and_then(|t| t.as_str()).map(String::from))
        });
    let error_text = data
        .get("error_text")
        .and_then(|e| e.as_str())
        .map(String::from);
    let events = task_event_lines(data);
    Ok(TaskStatusView {
        status,
        result_text,
        error_text,
        events,
    })
}

fn task_event_lines(data: &serde_json::Value) -> Vec<TaskEventLine> {
    data.pointer("/result_json/task_journal/trace/event_stream")
        .and_then(serde_json::Value::as_array)
        .map(|events| events.iter().filter_map(task_event_line).collect())
        .unwrap_or_default()
}

fn task_event_line(event: &serde_json::Value) -> Option<TaskEventLine> {
    let mut parts = Vec::new();
    push_scalar_token(&mut parts, "seq", event.get("seq"));
    push_scalar_token(&mut parts, "type", event.get("event_type"));
    let event_type = event
        .get("event_type")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown")
        .to_string();
    let payload = event.get("payload");
    for key in [
        "status",
        "state",
        "error_kind",
        "failure_attribution",
        "owner_layer",
        "stage",
        "decision",
        "reason_code",
        "role",
        "execution_mode",
        "write_enabled",
        "external_publish_enabled",
        "failure_isolated",
        "child_run_id",
        "objective_present",
        "objective_char_count",
        "context_ref_count",
        "allowed_capability_count",
        "skill",
        "tool_or_skill",
        "step_id",
        "action_kind",
        "action_ref",
        "requested_capability",
        "requested_action_ref",
        "resolved_tool_or_skill",
        "resolved_capability",
        "resolution_source",
        "output_evidence_count",
        "artifact_ref_count",
        "prompt_label",
        "llm_call_count",
        "elapsed_ms",
        "provider_attempt_count",
        "provider_retry_count",
        "provider_retryable_error_count",
        "provider_final_error_count",
        "prompt_truncation_count",
        "prompt_bytes_before_max",
        "prompt_bytes_budget_min",
        "prompt_bytes_after_max",
        "prompt_truncated_bytes_total",
        "checkpoint_id",
        "poll_ref",
        "final_status",
        "final_stop_signal",
    ] {
        push_scalar_token(&mut parts, key, payload.and_then(|value| value.get(key)));
    }
    push_scalar_token(
        &mut parts,
        "child_trace_merge_status",
        payload.and_then(|value| value.pointer("/child_run_summary/trace_merge_status")),
    );
    push_scalar_token(
        &mut parts,
        "child_result_status",
        payload.and_then(|value| value.pointer("/child_run_summary/result_status")),
    );
    push_scalar_token(
        &mut parts,
        "child_request_state",
        payload.and_then(|value| value.pointer("/child_request/state")),
    );
    push_scalar_token(
        &mut parts,
        "scheduler_status",
        payload.and_then(|value| value.pointer("/scheduler/status")),
    );
    push_scalar_token(
        &mut parts,
        "scheduler_reason_code",
        payload.and_then(|value| value.pointer("/scheduler/reason_code")),
    );
    push_scalar_token(
        &mut parts,
        "merge_strategy",
        payload.and_then(|value| value.pointer("/merge_contract/strategy")),
    );
    push_scalar_token(
        &mut parts,
        "merge_status",
        payload.and_then(|value| value.pointer("/merge_contract/child_trace_merge_status")),
    );
    (!parts.is_empty()).then(|| TaskEventLine {
        event_type,
        line: parts.join(" "),
    })
}

fn push_scalar_token(parts: &mut Vec<String>, key: &str, value: Option<&serde_json::Value>) {
    let Some(value) = value else {
        return;
    };
    let token = match value {
        serde_json::Value::String(value) => value.trim().to_string(),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_) => {
            String::new()
        }
    };
    if token.is_empty() {
        return;
    }
    parts.push(format!("{key}={token}"));
}

fn run_get(
    base_url: &str,
    key: &str,
    task_id: &str,
    events: bool,
    event_types: &[String],
    events_output: Option<&PathBuf>,
) -> Result<()> {
    let task = get_task_status(base_url, key, task_id)?;
    println!("status: {}", task.status);
    if let Some(t) = task.result_text {
        println!("{}", t);
    }
    if let Some(e) = task.error_text {
        eprintln!("error: {}", e);
    }
    let requested_event_types = event_types
        .iter()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    let filtered_events = task
        .events
        .iter()
        .filter(|event| {
            requested_event_types.is_empty()
                || requested_event_types
                    .iter()
                    .any(|requested| requested == &event.event_type.to_ascii_lowercase())
        })
        .map(|event| format!("event: {}", event.line))
        .collect::<Vec<_>>();
    if events || !requested_event_types.is_empty() || events_output.is_some() {
        for line in &filtered_events {
            println!("{line}");
        }
    }
    if let Some(path) = events_output {
        let mut content = filtered_events.join("\n");
        if !content.is_empty() {
            content.push('\n');
        }
        std::fs::write(path, content)
            .with_context(|| format!("write events output failed: path={}", path.display()))?;
    }
    Ok(())
}

fn run_active(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
) -> Result<()> {
    let url = format!("{}/tasks/active", base_v1(base_url));
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
        "exclude_task_id": exclude_task_id,
    });
    let resp = make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("list active tasks failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse active response")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("active returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}

fn run_cancel(
    base_url: &str,
    key: &str,
    user_id: i64,
    chat_id: i64,
    exclude_task_id: Option<String>,
) -> Result<()> {
    let url = format!("{}/tasks/cancel", base_v1(base_url));
    let payload = json!({
        "user_id": user_id,
        "chat_id": chat_id,
        "exclude_task_id": exclude_task_id,
    });
    let resp = make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .header("content-type", "application/json")
        .json(&payload)
        .send()
        .context("cancel tasks failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse cancel response")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("cancel returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}

fn run_reload_skills(base_url: &str, key: &str) -> Result<()> {
    let url = format!("{}/admin/reload-skills", base_v1(base_url));
    let resp = make_client()?
        .post(&url)
        .header("x-rustclaw-key", key)
        .send()
        .context("reload-skills failed")?;
    let status = resp.status();
    let body: serde_json::Value = resp.json().context("parse reload-skills response")?;
    println!(
        "{}",
        serde_json::to_string_pretty(&body).unwrap_or_default()
    );
    if !status.is_success() {
        anyhow::bail!("reload-skills returned {}: {:?}", status, body.get("error"));
    }
    Ok(())
}

const POLL_INTERVAL_MS: u64 = 800;
const TERMINAL_STATUS: &[&str] = &["succeeded", "failed", "canceled"];

fn run_chat(base_url: &str, key: &str) -> Result<()> {
    println!("clawcli chat mode (type a message, empty line or 'exit' to quit)");
    println!("---");
    let mut rl = rustyline::DefaultEditor::new().context("rustyline init (is stdin a TTY?)")?;
    loop {
        let line = match rl.readline("> ") {
            Ok(s) => s,
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(rustyline::error::ReadlineError::Interrupted) => break,
            Err(e) => {
                eprintln!("readline: {}", e);
                break;
            }
        };
        let text = line.trim();
        if text.is_empty() {
            break;
        }
        if text.eq_ignore_ascii_case("exit") || text.eq_ignore_ascii_case("quit") {
            break;
        }
        let task_id = match submit_ask(base_url, key, text) {
            Ok(id) => id,
            Err(e) => {
                eprintln!("提交失败: {}", e);
                continue;
            }
        };
        let mut wait_tick = 0usize;
        loop {
            let dots = match wait_tick % 4 {
                0 => ".",
                1 => "..",
                2 => "...",
                _ => "",
            };
            print!("\rWaiting for clawd reply{dots:<3}");
            std::io::Write::flush(&mut std::io::stdout()).context("flush stdout")?;
            wait_tick += 1;
            std::thread::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS));
            let task = match get_task_status(base_url, key, &task_id) {
                Ok(t) => t,
                Err(e) => {
                    print!("\r{:<48}\r", "");
                    std::io::Write::flush(&mut std::io::stdout()).context("flush stdout")?;
                    eprintln!("查询任务失败: {}", e);
                    break;
                }
            };
            if TERMINAL_STATUS.contains(&task.status.as_str()) {
                print!("\r{:<48}\r", "");
                std::io::Write::flush(&mut std::io::stdout()).context("flush stdout")?;
                if let Some(ref t) = task.result_text {
                    println!("{}\n", t);
                }
                if let Some(ref e) = task.error_text {
                    eprintln!("错误: {}\n", e);
                }
                if task.status == "failed"
                    && task.result_text.is_none()
                    && task.error_text.is_none()
                {
                    println!("(任务失败，无详情)\n");
                }
                break;
            }
        }
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let base_url = std::env::var("RUSTCLAW_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cli.base_url.clone());
    let base_url = base_url.trim_end_matches('/');
    let key: Option<String> = cli.key.or_else(default_admin_key);
    let cmd = cli.cmd.unwrap_or(Command::Chat);

    match &cmd {
        Command::Chat => {
            let k = key.as_deref().ok_or_else(key_required_error)?;
            run_chat(base_url, k)
        }
        Command::Health => run_health(base_url, key.as_deref()),
        Command::Submit { text } => {
            let k = key.as_deref().ok_or_else(key_required_error)?;
            run_submit(base_url, k, text)
        }
        Command::Resume { task_id, text } => {
            let k = key.as_deref().ok_or_else(key_required_error)?;
            run_resume(base_url, k, task_id, text)
        }
        Command::Get {
            task_id,
            events,
            event_types,
            events_output,
        } => {
            let k = key.as_deref().ok_or_else(key_required_error)?;
            run_get(
                base_url,
                k,
                task_id,
                *events,
                event_types,
                events_output.as_ref(),
            )
        }
        Command::Active {
            user_id,
            chat_id,
            exclude_task_id,
        } => {
            let k = key.as_deref().ok_or_else(key_required_error)?;
            run_active(base_url, k, *user_id, *chat_id, exclude_task_id.clone())
        }
        Command::Cancel {
            user_id,
            chat_id,
            exclude_task_id,
        } => {
            let k = key.as_deref().ok_or_else(key_required_error)?;
            run_cancel(base_url, k, *user_id, *chat_id, exclude_task_id.clone())
        }
        Command::ReloadSkills => {
            let k = key.as_deref().ok_or_else(key_required_error)?;
            run_reload_skills(base_url, k)
        }
    }
}
