use std::fs::{create_dir_all, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SKILL_NAME: &str = "package_manager";

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
                    extra: Some(error_extra("execution_failed")),
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: Some(error_extra("invalid_input")),
                error_text: Some(format!("invalid input: {err}")),
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
            let project_path = detect_path_arg(obj)
                .map(|path| resolve_path(&workspace_root(), &path))
                .transpose()?;
            if let Some(path) = project_path.as_deref() {
                if let Some(project) = detect_project_manager(path) {
                    let version = manager_version(project.manager);
                    let available = version.is_some();
                    let version_present = version.is_some();
                    let output = package_manager_detection_output(
                        project.manager,
                        available,
                        version_present,
                    );
                    return Ok((
                        output.clone(),
                        json!({
                            "action":"detect",
                            "manager":project.manager,
                            "manager_scope":"project",
                            "available":available,
                            "version_present":version_present,
                            "version":version,
                            "platform":std::env::consts::OS,
                            "path":path.display().to_string(),
                            "marker":project.marker,
                            "candidate_order":project_manager_markers(),
                            "system_candidate_order":package_manager_candidates(),
                            "system_manager":detect_manager().unwrap_or_else(|| "unknown".to_string()),
                            "output":output
                        }),
                    ));
                }
            }
            let mgr = detect_manager().unwrap_or_else(|| "unknown".to_string());
            let version = manager_version(&mgr);
            let available = mgr != "unknown";
            let version_present = version.is_some();
            let output = package_manager_detection_output(&mgr, available, version_present);
            Ok((
                output.clone(),
                json!({
                    "action":"detect",
                    "manager":mgr,
                    "manager_scope":"system",
                    "available":available,
                    "version_present":version_present,
                    "version":version,
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
            manage_packages(
                "smart_install",
                PackageOperation::Install,
                &manager,
                &packages,
                dry_run,
                use_sudo,
            )
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
            manage_packages(
                "install",
                PackageOperation::Install,
                &manager,
                &packages,
                dry_run,
                use_sudo,
            )
        }
        "uninstall" => {
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
            manage_packages(
                "uninstall",
                PackageOperation::Uninstall,
                &manager,
                &packages,
                dry_run,
                use_sudo,
            )
        }
        _ => Err("unsupported action; use detect|install|smart_install|uninstall".to_string()),
    }
}

fn detect_path_arg(obj: &serde_json::Map<String, Value>) -> Option<String> {
    ["path", "root", "project_path", "workspace"]
        .iter()
        .find_map(|key| {
            obj.get(*key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
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

fn package_manager_detection_output(
    manager: &str,
    available: bool,
    version_present: bool,
) -> String {
    format!("manager={manager} available={available} version_present={version_present}")
}

fn manager_version(manager: &str) -> Option<String> {
    if manager == "unknown" || !is_safe_token(manager) {
        return None;
    }
    let output = Command::new(manager).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    first_nonempty_version_line(&output.stdout)
        .or_else(|| first_nonempty_version_line(&output.stderr))
}

fn first_nonempty_version_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(160).collect::<String>())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProjectManagerDetection {
    manager: &'static str,
    marker: &'static str,
}

fn project_manager_markers() -> &'static [(&'static str, &'static str)] {
    &[
        ("pnpm", "pnpm-lock.yaml"),
        ("yarn", "yarn.lock"),
        ("npm", "package-lock.json"),
        ("bun", "bun.lockb"),
        ("bun", "bun.lock"),
        ("cargo", "Cargo.lock"),
        ("cargo", "Cargo.toml"),
        ("poetry", "poetry.lock"),
        ("python", "pyproject.toml"),
        ("npm", "package.json"),
    ]
}

fn detect_project_manager(path: &Path) -> Option<ProjectManagerDetection> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    project_manager_markers()
        .iter()
        .find(|(_manager, marker)| dir.join(marker).exists())
        .map(|(manager, marker)| ProjectManagerDetection {
            manager: *manager,
            marker: *marker,
        })
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn resolve_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let raw = Path::new(input);
    let mut normalized = PathBuf::new();
    for comp in raw.components() {
        match comp {
            Component::ParentDir => return Err("path with '..' is not allowed".to_string()),
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    if raw.is_absolute() {
        return Ok(normalized);
    }
    Ok(workspace_root.join(normalized))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PackageOperation {
    Install,
    Uninstall,
}

fn append_package_manager_argv(
    argv: &mut Vec<String>,
    manager: &str,
    operation: PackageOperation,
) -> Result<(), String> {
    match (manager, operation) {
        ("apt-get", PackageOperation::Install) => {
            argv.extend(["apt-get", "install", "-y"].into_iter().map(str::to_string));
        }
        ("apt-get", PackageOperation::Uninstall) => {
            argv.extend(["apt-get", "remove", "-y"].into_iter().map(str::to_string));
        }
        ("apt", PackageOperation::Install) => {
            argv.extend(["apt", "install", "-y"].into_iter().map(str::to_string));
        }
        ("apt", PackageOperation::Uninstall) => {
            argv.extend(["apt", "remove", "-y"].into_iter().map(str::to_string));
        }
        ("dnf", PackageOperation::Install) => {
            argv.extend(["dnf", "install", "-y"].into_iter().map(str::to_string));
        }
        ("dnf", PackageOperation::Uninstall) => {
            argv.extend(["dnf", "remove", "-y"].into_iter().map(str::to_string));
        }
        ("yum", PackageOperation::Install) => {
            argv.extend(["yum", "install", "-y"].into_iter().map(str::to_string));
        }
        ("yum", PackageOperation::Uninstall) => {
            argv.extend(["yum", "remove", "-y"].into_iter().map(str::to_string));
        }
        ("pacman", PackageOperation::Install) => {
            argv.extend(
                ["pacman", "-S", "--noconfirm"]
                    .into_iter()
                    .map(str::to_string),
            );
        }
        ("pacman", PackageOperation::Uninstall) => {
            argv.extend(
                ["pacman", "-R", "--noconfirm"]
                    .into_iter()
                    .map(str::to_string),
            );
        }
        ("apk", PackageOperation::Install) => {
            argv.extend(["apk", "add", "--no-cache"].into_iter().map(str::to_string));
        }
        ("apk", PackageOperation::Uninstall) => {
            argv.extend(["apk", "del"].into_iter().map(str::to_string));
        }
        ("zypper", PackageOperation::Install) => {
            argv.extend(
                ["zypper", "--non-interactive", "install"]
                    .into_iter()
                    .map(str::to_string),
            );
        }
        ("zypper", PackageOperation::Uninstall) => {
            argv.extend(
                ["zypper", "--non-interactive", "remove"]
                    .into_iter()
                    .map(str::to_string),
            );
        }
        ("brew", PackageOperation::Install) => {
            argv.extend(["brew", "install"].into_iter().map(str::to_string));
        }
        ("brew", PackageOperation::Uninstall) => {
            argv.extend(["brew", "uninstall"].into_iter().map(str::to_string));
        }
        _ => return Err(format!("unsupported manager: {manager}")),
    }
    Ok(())
}

fn package_manager_action_output(
    action: &str,
    manager: &str,
    packages: &[String],
    dry_run: bool,
    command: &str,
) -> String {
    let mut fields = vec![
        format!("action={action}"),
        format!("manager={manager}"),
        format!("dry_run={dry_run}"),
        format!("packages={}", packages.join(",")),
    ];
    if let Some(package) = single_package(packages) {
        fields.push(format!("package={package}"));
    }
    fields.push(format!("command={command}"));
    fields.join("\n")
}

fn single_package(packages: &[String]) -> Option<&str> {
    if packages.len() == 1 {
        packages.first().map(String::as_str)
    } else {
        None
    }
}

fn manage_packages(
    action: &str,
    operation: PackageOperation,
    manager: &str,
    packages: &[String],
    dry_run: bool,
    use_sudo: bool,
) -> Result<(String, Value), String> {
    let mut argv: Vec<String> = Vec::new();
    append_package_manager_argv(&mut argv, manager, operation)?;
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
            action,
            manager,
            packages,
            &full_cmd,
            None,
            Some("dry_run only"),
            None,
            dry_run,
            use_sudo,
        );
        let command = full_cmd.join(" ");
        let output = package_manager_action_output(action, manager, packages, dry_run, &command);
        return Ok((
            output.clone(),
            json!({
                "action": action,
                "manager": manager,
                "package": single_package(packages),
                "packages": packages,
                "dry_run": true,
                "use_sudo": use_sudo,
                "platform": std::env::consts::OS,
                "command": command,
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
        action,
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
        let command = full_cmd.join(" ");
        let summary = package_manager_action_output(action, manager, packages, dry_run, &command);
        let output = if text.trim().is_empty() {
            format!("{summary}\nexit_code={exit_code}")
        } else {
            format!("{summary}\nexit_code={exit_code}\n{text}")
        };
        Ok((
            output.clone(),
            json!({
                "action": action,
                "manager": manager,
                "package": single_package(packages),
                "packages": packages,
                "dry_run": false,
                "use_sudo": use_sudo,
                "platform": std::env::consts::OS,
                "exit_code": exit_code,
                "command": command,
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
    for key in ["packages", "modules"] {
        if let Some(arr) = obj.get(key).and_then(|v| v.as_array()) {
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
    }
    for key in ["package", "module"] {
        if let Some(p) = obj.get(key).and_then(|v| v.as_str()) {
            let t = p.trim();
            if !t.is_empty() {
                return Ok(vec![t.to_string()]);
            }
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
    action: &str,
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
        "action": action,
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

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
