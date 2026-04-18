//! Skill-runner: 接收 clawd 通过 stdin 投递的技能请求，分发到对应 skill
//! 二进制并把执行结果按行回写 stdout。
//!
//! P4.3 重写要点（vs 旧实现）：
//! - 全链路 tokio：`tokio::io::stdin/stdout` 替代阻塞的 `io::stdin().lock().lines()`，
//!   `tokio::process::Command` 替代 `std::process::Command`。
//! - 子进程超时改用 `tokio::time::timeout` + `Command::kill_on_drop(true)`，
//!   不再 `try_wait` + `thread::sleep(30ms)` busy-poll，CPU 占用归零。
//! - 用 `wait_with_output()` 一次性收 stdout/stderr，避免旧实现先 `try_wait`
//!   再 `wait_with_output` 时\"子进程写满 pipe buffer 阻塞\"的潜在死锁。
//! - 单进程串行处理多次请求语义保持不变（每条 stdin 行 = 一次请求）。

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

#[derive(Debug, Deserialize)]
struct SkillRequest {
    request_id: String,
    user_id: i64,
    chat_id: i64,
    user_key: Option<String>,
    skill_name: String,
    args: Value,
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct SkillResponse {
    request_id: String,
    status: String,
    text: String,
    buttons: Option<Value>,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChildSkillResponse {
    request_id: Option<String>,
    status: Option<String>,
    text: Option<String>,
    buttons: Option<Value>,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Result<SkillRequest, _> = serde_json::from_str(trimmed);
        let resp = match parsed {
            Ok(req) => execute_skill(req).await,
            Err(err) => SkillResponse {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                extra: None,
                error_text: Some(format!("invalid request: {err}")),
            },
        };

        let mut out = serde_json::to_string(&resp)?;
        out.push('\n');
        stdout.write_all(out.as_bytes()).await?;
        stdout.flush().await?;
    }

    Ok(())
}

async fn execute_skill(req: SkillRequest) -> SkillResponse {
    let timeout_secs: u64 = std::env::var("SKILL_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(30);

    let child_bin = match skill_binary_path(&req.skill_name) {
        Ok(path) => path,
        Err(err) => {
            return SkillResponse {
                request_id: req.request_id,
                status: "error".to_string(),
                text: String::new(),
                buttons: None,
                extra: None,
                error_text: Some(err),
            }
        }
    };

    let child_req = serde_json::json!({
        "request_id": req.request_id,
        "args": req.args,
        "context": req.context,
        "user_id": req.user_id,
        "chat_id": req.chat_id,
        "user_key": req.user_key,
    });

    match run_child_skill(
        &child_bin,
        &child_req.to_string(),
        Duration::from_secs(timeout_secs),
    )
    .await
    {
        Ok(out) => {
            let parsed: Result<ChildSkillResponse, _> = serde_json::from_str(&out);
            match parsed {
                Ok(v) => SkillResponse {
                    request_id: v.request_id.unwrap_or_else(|| "unknown".to_string()),
                    status: v.status.unwrap_or_else(|| "ok".to_string()),
                    text: v.text.unwrap_or_default(),
                    buttons: v.buttons,
                    extra: v.extra,
                    error_text: v.error_text,
                },
                Err(err) => SkillResponse {
                    request_id: "unknown".to_string(),
                    status: "error".to_string(),
                    text: String::new(),
                    buttons: None,
                    extra: None,
                    error_text: Some(format!("invalid child response: {err}; raw={out}")),
                },
            }
        }
        Err(err) => SkillResponse {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            buttons: None,
            extra: None,
            error_text: Some(err),
        },
    }
}

fn skill_binary_path(skill_name: &str) -> Result<String, String> {
    let bin_name = runner_bin_name(skill_name)?;

    let release_path = format!("target/release/{bin_name}");
    if Path::new(&release_path).exists() {
        return Ok(release_path);
    }

    Err(format!(
        "{skill_name} skill binary not found in target/release, build it first"
    ))
}

fn runner_bin_name(skill_name: &str) -> Result<String, String> {
    let raw = skill_name.trim();
    if raw.is_empty() {
        return Err("skill_name is empty".to_string());
    }
    if raw.contains('/') || raw.contains('\\') {
        return Err(format!(
            "invalid skill name `{raw}`: runner name must be a binary name, not a path"
        ));
    }

    let normalized = raw.replace('_', "-");
    if normalized.ends_with("-skill") {
        Ok(normalized)
    } else {
        Ok(format!("{normalized}-skill"))
    }
}

async fn run_child_skill(
    child_bin: &str,
    input_line: &str,
    timeout: Duration,
) -> Result<String, String> {
    let mut child = Command::new(child_bin)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|err| format!("spawn child failed: {err}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{input_line}\n").as_bytes())
            .await
            .map_err(|err| format!("write child stdin failed: {err}"))?;
        stdin
            .flush()
            .await
            .map_err(|err| format!("flush child stdin failed: {err}"))?;
    }

    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => return Err(format!("collect child output failed: {err}")),
        Err(_) => {
            return Err("child skill timeout".to_string());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "child exited with {:?}: {stderr}",
            output.status.code()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or_default().trim().to_string();
    if line.is_empty() {
        return Err("child stdout is empty".to_string());
    }

    Ok(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runner_bin_name_normalizes_underscores_and_appends_suffix() {
        assert_eq!(runner_bin_name("fs_search").unwrap(), "fs-search-skill");
        assert_eq!(runner_bin_name("rss_fetch").unwrap(), "rss-fetch-skill");
    }

    #[test]
    fn runner_bin_name_passes_through_when_already_suffixed() {
        assert_eq!(runner_bin_name("chat-skill").unwrap(), "chat-skill");
        assert_eq!(runner_bin_name("chat_skill").unwrap(), "chat-skill");
    }

    #[test]
    fn runner_bin_name_rejects_empty_or_path_like() {
        assert!(runner_bin_name("").is_err());
        assert!(runner_bin_name("   ").is_err());
        assert!(runner_bin_name("a/b").is_err());
        assert!(runner_bin_name("a\\b").is_err());
    }

    #[tokio::test]
    async fn run_child_skill_times_out_and_kills_child() {
        // tail -f /dev/null 永不退出，必然触发我们的 tokio::time::timeout 分支；
        // 顺带证明 kill_on_drop 真的会清理子进程（否则 cargo test 会挂在退出阶段）。
        let dir = std::env::temp_dir();
        let script = dir.join(format!("p43_timeout_{}.sh", std::process::id()));
        std::fs::write(&script, "#!/bin/sh\nexec tail -f /dev/null\n").unwrap();
        let mut perms = std::fs::metadata(&script).unwrap().permissions();
        use std::os::unix::fs::PermissionsExt;
        perms.set_mode(0o755);
        std::fs::set_permissions(&script, perms).unwrap();

        let result = run_child_skill(
            script.to_str().unwrap(),
            "ignored",
            Duration::from_millis(150),
        )
        .await;
        let _ = std::fs::remove_file(&script);
        assert!(
            matches!(result, Err(ref e) if e == "child skill timeout"),
            "expected timeout, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn run_child_skill_reports_nonzero_exit() {
        let result = run_child_skill("/bin/false", "ignored", Duration::from_secs(2)).await;
        assert!(matches!(result, Err(ref e) if e.starts_with("child exited with")));
    }

    #[tokio::test]
    async fn run_child_skill_returns_first_stdout_line() {
        let result = run_child_skill("/bin/cat", "hello-from-stdin", Duration::from_secs(2))
            .await
            .expect("cat should echo stdin");
        assert_eq!(result, "hello-from-stdin");
    }
}
