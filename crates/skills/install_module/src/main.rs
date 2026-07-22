use std::io::{self, BufRead, Write};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SKILL_NAME: &str = "install_module";

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
            Ok(req) => handle(req),
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

fn handle(req: Req) -> Resp {
    match install_modules(req.args) {
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
    }
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

fn install_modules(args: Value) -> Result<(String, Value), String> {
    let action = extract_action(&args)?;
    let ecosystem = extract_ecosystem(&args);
    let modules = extract_modules(&args)?;
    if modules.is_empty() {
        return Err("no modules to install".to_string());
    }

    for module in &modules {
        if !is_safe_module_name(module) {
            return Err(format!("invalid module name: {module}"));
        }
    }

    let version = extract_version(&args);
    let dry_run = action == "preview_install" || extract_dry_run(&args);
    let installer_available = installer_available(ecosystem)?;
    if !dry_run && !installer_available {
        return Err(installer_unavailable_message(ecosystem));
    }

    let commands = modules
        .iter()
        .map(|module| install_command_args(ecosystem, module, version.as_deref()))
        .collect::<Result<Vec<_>, _>>()?;

    if dry_run {
        let text = module_install_summary(
            action,
            ecosystem,
            &modules,
            version.as_deref(),
            true,
            installer_available,
            &commands,
        );
        return Ok((
            text.clone(),
            json!({
                "action": action,
                "skill": "install_module",
                "ecosystem": ecosystem,
                "module": single_module(&modules),
                "modules": modules,
                "version": version,
                "dry_run": true,
                "installer_available": installer_available,
                "commands": commands.iter().map(|argv| argv.join(" ")).collect::<Vec<_>>(),
                "output": text,
            }),
        ));
    }

    let mut installed = Vec::new();
    for (module, command_args) in modules.iter().zip(commands.iter()) {
        let out = run_install_command(command_args)?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            let stdout = String::from_utf8_lossy(&out.stdout);
            let detail = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else {
                stdout.trim().to_string()
            };
            return Err(format!(
                "install module failed: ecosystem={ecosystem}, module={module}; {detail}"
            ));
        }
        installed.push(render_installed_name(&module, version.as_deref()));
    }

    let text = module_install_summary(
        action,
        ecosystem,
        &installed,
        version.as_deref(),
        false,
        installer_available,
        &commands,
    );
    Ok((
        text.clone(),
        json!({
            "action": action,
            "skill": "install_module",
            "ecosystem": ecosystem,
            "module": single_module(&installed),
            "modules": installed,
            "version": version,
            "dry_run": false,
            "installer_available": installer_available,
            "commands": commands.iter().map(|argv| argv.join(" ")).collect::<Vec<_>>(),
            "output": text,
        }),
    ))
}

fn extract_ecosystem(args: &Value) -> &'static str {
    let Some(obj) = args.as_object() else {
        return "python";
    };
    match obj
        .get("ecosystem")
        .and_then(|v| v.as_str())
        .unwrap_or("python")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "python" | "pip" => "python",
        "node" | "npm" => "node",
        "rust" | "cargo" => "rust",
        "go" | "golang" => "go",
        _ => "python",
    }
}

fn extract_action(args: &Value) -> Result<&'static str, String> {
    match args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("install")
        .trim()
    {
        "install" => Ok("install"),
        "preview_install" => Ok("preview_install"),
        action => Err(format!("unsupported action: {action}")),
    }
}

fn extract_version(args: &Value) -> Option<String> {
    let obj = args.as_object()?;
    obj.get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && is_safe_module_name(s))
}

