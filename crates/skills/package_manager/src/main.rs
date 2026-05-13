use std::fs::{create_dir_all, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(req.args) {
                Ok((text, extra)) => Resp {
                    request_id: req.request_id,
                    status: "ok".to_string(),
                    text,
                    extra: Some(extra),
                    error_text: None,
                },
                Err(err) => Resp {
                    request_id: req.request_id,
                    status: "error".to_string(),
                    text: String::new(),
                    extra: None,
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: None,
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn execute(args: Value) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("detect");

    match action {
        "detect" => {
            let mgr = detect_manager().unwrap_or_else(|| "unknown".to_string());
            let output = format!("package_manager={mgr}");
            Ok((
                output.clone(),
                json!({
                    "action":"detect",
                    "manager":mgr,
                    "platform":std::env::consts::OS,
                    "candidate_order":package_manager_candidates(),
                    "output":output
                }),
            ))
        }
        "smart_install" => {
            let manager = detect_manager()
                .ok_or_else(|| "cannot detect package manager; install manually or set args.manager and use action=install".to_string())?;
            let packages = extract_safe_packages(obj)?;
            let dry_run = obj
                .get("dry_run")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let use_sudo = obj
                .get("use_sudo")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            install_packages("smart_install", &manager, &packages, dry_run, use_sudo)
        }
        "install" => {
            let manager = obj
                .get("manager")
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase())
                .or_else(detect_manager)
                .ok_or_else(|| "cannot detect package manager; set args.manager".to_string())?;

            let packages = extract_safe_packages(obj)?;

            let dry_run = obj.get("dry_run").and_then(|v| v.as_bool()).unwrap_or(true);
            let use_sudo = obj
                .get("use_sudo")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            install_packages("install", &manager, &packages, dry_run, use_sudo)
        }
        _ => Err("unsupported action; use detect|install|smart_install".to_string()),
    }
}

fn package_manager_candidates() -> &'static [&'static str] {
    match std::env::consts::OS {
        "macos" => &[
            "brew", "apt-get", "apt", "dnf", "yum", "pacman", "apk", "zypper",
        ],
        _ => &[
            "apt-get", "apt", "dnf", "yum", "pacman", "apk", "zypper", "brew",
        ],
    }
}

fn detect_manager() -> Option<String> {
    for m in package_manager_candidates() {
        let ok = Command::new("sh")
            .arg("-lc")
            .arg(format!("command -v {m} >/dev/null 2>&1"))
            .status()
            .ok()
            .is_some_and(|s| s.success());
        if ok {
            return Some(m.to_string());
        }
    }
    None
}

fn install_packages(
    action: &str,
    manager: &str,
    packages: &[String],
    dry_run: bool,
    use_sudo: bool,
) -> Result<(String, Value), String> {
    let mut argv: Vec<String> = Vec::new();
    match manager {
        "apt-get" => {
            argv.push("apt-get".to_string());
            argv.push("install".to_string());
            argv.push("-y".to_string());
        }
        "apt" => {
            argv.push("apt".to_string());
            argv.push("install".to_string());
            argv.push("-y".to_string());
        }
        "dnf" => {
            argv.push("dnf".to_string());
            argv.push("install".to_string());
            argv.push("-y".to_string());
        }
        "yum" => {
            argv.push("yum".to_string());
            argv.push("install".to_string());
            argv.push("-y".to_string());
        }
        "pacman" => {
            argv.push("pacman".to_string());
            argv.push("-S".to_string());
            argv.push("--noconfirm".to_string());
        }
        "apk" => {
            argv.push("apk".to_string());
            argv.push("add".to_string());
            argv.push("--no-cache".to_string());
        }
        "zypper" => {
            argv.push("zypper".to_string());
            argv.push("--non-interactive".to_string());
            argv.push("install".to_string());
        }
        "brew" => {
            argv.push("brew".to_string());
            argv.push("install".to_string());
        }
        _ => return Err(format!("unsupported manager: {manager}")),
    }
    argv.extend(packages.iter().cloned());

    let mut full_cmd = Vec::new();
    if use_sudo && !is_root() && manager != "brew" {
        full_cmd.push("sudo".to_string());
        full_cmd.push("-n".to_string());
    }
    full_cmd.extend(argv);

    if dry_run {
        append_install_log(
            "dry_run",
            manager,
            packages,
            &full_cmd,
            None,
            Some("dry_run only"),
            None,
            dry_run,
            use_sudo,
        );
        let output = format!("dry_run=1 command: {}", full_cmd.join(" "));
        return Ok((
            output.clone(),
            json!({
                "action": action,
                "manager": manager,
                "packages": packages,
                "dry_run": true,
                "use_sudo": use_sudo,
                "platform": std::env::consts::OS,
                "command": full_cmd.join(" "),
                "output": output,
            }),
        ));
    }

    let (bin, rest) = full_cmd
        .split_first()
        .ok_or_else(|| "empty command".to_string())?;
    let output = Command::new(bin)
        .args(rest)
        .output()
        .map_err(|err| format!("run package install failed: {err}"))?;

    let mut text = format_command_output(&output.stdout, &output.stderr);
    if text.len() > 12000 {
        text.truncate(12000);
    }
    let exit_code = output.status.code().unwrap_or(-1);
    append_install_log(
        if output.status.success() {
            "ok"
        } else {
            "failed"
        },
        manager,
        packages,
        &full_cmd,
        Some(exit_code),
        Some(&text),
        None,
        dry_run,
        use_sudo,
    );
    if output.status.success() {
        let output = format!("exit={exit_code}\n{text}");
        Ok((
            output.clone(),
            json!({
                "action": action,
                "manager": manager,
                "packages": packages,
                "dry_run": false,
                "use_sudo": use_sudo,
                "platform": std::env::consts::OS,
                "exit_code": exit_code,
                "command": full_cmd.join(" "),
                "output": output,
            }),
        ))
    } else {
        Err(format!("package install failed: exit={exit_code}\n{text}"))
    }
}

