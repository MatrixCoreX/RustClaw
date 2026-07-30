use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use skill_sdk::{
    digest_file, ArtifactReceipt, ArtifactSpill, BoundedResult, ExpectedPathKind, SkillPathPolicy,
};

const SKILL_NAME: &str = "install_module";

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    context: Option<Value>,
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
    match install_modules(req.args, req.context.as_ref()) {
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

fn error_extra(error_code: &str) -> Value {
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": error_code,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_code),
        "retryable": false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InstallScope {
    Project,
    ToolCache,
}

impl InstallScope {
    fn token(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::ToolCache => "tool_cache",
        }
    }
}

#[derive(Debug, Clone)]
struct InstallCommand {
    argv: Vec<String>,
    cwd: PathBuf,
    env: BTreeMap<String, String>,
    target_files: Vec<PathBuf>,
    create_dirs: Vec<PathBuf>,
}

fn install_modules(args: Value, context: Option<&Value>) -> Result<(String, Value), String> {
    let action = extract_action(&args)?;
    let ecosystem = extract_ecosystem(&args);
    let modules = extract_modules(&args)?;
    if modules.is_empty() {
        return Err("no modules to install".to_string());
    }

    for module in &modules {
        if !is_safe_module_name(ecosystem, module) {
            return Err(format!("invalid module name: {module}"));
        }
    }

    let version = extract_version(&args);
    let dry_run = action == "preview_install" || extract_dry_run(&args);
    let workspace_root = request_workspace_root(context)?;
    let policy =
        SkillPathPolicy::new(&workspace_root, context).map_err(|error| error.to_string())?;
    let scope = extract_scope(&args, ecosystem, &workspace_root);
    let commands = build_install_commands(
        &args,
        ecosystem,
        scope,
        &modules,
        version.as_deref(),
        &workspace_root,
        &policy,
    )?;
    let installer_available = commands
        .first()
        .and_then(|command| command.argv.first())
        .is_some_and(|program| program_available(program));
    if !dry_run && !installer_available {
        return Err(installer_unavailable_message(ecosystem));
    }

    let command_strings = commands.iter().map(render_command).collect::<Vec<_>>();
    let target_files = commands
        .iter()
        .flat_map(|command| command.target_files.iter())
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>();

    if dry_run {
        let text = module_install_summary(
            action,
            ecosystem,
            scope,
            &modules,
            version.as_deref(),
            true,
            installer_available,
            &command_strings,
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
                "scope": scope.token(),
                "dry_run": true,
                "would_write": false,
                "confirmation_required_for_install": true,
                "installer_available": installer_available,
                "commands": command_strings,
                "command_argv": commands.iter().map(|command| &command.argv).collect::<Vec<_>>(),
                "working_directories": commands.iter().map(|command| command.cwd.display().to_string()).collect::<Vec<_>>(),
                "target_files": target_files,
                "output": text,
            }),
        ));
    }

    let mut installed = Vec::new();
    let mut output_results = Vec::new();
    for (module, command) in modules.iter().zip(commands.iter()) {
        prepare_command_targets(command)?;
        let out = run_install_command(command)?;

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
        let full_output = command_output(&out.stdout, &out.stderr);
        let spill = ArtifactSpill::new(
            workspace_state_root(&workspace_root).join("artifacts/skill-output"),
            SKILL_NAME,
        )
        .map_err(|error| error.to_string())?;
        output_results.push(
            BoundedResult::text(&full_output, 12_000, Some(&spill), "module-install-output")
                .map_err(|error| error.to_string())?,
        );
        installed.push(render_installed_name(&module, version.as_deref()));
    }

    let artifacts = receipt_artifacts(&commands)?;

    let text = module_install_summary(
        action,
        ecosystem,
        scope,
        &installed,
        version.as_deref(),
        false,
        installer_available,
        &command_strings,
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
            "scope": scope.token(),
            "dry_run": false,
            "would_write": true,
            "installer_available": installer_available,
            "commands": command_strings,
            "command_argv": commands.iter().map(|command| &command.argv).collect::<Vec<_>>(),
            "working_directories": commands.iter().map(|command| command.cwd.display().to_string()).collect::<Vec<_>>(),
            "target_files": target_files,
            "operation_receipt": {
                "schema_version": 1,
                "operation": "dependency_install",
                "scope": scope.token(),
                "platform": std::env::consts::OS,
                "artifacts": artifacts,
            },
            "output_results": output_results,
            "output": text,
        }),
    ))
}

