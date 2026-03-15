use std::io::{self, BufRead, Write};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
        Ok(text) => Resp {
            request_id: req.request_id,
            status: "ok".to_string(),
            text,
            error_text: None,
        },
        Err(err) => Resp {
            request_id: req.request_id,
            status: "error".to_string(),
            text: String::new(),
            error_text: Some(err),
        },
    }
}

fn install_modules(args: Value) -> Result<String, String> {
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
    ensure_installer_available(ecosystem)?;

    let mut installed = Vec::new();
    for module in modules {
        let out = run_install_command(ecosystem, &module, version.as_deref())?;

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

    Ok(format!(
        "installed modules: ecosystem={ecosystem}; {}",
        installed.join(", ")
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

fn extract_version(args: &Value) -> Option<String> {
    let obj = args.as_object()?;
    obj.get("version")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && is_safe_module_name(s))
}

fn ensure_installer_available(ecosystem: &str) -> Result<(), String> {
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
    if out.status.success() {
        return Ok(());
    }
    match ecosystem {
        "python" => Err("python3 pip is not available. install python3-pip first".to_string()),
        "node" => Err("npm is not available. install nodejs/npm first".to_string()),
        "rust" => Err("cargo is not available. install Rust toolchain first".to_string()),
        "go" => Err("go is not available. install golang toolchain first".to_string()),
        _ => Err(format!("unsupported ecosystem: {ecosystem}")),
    }
}

fn run_install_command(
    ecosystem: &str,
    module: &str,
    version: Option<&str>,
) -> Result<std::process::Output, String> {
    let mut cmd = match ecosystem {
        "python" => {
            let mut c = Command::new("python3");
            c.arg("-m").arg("pip").arg("install").arg("--user");
            c.arg(render_module_for_python(module, version));
            c
        }
        "node" => {
            let mut c = Command::new("npm");
            c.arg("install").arg("-g");
            c.arg(render_module_for_node(module, version));
            c
        }
        "rust" => {
            let mut c = Command::new("cargo");
            c.arg("install").arg(module);
            if let Some(v) = version {
                c.arg("--version").arg(v);
            }
            c
        }
        "go" => {
            let mut c = Command::new("go");
            c.arg("install")
                .arg(render_module_for_go(module, version));
            c
        }
        _ => return Err(format!("unsupported ecosystem: {ecosystem}")),
    };

    cmd.output()
        .map_err(|err| format!("run installer failed: {err}"))
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