fn format_command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut text = String::new();
    text.push_str(&String::from_utf8_lossy(stdout));
    if !stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&String::from_utf8_lossy(stderr));
    }
    text
}

fn extract_packages(obj: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
    if let Some(arr) = obj.get("packages").and_then(|v| v.as_array()) {
        let mut out = Vec::new();
        for v in arr {
            if let Some(s) = v.as_str() {
                let t = s.trim();
                if !t.is_empty() {
                    out.push(t.to_string());
                }
            }
        }
        return Ok(out);
    }
    if let Some(p) = obj.get("package").and_then(|v| v.as_str()) {
        let t = p.trim();
        if !t.is_empty() {
            return Ok(vec![t.to_string()]);
        }
    }
    Err("args.package or args.packages is required".to_string())
}

fn extract_safe_packages(obj: &serde_json::Map<String, Value>) -> Result<Vec<String>, String> {
    let packages = extract_packages(obj)?;
    if packages.is_empty() {
        return Err("no packages provided".to_string());
    }
    for package in &packages {
        if !is_safe_token(package) {
            return Err(format!("invalid package name: {package}"));
        }
    }
    Ok(packages)
}

fn is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .is_some_and(|uid| uid.trim() == "0")
}

fn is_safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | ':'))
}

fn append_install_log(
    status: &str,
    manager: &str,
    packages: &[String],
    command: &[String],
    exit_code: Option<i32>,
    output: Option<&str>,
    error: Option<&str>,
    dry_run: bool,
    use_sudo: bool,
) {
    let root = workspace_root();
    let log_dir = root.join("logs");
    if let Err(err) = create_dir_all(&log_dir) {
        eprintln!("create install logs dir failed: {err}");
        return;
    }
    let file_path = log_dir.join("install_ops.log");
    let mut file = match OpenOptions::new()
        .create(true)
        .append(true)
        .open(&file_path)
    {
        Ok(f) => f,
        Err(err) => {
            eprintln!("open install log failed: {err}");
            return;
        }
    };

    let line = serde_json::json!({
        "ts": now_ts(),
        "status": status,
        "manager": manager,
        "packages": packages,
        "dry_run": dry_run,
        "use_sudo": use_sudo,
        "command": command.join(" "),
        "exit_code": exit_code,
        "output": output.map(truncate_for_log),
        "error": error.map(truncate_for_log),
    })
    .to_string();

    if let Err(err) = writeln!(file, "{line}") {
        eprintln!("write install log failed: {err}");
    }
}

fn truncate_for_log(s: &str) -> String {
    const MAX: usize = 8000;
    if s.len() <= MAX {
        return s.to_string();
    }
    let mut out = s[..MAX].to_string();
    out.push_str("...(truncated)");
    out
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