fn request_workspace_root(context: Option<&Value>) -> Result<PathBuf, String> {
    let root = context
        .and_then(|value| value.get("workspace_root"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("WORKSPACE_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    fs::canonicalize(&root).map_err(|error| format!("workspace root unavailable: {error}"))
}

fn extract_scope(args: &Value, ecosystem: &str, workspace_root: &Path) -> InstallScope {
    match args.get("scope").and_then(Value::as_str) {
        Some("project") => InstallScope::Project,
        Some("tool_cache") => InstallScope::ToolCache,
        _ if project_marker_exists(ecosystem, workspace_root) => InstallScope::Project,
        _ => InstallScope::ToolCache,
    }
}

fn project_marker_exists(ecosystem: &str, root: &Path) -> bool {
    project_markers(ecosystem)
        .iter()
        .any(|marker| root.join(marker).exists())
}

fn project_markers(ecosystem: &str) -> &'static [&'static str] {
    match ecosystem {
        "python" => &[
            "pyproject.toml",
            "requirements.txt",
            "Pipfile",
            "uv.lock",
            "poetry.lock",
        ],
        "node" => &["package.json"],
        "rust" => &["Cargo.toml"],
        "go" => &["go.mod"],
        _ => &[],
    }
}

#[allow(clippy::too_many_arguments)]
fn build_install_commands(
    args: &Value,
    ecosystem: &str,
    scope: InstallScope,
    modules: &[String],
    version: Option<&str>,
    workspace_root: &Path,
    policy: &SkillPathPolicy,
) -> Result<Vec<InstallCommand>, String> {
    let project_input = args
        .get("project_path")
        .and_then(Value::as_str)
        .unwrap_or(".");
    let project_root = policy
        .resolve_existing(project_input, ExpectedPathKind::Directory)
        .map_err(|error| error.to_string())?;
    if scope == InstallScope::Project && !project_marker_exists(ecosystem, &project_root) {
        return Err(format!(
            "project manifest not found for ecosystem={ecosystem}; use scope=tool_cache for a standalone tool"
        ));
    }

    modules
        .iter()
        .map(|module| match scope {
            InstallScope::Project => {
                project_install_command(ecosystem, module, version, &project_root)
            }
            InstallScope::ToolCache => {
                tool_cache_install_command(ecosystem, module, version, workspace_root)
            }
        })
        .collect()
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
        .filter(|s| !s.is_empty() && is_safe_version(s))
}

fn extract_dry_run(args: &Value) -> bool {
    args.as_object()
        .and_then(|obj| obj.get("dry_run"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn program_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
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

fn project_install_command(
    ecosystem: &str,
    module: &str,
    version: Option<&str>,
    project_root: &Path,
) -> Result<InstallCommand, String> {
    let (argv, target_files, create_dirs) = match ecosystem {
        "python" if project_root.join("uv.lock").exists() => (
            vec![
                "uv".to_string(),
                "add".to_string(),
                render_module_for_python(module, version),
            ],
            vec![
                project_root.join("pyproject.toml"),
                project_root.join("uv.lock"),
            ],
            Vec::new(),
        ),
        "python" if project_root.join("poetry.lock").exists() => (
            vec![
                "poetry".to_string(),
                "add".to_string(),
                render_module_for_python(module, version),
            ],
            vec![
                project_root.join("pyproject.toml"),
                project_root.join("poetry.lock"),
            ],
            Vec::new(),
        ),
        "python" => {
            let target = workspace_state_root(project_root).join("dependencies/python");
            (
                vec![
                    "python3".to_string(),
                    "-m".to_string(),
                    "pip".to_string(),
                    "install".to_string(),
                    "--target".to_string(),
                    target.display().to_string(),
                    render_module_for_python(module, version),
                ],
                vec![target.clone()],
                vec![target],
            )
        }
        "node" => (
            vec![
                "npm".to_string(),
                "install".to_string(),
                "--save".to_string(),
                render_module_for_node(module, version),
            ],
            vec![
                project_root.join("package.json"),
                project_root.join("package-lock.json"),
            ],
            Vec::new(),
        ),
        "rust" => {
            let mut argv = vec!["cargo".to_string(), "add".to_string(), module.to_string()];
            if let Some(version) = version {
                argv.extend(["--vers".to_string(), version.to_string()]);
            }
            (
                argv,
                vec![
                    project_root.join("Cargo.toml"),
                    project_root.join("Cargo.lock"),
                ],
                Vec::new(),
            )
        }
        "go" => (
            vec![
                "go".to_string(),
                "get".to_string(),
                render_module_for_go(module, version),
            ],
            vec![project_root.join("go.mod"), project_root.join("go.sum")],
            Vec::new(),
        ),
        _ => return Err(format!("unsupported ecosystem: {ecosystem}")),
    };
    Ok(InstallCommand {
        argv,
        cwd: project_root.to_path_buf(),
        env: BTreeMap::new(),
        target_files,
        create_dirs,
    })
}

fn tool_cache_install_command(
    ecosystem: &str,
    module: &str,
    version: Option<&str>,
    workspace_root: &Path,
) -> Result<InstallCommand, String> {
    let cache_root = workspace_root
        .join("data/tool-cache/modules")
        .join(ecosystem)
        .join(cache_path_token(module))
        .join(cache_path_token(version.unwrap_or("latest")));
    let mut env = BTreeMap::new();
    let argv = match ecosystem {
        "python" => vec![
            "python3".to_string(),
            "-m".to_string(),
            "pip".to_string(),
            "install".to_string(),
            "--target".to_string(),
            cache_root.display().to_string(),
            render_module_for_python(module, version),
        ],
        "node" => vec![
            "npm".to_string(),
            "install".to_string(),
            "--prefix".to_string(),
            cache_root.display().to_string(),
            render_module_for_node(module, version),
        ],
        "rust" => {
            let mut argv = vec![
                "cargo".to_string(),
                "install".to_string(),
                "--root".to_string(),
                cache_root.display().to_string(),
                module.to_string(),
            ];
            if let Some(version) = version {
                argv.extend(["--version".to_string(), version.to_string()]);
            }
            argv
        }
        "go" => {
            let bin = cache_root.join("bin");
            env.insert("GOBIN".to_string(), bin.display().to_string());
            vec![
                "go".to_string(),
                "install".to_string(),
                render_module_for_go(module, version),
            ]
        }
        _ => return Err(format!("unsupported ecosystem: {ecosystem}")),
    };
    Ok(InstallCommand {
        argv,
        cwd: workspace_root.to_path_buf(),
        env,
        target_files: vec![cache_root.clone()],
        create_dirs: vec![cache_root],
    })
}

fn prepare_command_targets(command: &InstallCommand) -> Result<(), String> {
    for directory in &command.create_dirs {
        fs::create_dir_all(directory).map_err(|error| {
            format!(
                "prepare installation target {} failed: {error}",
                directory.display()
            )
        })?;
    }
    Ok(())
}

fn run_install_command(command: &InstallCommand) -> Result<std::process::Output, String> {
    let (bin, rest) = command
        .argv
        .split_first()
        .ok_or_else(|| "empty install command".to_string())?;

    let mut process = Command::new(bin);
    process
        .args(rest)
        .current_dir(&command.cwd)
        .envs(&command.env)
        .output()
        .map_err(|err| format!("run installer failed: {err}"))
}

fn render_command(command: &InstallCommand) -> String {
    let env = command
        .env
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    env.into_iter()
        .chain(command.argv.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

fn command_output(stdout: &[u8], stderr: &[u8]) -> String {
    let mut output = String::from_utf8_lossy(stdout).into_owned();
    if !stderr.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&String::from_utf8_lossy(stderr));
    }
    output
}

fn receipt_artifacts(commands: &[InstallCommand]) -> Result<Vec<ArtifactReceipt>, String> {
    let mut artifacts = Vec::new();
    for path in commands.iter().flat_map(|command| &command.target_files) {
        if !path.is_file() {
            continue;
        }
        let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
        artifacts.push(ArtifactReceipt {
            path: path.display().to_string(),
            sha256: digest_file(path).map_err(|error| error.to_string())?,
            size_bytes: metadata.len(),
            executable: false,
        });
    }
    Ok(artifacts)
}

fn module_install_summary(
    action: &str,
    ecosystem: &str,
    scope: InstallScope,
    modules: &[String],
    version: Option<&str>,
    dry_run: bool,
    installer_available: bool,
    commands: &[String],
) -> String {
    let mut fields = vec![
        "skill=install_module".to_string(),
        format!("action={action}"),
        format!("ecosystem={ecosystem}"),
        format!("scope={}", scope.token()),
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
        fields.push(format!("command_{idx}={command}"));
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

fn is_safe_module_name(ecosystem: &str, name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && name.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, '_' | '-' | '.')
                || (ecosystem == "node" && matches!(c, '@' | '/'))
                || (ecosystem == "go" && c == '/')
        })
        && !name.contains("..")
        && !name.starts_with('/')
        && !name.ends_with('/')
}

fn is_safe_version(version: &str) -> bool {
    version.len() <= 128
        && version.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '.' | '_' | '-' | '+' | '^' | '~' | '<' | '>' | '=' | '*'
                )
        })
}

fn cache_path_token(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn workspace_state_root(workspace_root: &Path) -> PathBuf {
    let state_dir =
        std::env::var("APP_WORKSPACE_STATE_DIR").unwrap_or_else(|_| ".agent-runtime".to_string());
    workspace_root.join(state_dir)
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