fn extract_dry_run(args: &Value) -> bool {
    args.as_object()
        .and_then(|obj| obj.get("dry_run"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn installer_available(ecosystem: &str) -> Result<bool, String> {
    let mut cmd = match ecosystem {
        "python" => {
            let mut c = Command::new("python3");
            c.arg("-m").arg("pip").arg("--version");
            c
        }
        "node" => {
            let mut c = Command::new("npm");
            c.arg("--version");
            c
        }
        "rust" => {
            let mut c = Command::new("cargo");
            c.arg("--version");
            c
        }
        "go" => {
            let mut c = Command::new("go");
            c.arg("version");
            c
        }
        _ => return Err(format!("unsupported ecosystem: {ecosystem}")),
    };

    let out = cmd
        .output()
        .map_err(|err| format!("check installer failed: {err}"))?;
    Ok(out.status.success())
}

fn installer_unavailable_message(ecosystem: &str) -> String {
    match ecosystem {
        "python" => "python3 pip is not available. install python3-pip first".to_string(),
        "node" => "npm is not available. install nodejs/npm first".to_string(),
        "rust" => "cargo is not available. install Rust toolchain first".to_string(),
        "go" => "go is not available. install golang toolchain first".to_string(),
        _ => format!("unsupported ecosystem: {ecosystem}"),
    }
}

fn install_command_args(
    ecosystem: &str,
    module: &str,
    version: Option<&str>,
) -> Result<Vec<String>, String> {
    let args = match ecosystem {
        "python" => vec![
            "python3".to_string(),
            "-m".to_string(),
            "pip".to_string(),
            "install".to_string(),
            "--user".to_string(),
            render_module_for_python(module, version),
        ],
        "node" => vec![
            "npm".to_string(),
            "install".to_string(),
            "-g".to_string(),
            render_module_for_node(module, version),
        ],
        "rust" => {
            let mut args = vec![
                "cargo".to_string(),
                "install".to_string(),
                module.to_string(),
            ];
            if let Some(v) = version {
                args.push("--version".to_string());
                args.push(v.to_string());
            }
            args
        }
        "go" => vec![
            "go".to_string(),
            "install".to_string(),
            render_module_for_go(module, version),
        ],
        _ => return Err(format!("unsupported ecosystem: {ecosystem}")),
    };
    Ok(args)
}

fn run_install_command(command_args: &[String]) -> Result<std::process::Output, String> {
    let (bin, rest) = command_args
        .split_first()
        .ok_or_else(|| "empty install command".to_string())?;

    Command::new(bin)
        .args(rest)
        .output()
        .map_err(|err| format!("run installer failed: {err}"))
}

fn module_install_summary(
    action: &str,
    ecosystem: &str,
    modules: &[String],
    version: Option<&str>,
    dry_run: bool,
    installer_available: bool,
    commands: &[Vec<String>],
) -> String {
    let mut fields = vec![
        "skill=install_module".to_string(),
        format!("action={action}"),
        format!("ecosystem={ecosystem}"),
        format!("dry_run={dry_run}"),
        format!("installer_available={installer_available}"),
        format!("modules={}", modules.join(",")),
    ];
    if let Some(module) = single_module(modules) {
        fields.push(format!("module={module}"));
    }
    if let Some(version) = version {
        fields.push(format!("version={version}"));
    }
    for (idx, command) in commands.iter().enumerate() {
        fields.push(format!("command_{idx}={}", command.join(" ")));
    }
    fields.join("\n")
}

fn single_module(modules: &[String]) -> Option<&str> {
    if modules.len() == 1 {
        modules.first().map(String::as_str)
    } else {
        None
    }
}

fn render_module_for_python(module: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{module}=={v}"),
        None => module.to_string(),
    }
}

fn render_module_for_node(module: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{module}@{v}"),
        None => module.to_string(),
    }
}

fn render_module_for_go(module: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{module}@{v}"),
        None => format!("{module}@latest"),
    }
}

fn render_installed_name(module: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{module}@{v}"),
        None => module.to_string(),
    }
}

fn extract_modules(args: &Value) -> Result<Vec<String>, String> {
    if let Some(s) = args.as_str() {
        let one = s.trim();
        if one.is_empty() {
            return Ok(Vec::new());
        }
        return Ok(vec![one.to_string()]);
    }

    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object or string".to_string())?;

    if let Some(list) = obj.get("modules").and_then(|v| v.as_array()) {
        let mut out = Vec::new();
        for item in list {
            if let Some(s) = item.as_str() {
                let s = s.trim();
                if !s.is_empty() {
                    out.push(s.to_string());
                }
            }
        }
        return Ok(out);
    }

    for key in ["module", "package", "module_name"] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            let one = v.trim();
            if !one.is_empty() {
                return Ok(vec![one.to_string()]);
            }
        }
    }

    Ok(Vec::new())
}

fn is_safe_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
