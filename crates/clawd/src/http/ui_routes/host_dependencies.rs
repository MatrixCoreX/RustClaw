const DEPENDENCY_VERSION_LIMIT: usize = 240;
const DEPENDENCY_INSTALL_LOG_LIMIT: usize = 12_000;
const DEPENDENCY_OPERATION_LIMIT: usize = 64;

#[derive(Debug, Clone, Serialize)]
struct HostDependenciesSnapshot {
    schema_version: u32,
    collected_at_ts: i64,
    platform: String,
    package_manager: Option<String>,
    summary: HostDependencySummary,
    dependencies: Vec<HostDependencyStatus>,
    operations: Vec<DependencyInstallOperation>,
}

#[derive(Debug, Clone, Serialize)]
struct HostDependencySummary {
    total: usize,
    installed: usize,
    missing_required: usize,
    missing_optional: usize,
}

#[derive(Debug, Clone, Serialize)]
struct HostDependencyStatus {
    id: String,
    category: String,
    required: bool,
    installed: bool,
    version: Option<String>,
    executable: Option<String>,
    package_manager: Option<String>,
    installable: bool,
    used_by: Vec<String>,
    status_code: String,
}

#[derive(Debug, Clone)]
struct HostDependencyDefinition {
    id: &'static str,
    category: &'static str,
    required: bool,
    commands: &'static [&'static str],
    version_args: &'static [&'static str],
    linux_package: Option<&'static str>,
    macos_package: Option<&'static str>,
    macos_cask: bool,
    used_by: &'static [&'static str],
}

#[derive(Debug, Deserialize)]
struct DependencyInstallRequest {
    dependency_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct DependencyInstallOperation {
    schema_version: u32,
    operation_id: String,
    dependency_id: String,
    status: String,
    package_manager: String,
    started_ts: Option<i64>,
    finished_ts: Option<i64>,
    exit_code: Option<i32>,
    log_tail: String,
    error_code: Option<String>,
}

#[derive(Debug, Clone)]
struct DependencyInstallCommand {
    program: String,
    args: Vec<String>,
    current_dir: Option<PathBuf>,
}

static DEPENDENCY_INSTALL_OPERATIONS: OnceLock<
    Arc<Mutex<HashMap<String, DependencyInstallOperation>>>,
> = OnceLock::new();
static DEPENDENCY_INSTALL_SEMAPHORE: OnceLock<Arc<Semaphore>> = OnceLock::new();

fn dependency_install_operations(
) -> &'static Arc<Mutex<HashMap<String, DependencyInstallOperation>>> {
    DEPENDENCY_INSTALL_OPERATIONS.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

fn dependency_install_semaphore() -> Arc<Semaphore> {
    DEPENDENCY_INSTALL_SEMAPHORE
        .get_or_init(|| Arc::new(Semaphore::new(1)))
        .clone()
}

async fn host_dependencies(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    let workspace_root = state.skill_rt.workspace_root.clone();
    let snapshot = tokio::task::spawn_blocking(move || collect_host_dependencies(&workspace_root))
        .await
        .unwrap_or_else(|_| HostDependenciesSnapshot::collection_failed());
    let snapshot = if identity.role.eq_ignore_ascii_case("admin") {
        snapshot
    } else {
        HostDependenciesSnapshot {
            operations: Vec::new(),
            ..snapshot
        }
    };
    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!(snapshot)),
            error: None,
        }),
    )
}

async fn start_dependency_install(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<DependencyInstallRequest>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return dependency_api_error(StatusCode::FORBIDDEN, "dependency_install_admin_required");
    }

    let dependency_id = request.dependency_id.trim();
    let Some(definition) = host_dependency_catalog()
        .into_iter()
        .find(|definition| definition.id == dependency_id)
    else {
        return dependency_api_error(StatusCode::NOT_FOUND, "dependency_unknown");
    };
    let canonical_dependency_id = definition.id.to_string();
    let preparation_definition = definition.clone();
    let workspace_root = state.skill_rt.workspace_root.clone();
    let preparation_root = workspace_root.clone();
    let preparation = tokio::task::spawn_blocking(move || {
        prepare_dependency_install(&preparation_definition, &preparation_root)
    })
    .await
    .unwrap_or(Err("dependency_preparation_failed"));
    let (package_manager, commands) = match preparation {
        Ok(prepared) => prepared,
        Err("dependency_already_installed") => {
            return dependency_api_error(StatusCode::CONFLICT, "dependency_already_installed");
        }
        Err(error_code) => {
            return dependency_api_error(StatusCode::UNPROCESSABLE_ENTITY, error_code);
        }
    };

    let operations = dependency_install_operations();
    let mut guard = operations
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(existing) = guard.values().find(|operation| {
        operation.dependency_id == canonical_dependency_id
            && matches!(operation.status.as_str(), "queued" | "running")
    }) {
        return (
            StatusCode::ACCEPTED,
            Json(ApiResponse {
                ok: true,
                data: Some(json!(existing)),
                error: None,
            }),
        );
    }
    prune_dependency_operations(&mut guard);
    let operation_id = uuid::Uuid::new_v4().to_string();
    let operation = DependencyInstallOperation {
        schema_version: 1,
        operation_id: operation_id.clone(),
        dependency_id: canonical_dependency_id.clone(),
        status: "queued".to_string(),
        package_manager: package_manager.clone(),
        started_ts: None,
        finished_ts: None,
        exit_code: None,
        log_tail: String::new(),
        error_code: None,
    };
    guard.insert(operation_id.clone(), operation.clone());
    drop(guard);

    tokio::spawn(run_dependency_install_operation(
        operation_id,
        canonical_dependency_id,
        workspace_root,
        commands,
    ));
    (
        StatusCode::ACCEPTED,
        Json(ApiResponse {
            ok: true,
            data: Some(json!(operation)),
            error: None,
        }),
    )
}

fn prepare_dependency_install(
    definition: &HostDependencyDefinition,
    workspace_root: &Path,
) -> Result<(String, Vec<DependencyInstallCommand>), &'static str> {
    if detect_dependency(definition, workspace_root).is_some() {
        return Err("dependency_already_installed");
    }
    if definition.id == "browser_playwright" {
        let commands = browser_playwright_install_commands(workspace_root)
            .ok_or("dependency_install_unsupported")?;
        return Ok(("npm".to_string(), commands));
    }
    let package_manager = detect_package_manager().ok_or("package_manager_unavailable")?;
    let commands = dependency_install_commands(definition, &package_manager)
        .ok_or("dependency_install_unsupported")?;
    Ok((package_manager, commands))
}

async fn get_dependency_install_operation(
    State(state): State<AppState>,
    headers: HeaderMap,
    AxumPath(operation_id): AxumPath<String>,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let identity = match require_ui_identity(&state, &headers) {
        Ok(identity) => identity,
        Err(response) => return response,
    };
    if !identity.role.eq_ignore_ascii_case("admin") {
        return dependency_api_error(StatusCode::FORBIDDEN, "dependency_install_admin_required");
    }
    let operation = dependency_install_operations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&operation_id)
        .cloned();
    match operation {
        Some(operation) => (
            StatusCode::OK,
            Json(ApiResponse {
                ok: true,
                data: Some(json!(operation)),
                error: None,
            }),
        ),
        None => dependency_api_error(StatusCode::NOT_FOUND, "dependency_operation_not_found"),
    }
}

fn dependency_api_error(
    status: StatusCode,
    error_code: &'static str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: Some(json!({
                "owner_layer": "host_dependencies",
                "error_code": error_code,
                "message_key": format!("clawd.host_dependencies.{error_code}"),
            })),
            error: Some(error_code.to_string()),
        }),
    )
}

async fn run_dependency_install_operation(
    operation_id: String,
    dependency_id: String,
    workspace_root: PathBuf,
    commands: Vec<DependencyInstallCommand>,
) {
    let Ok(_permit) = dependency_install_semaphore().acquire_owned().await else {
        update_dependency_operation(&operation_id, |operation| {
            operation.status = "failed".to_string();
            operation.finished_ts = Some(now_unix_seconds());
            operation.error_code = Some("dependency_install_queue_closed".to_string());
        });
        return;
    };
    update_dependency_operation(&operation_id, |operation| {
        operation.status = "running".to_string();
        operation.started_ts = Some(now_unix_seconds());
    });

    let mut combined_log = String::new();
    let mut last_exit_code = None;
    let mut error_code = None;
    for command in commands {
        let mut process = Command::new(&command.program);
        process.args(&command.args).kill_on_drop(true);
        if let Some(current_dir) = &command.current_dir {
            process.current_dir(current_dir);
        }
        process.env("PATH", dependency_install_path());
        let output = process.output().await;
        match output {
            Ok(output) => {
                last_exit_code = output.status.code();
                append_dependency_log(&mut combined_log, &output.stdout);
                append_dependency_log(&mut combined_log, &output.stderr);
                update_dependency_operation(&operation_id, |operation| {
                    operation.exit_code = last_exit_code;
                    operation.log_tail = bounded_tail(&combined_log, DEPENDENCY_INSTALL_LOG_LIMIT);
                });
                if !output.status.success() {
                    error_code = Some("package_install_failed".to_string());
                    break;
                }
            }
            Err(_) => {
                error_code = Some("package_manager_launch_failed".to_string());
                break;
            }
        }
    }

    let installed = error_code.is_none()
        && host_dependency_catalog()
            .into_iter()
            .find(|definition| definition.id == dependency_id)
            .and_then(|definition| detect_dependency(&definition, &workspace_root))
            .is_some();
    if error_code.is_none() && !installed {
        error_code = Some("dependency_still_missing".to_string());
    }
    update_dependency_operation(&operation_id, |operation| {
        operation.status = if error_code.is_none() {
            "succeeded".to_string()
        } else {
            "failed".to_string()
        };
        operation.finished_ts = Some(now_unix_seconds());
        operation.exit_code = last_exit_code;
        operation.log_tail = bounded_tail(&combined_log, DEPENDENCY_INSTALL_LOG_LIMIT);
        operation.error_code = error_code;
    });
}

fn update_dependency_operation(
    operation_id: &str,
    update: impl FnOnce(&mut DependencyInstallOperation),
) {
    let mut guard = dependency_install_operations()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(operation) = guard.get_mut(operation_id) {
        update(operation);
    }
}

fn prune_dependency_operations(operations: &mut HashMap<String, DependencyInstallOperation>) {
    if operations.len() < DEPENDENCY_OPERATION_LIMIT {
        return;
    }
    let mut finished = operations
        .values()
        .filter(|operation| !matches!(operation.status.as_str(), "queued" | "running"))
        .map(|operation| {
            (
                operation.finished_ts.unwrap_or(i64::MIN),
                operation.operation_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    finished.sort_by_key(|(finished_ts, _)| *finished_ts);
    let remove_count = operations
        .len()
        .saturating_sub(DEPENDENCY_OPERATION_LIMIT - 1);
    for (_, operation_id) in finished.into_iter().take(remove_count) {
        operations.remove(&operation_id);
    }
}

fn collect_host_dependencies(workspace_root: &Path) -> HostDependenciesSnapshot {
    let package_manager = detect_package_manager();
    let dependencies = host_dependency_catalog()
        .into_iter()
        .map(|definition| {
            let detected = detect_dependency(&definition, workspace_root);
            let installed = detected.is_some();
            let dependency_package_manager = if definition.id == "browser_playwright" {
                browser_playwright_install_commands(workspace_root)
                    .is_some()
                    .then(|| "npm".to_string())
            } else {
                package_manager.clone()
            };
            let installable = if definition.id == "browser_playwright" {
                dependency_package_manager.is_some()
            } else {
                dependency_install_commands(
                    &definition,
                    package_manager.as_deref().unwrap_or_default(),
                )
                .is_some()
            };
            HostDependencyStatus {
                id: definition.id.to_string(),
                category: definition.category.to_string(),
                required: definition.required,
                installed,
                version: detected.as_ref().map(|(_, version)| version.clone()),
                executable: detected.map(|(executable, _)| executable),
                package_manager: dependency_package_manager,
                installable,
                used_by: definition
                    .used_by
                    .iter()
                    .map(|value| (*value).to_string())
                    .collect(),
                status_code: if installed {
                    "installed".to_string()
                } else if definition.required {
                    "missing_required".to_string()
                } else {
                    "missing_optional".to_string()
                },
            }
        })
        .collect::<Vec<_>>();
    let installed = dependencies
        .iter()
        .filter(|dependency| dependency.installed)
        .count();
    let missing_required = dependencies
        .iter()
        .filter(|dependency| dependency.required && !dependency.installed)
        .count();
    let missing_optional = dependencies
        .iter()
        .filter(|dependency| !dependency.required && !dependency.installed)
        .count();
    HostDependenciesSnapshot {
        schema_version: 1,
        collected_at_ts: now_unix_seconds(),
        platform: std::env::consts::OS.to_string(),
        package_manager,
        summary: HostDependencySummary {
            total: dependencies.len(),
            installed,
            missing_required,
            missing_optional,
        },
        dependencies,
        operations: dependency_install_operations()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect(),
    }
}

impl HostDependenciesSnapshot {
    fn collection_failed() -> Self {
        Self {
            schema_version: 1,
            collected_at_ts: now_unix_seconds(),
            platform: std::env::consts::OS.to_string(),
            package_manager: detect_package_manager(),
            summary: HostDependencySummary {
                total: 0,
                installed: 0,
                missing_required: 0,
                missing_optional: 0,
            },
            dependencies: Vec::new(),
            operations: Vec::new(),
        }
    }
}

fn detect_dependency(
    definition: &HostDependencyDefinition,
    workspace_root: &Path,
) -> Option<(String, String)> {
    if definition.id == "browser_playwright" {
        return detect_browser_playwright(workspace_root);
    }
    if definition.id == "sandbox_backend" && cfg!(target_os = "macos") {
        return Path::new("/usr/bin/sandbox-exec")
            .is_file()
            .then(|| ("sandbox-exec".to_string(), "macOS Seatbelt".to_string()));
    }
    if definition.id == "process_tools" && cfg!(target_os = "macos") {
        return Path::new("/bin/ps")
            .is_file()
            .then(|| ("ps".to_string(), "macOS system utility".to_string()));
    }
    if definition.id == "libclang" {
        if let Some(version) = detect_libclang_library() {
            return Some(("libclang".to_string(), version));
        }
    }
    definition.commands.iter().find_map(|candidate| {
        dependency_command_candidates(candidate)
            .into_iter()
            .find_map(|program| {
                let output = StdCommand::new(&program)
                    .args(definition.version_args)
                    .stdin(StdProcessStdio::null())
                    .output()
                    .ok()?;
                if !output.status.success() {
                    return None;
                }
                let version =
                    dependency_version_text_for(definition.id, &output.stdout, &output.stderr)?;
                Some((dependency_executable_label(&program), version))
            })
    })
}

fn detect_browser_playwright(workspace_root: &Path) -> Option<(String, String)> {
    let package_dir = workspace_root.join("crates/skills/browser_web");
    let version = detect_browser_playwright_manifest(&package_dir)?;
    if !playwright_managed_browser_available(&package_dir) && detect_system_chromium().is_none() {
        return None;
    }
    Some((
        "browser_web/node_modules/playwright".to_string(),
        format!(
            "Playwright {}",
            version
                .chars()
                .take(DEPENDENCY_VERSION_LIMIT)
                .collect::<String>()
        ),
    ))
}

fn playwright_managed_browser_available(package_dir: &Path) -> bool {
    let node = dependency_command_candidates("node")
        .into_iter()
        .find(|candidate| candidate.is_file());
    let Some(node) = node else {
        return false;
    };
    let runtime_probe = r#"const fs=require('fs');const {chromium}=require('playwright');const p=chromium.executablePath();if(!p||!fs.existsSync(p)){process.exit(2)}"#;
    StdCommand::new(node)
        .args(["-e", runtime_probe])
        .current_dir(package_dir)
        .env("PATH", dependency_install_path())
        .stdin(StdProcessStdio::null())
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn detect_browser_playwright_manifest(package_dir: &Path) -> Option<String> {
    let manifest_path = package_dir.join("node_modules/playwright/package.json");
    let manifest: Value = serde_json::from_slice(&fs::read(&manifest_path).ok()?).ok()?;
    let version = manifest.get("version")?.as_str()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

fn detect_system_chromium() -> Option<PathBuf> {
    [
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/opt/homebrew/bin/chromium",
        "/usr/local/bin/chromium",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
        "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

fn browser_playwright_install_commands(
    workspace_root: &Path,
) -> Option<Vec<DependencyInstallCommand>> {
    let package_dir = workspace_root.join("crates/skills/browser_web");
    if !package_dir.join("package.json").is_file()
        || !package_dir.join("package-lock.json").is_file()
    {
        return None;
    }
    let npm = dependency_command_candidates("npm")
        .into_iter()
        .find(|candidate| {
            StdCommand::new(candidate)
                .arg("--version")
                .env("PATH", dependency_install_path())
                .stdout(StdProcessStdio::null())
                .stderr(StdProcessStdio::null())
                .status()
                .map(|status| status.success())
                .unwrap_or(false)
        })?;
    let node = dependency_command_candidates("node")
        .into_iter()
        .find(|candidate| candidate.is_file())?;
    let playwright_cli = package_dir.join("node_modules/playwright/cli.js");
    Some(vec![
        DependencyInstallCommand {
            program: npm.to_string_lossy().to_string(),
            args: vec![
                "ci".to_string(),
                "--omit=dev".to_string(),
                "--no-audit".to_string(),
                "--no-fund".to_string(),
            ],
            current_dir: Some(package_dir.clone()),
        },
        DependencyInstallCommand {
            program: node.to_string_lossy().to_string(),
            args: vec![
                playwright_cli.to_string_lossy().to_string(),
                "install".to_string(),
                "chromium".to_string(),
            ],
            current_dir: Some(package_dir),
        },
    ])
}

fn dependency_install_path() -> std::ffi::OsString {
    let mut paths = if cfg!(target_os = "macos") {
        vec![
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/sbin"),
            PathBuf::from("/sbin"),
        ]
    } else {
        vec![
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/usr/local/sbin"),
            PathBuf::from("/usr/sbin"),
        ]
    };
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home).join(".cargo/bin"));
    }
    if let Some(existing) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&existing) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    std::env::join_paths(paths).unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default())
}

fn dependency_command_candidates(candidate: &str) -> Vec<PathBuf> {
    let candidate_path = Path::new(candidate);
    if candidate_path.components().count() > 1 {
        return vec![candidate_path.to_path_buf()];
    }
    let mut candidates = Vec::new();
    let roots: &[&str] = if cfg!(target_os = "macos") {
        &[
            "/opt/homebrew/bin",
            "/usr/local/bin",
            "/opt/homebrew/sbin",
            "/usr/local/sbin",
            "/usr/bin",
            "/bin",
            "/usr/sbin",
            "/sbin",
        ]
    } else {
        &[
            "/usr/local/bin",
            "/usr/bin",
            "/bin",
            "/usr/local/sbin",
            "/usr/sbin",
        ]
    };
    candidates.extend(roots.iter().map(|root| Path::new(root).join(candidate)));
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".cargo/bin").join(candidate));
    }
    candidates.push(candidate_path.to_path_buf());
    candidates
}

fn dependency_version_text_for(
    dependency_id: &str,
    stdout: &[u8],
    stderr: &[u8],
) -> Option<String> {
    let text = if stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        String::from_utf8_lossy(stdout)
    } else {
        String::from_utf8_lossy(stderr)
    };
    if dependency_id == "zip" {
        if let Some(line) = text
            .lines()
            .map(str::trim)
            .find(|line| line.starts_with("This is Zip "))
        {
            return Some(line.chars().take(DEPENDENCY_VERSION_LIMIT).collect());
        }
    }
    if dependency_id == "lsof" {
        if let Some(revision) = text
            .lines()
            .map(str::trim)
            .find_map(|line| line.strip_prefix("revision:"))
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(format!("lsof {revision}"));
        }
    }
    dependency_version_text(stdout, stderr)
}

fn detect_libclang_library() -> Option<String> {
    let mut directories = vec![
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/local/lib"),
        PathBuf::from("/usr/lib/x86_64-linux-gnu"),
        PathBuf::from("/usr/lib/aarch64-linux-gnu"),
        PathBuf::from("/opt/homebrew/opt/llvm/lib"),
        PathBuf::from("/usr/local/opt/llvm/lib"),
        PathBuf::from(
            "/Applications/Xcode.app/Contents/Developer/Toolchains/XcodeDefault.xctoolchain/usr/lib",
        ),
        PathBuf::from("/Library/Developer/CommandLineTools/usr/lib"),
    ];
    directories
        .extend((12..=21).map(|version| PathBuf::from(format!("/usr/lib/llvm-{version}/lib"))));
    for directory in directories {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten().take(512) {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("libclang") && (name.contains(".so") || name.ends_with(".dylib")) {
                return Some(name.chars().take(DEPENDENCY_VERSION_LIMIT).collect());
            }
        }
    }
    None
}

fn dependency_executable_label(candidate: &Path) -> String {
    candidate
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("dependency")
        .to_string()
}

fn dependency_version_text(stdout: &[u8], stderr: &[u8]) -> Option<String> {
    let text = if stdout.iter().any(|byte| !byte.is_ascii_whitespace()) {
        String::from_utf8_lossy(stdout)
    } else {
        String::from_utf8_lossy(stderr)
    };
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(line.chars().take(DEPENDENCY_VERSION_LIMIT).collect())
}

fn detect_package_manager() -> Option<String> {
    let candidates: &[(&str, &str)] = if cfg!(target_os = "macos") {
        &[
            ("/opt/homebrew/bin/brew", "homebrew"),
            ("/usr/local/bin/brew", "homebrew"),
            ("brew", "homebrew"),
        ]
    } else {
        &[
            ("apt-get", "apt"),
            ("dnf", "dnf"),
            ("yum", "yum"),
            ("zypper", "zypper"),
            ("pacman", "pacman"),
            ("apk", "apk"),
        ]
    };
    candidates.iter().find_map(|(command, name)| {
        StdCommand::new(command)
            .arg("--version")
            .stdin(StdProcessStdio::null())
            .stdout(StdProcessStdio::null())
            .stderr(StdProcessStdio::null())
            .status()
            .ok()
            .filter(|status| status.success())
            .map(|_| (*name).to_string())
    })
}

fn dependency_install_commands(
    definition: &HostDependencyDefinition,
    package_manager: &str,
) -> Option<Vec<DependencyInstallCommand>> {
    let (package, cask) = if package_manager == "homebrew" {
        (definition.macos_package?, definition.macos_cask)
    } else {
        (
            linux_dependency_package(definition, package_manager)?,
            false,
        )
    };
    let manager_program = package_manager_program(package_manager)?;
    let (program, mut prefix) = privilege_prefix(package_manager, &manager_program)?;
    let mut commands = Vec::new();
    if package_manager == "apt" {
        let mut args = prefix.clone();
        args.extend(["update".to_string(), "-qq".to_string()]);
        commands.push(DependencyInstallCommand {
            program: program.clone(),
            args,
            current_dir: None,
        });
    }
    let install_args: &[&str] = match package_manager {
        "homebrew" => &["install"],
        "apt" | "dnf" | "yum" => &["install", "-y"],
        "zypper" => &["--non-interactive", "install"],
        "pacman" => &["-Sy", "--needed", "--noconfirm"],
        "apk" => &["add"],
        _ => return None,
    };
    prefix.extend(install_args.iter().map(|value| (*value).to_string()));
    if cask {
        prefix.push("--cask".to_string());
    }
    prefix.push(package.to_string());
    commands.push(DependencyInstallCommand {
        program,
        args: prefix,
        current_dir: None,
    });
    Some(commands)
}

fn package_manager_program(package_manager: &str) -> Option<String> {
    match package_manager {
        "homebrew" => ["/opt/homebrew/bin/brew", "/usr/local/bin/brew", "brew"]
            .into_iter()
            .find(|candidate| {
                StdCommand::new(candidate)
                    .arg("--version")
                    .stdout(StdProcessStdio::null())
                    .stderr(StdProcessStdio::null())
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false)
            })
            .map(str::to_string),
        "apt" => Some("apt-get".to_string()),
        "dnf" => Some("dnf".to_string()),
        "yum" => Some("yum".to_string()),
        "zypper" => Some("zypper".to_string()),
        "pacman" => Some("pacman".to_string()),
        "apk" => Some("apk".to_string()),
        _ => None,
    }
}

fn linux_dependency_package(
    definition: &HostDependencyDefinition,
    package_manager: &str,
) -> Option<&'static str> {
    match (definition.id, package_manager) {
        ("docker", "apt") => Some("docker.io"),
        ("docker", _) => Some("docker"),
        ("protoc", "apt" | "dnf" | "yum") => Some("protobuf-compiler"),
        ("protoc", "zypper") => Some("protobuf-devel"),
        ("protoc", "pacman" | "apk") => Some("protobuf"),
        ("pkg_config", "apk") => Some("pkgconf"),
        ("go", "apt") => Some("golang-go"),
        ("go", "dnf" | "yum") => Some("golang"),
        ("go", "zypper" | "pacman" | "apk") => Some("go"),
        ("rustc", "dnf" | "yum" | "zypper" | "pacman" | "apk") => Some("rust"),
        ("pdf_tools", "apt" | "dnf" | "yum") => Some("poppler-utils"),
        ("pdf_tools", "zypper" | "pacman") => Some("poppler"),
        ("pdf_tools", "apk") => Some("poppler-utils"),
        ("libclang", "apt") => Some("libclang-dev"),
        ("libclang", "dnf" | "yum") => Some("clang-devel"),
        ("libclang", "zypper") => Some("llvm-clang-devel"),
        ("libclang", "pacman") => Some("clang"),
        ("libclang", "apk") => Some("clang-extra-tools"),
        _ => definition.linux_package,
    }
}

fn privilege_prefix(package_manager: &str, manager_program: &str) -> Option<(String, Vec<String>)> {
    if package_manager == "homebrew" || unsafe { libc::geteuid() } == 0 {
        return Some((manager_program.to_string(), Vec::new()));
    }
    if StdCommand::new("sudo")
        .args(["-n", "true"])
        .stdout(StdProcessStdio::null())
        .stderr(StdProcessStdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
    {
        return Some((
            "sudo".to_string(),
            vec!["-n".to_string(), manager_program.to_string()],
        ));
    }
    None
}

fn append_dependency_log(target: &mut String, bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    if !target.is_empty() && !target.ends_with('\n') {
        target.push('\n');
    }
    target.push_str(&String::from_utf8_lossy(bytes));
    *target = bounded_tail(target, DEPENDENCY_INSTALL_LOG_LIMIT);
}

fn bounded_tail(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_string();
    }
    value.chars().skip(count - max_chars).collect()
}

fn host_dependency_catalog() -> Vec<HostDependencyDefinition> {
    vec![
        dependency(
            "bash",
            "runtime",
            true,
            &["bash"],
            &["--version"],
            Some("bash"),
            Some("bash"),
            false,
            &["runtime_scripts", "deployment"],
        ),
        dependency(
            "tar",
            "runtime",
            true,
            &["tar"],
            &["--version"],
            Some("tar"),
            None,
            false,
            &["system_update", "deployment"],
        ),
        dependency(
            "git",
            "build",
            false,
            &["git"],
            &["--version"],
            Some("git"),
            Some("git"),
            false,
            &["workspace", "system_update"],
        ),
        dependency(
            "curl",
            "runtime",
            true,
            &["curl"],
            &["--version"],
            Some("curl"),
            Some("curl"),
            false,
            &["http_basic", "system_update"],
        ),
        dependency(
            "python3",
            "runtime",
            true,
            &["python3"],
            &["--version"],
            Some("python3"),
            Some("python"),
            false,
            &["runtime_scripts", "nni"],
        ),
        dependency(
            "sandbox_backend",
            "runtime",
            true,
            &["bwrap"],
            &["--version"],
            Some("bubblewrap"),
            None,
            false,
            &["agent_tools", "skill_runtime"],
        ),
        dependency(
            "process_tools",
            "tool",
            false,
            &["ps"],
            &["--version"],
            Some("procps"),
            None,
            false,
            &["process_basic", "health_check"],
        ),
        dependency(
            "rustc",
            "build",
            false,
            &["rustc"],
            &["--version"],
            Some("rustc"),
            None,
            false,
            &["source_build"],
        ),
        dependency(
            "cargo",
            "build",
            false,
            &["cargo"],
            &["--version"],
            Some("cargo"),
            None,
            false,
            &["source_build", "skill_store"],
        ),
        dependency(
            "clang",
            "build",
            false,
            &["clang"],
            &["--version"],
            Some("clang"),
            None,
            false,
            &["source_build", "native_bindings"],
        ),
        dependency(
            "libclang",
            "build",
            false,
            &[
                "llvm-config",
                "llvm-config-19",
                "llvm-config-18",
                "/opt/homebrew/opt/llvm/bin/llvm-config",
                "/usr/local/opt/llvm/bin/llvm-config",
            ],
            &["--version"],
            Some("libclang-dev"),
            Some("llvm"),
            false,
            &["source_build", "native_bindings"],
        ),
        dependency(
            "cmake",
            "build",
            false,
            &["cmake"],
            &["--version"],
            Some("cmake"),
            Some("cmake"),
            false,
            &["source_build", "native_bindings"],
        ),
        dependency(
            "pkg_config",
            "build",
            false,
            &["pkg-config"],
            &["--version"],
            Some("pkg-config"),
            Some("pkg-config"),
            false,
            &["source_build", "native_bindings"],
        ),
        dependency(
            "protoc",
            "build",
            false,
            &["protoc"],
            &["--version"],
            Some("protobuf-compiler"),
            Some("protobuf"),
            false,
            &["source_build"],
        ),
        dependency(
            "make",
            "build",
            false,
            &["make"],
            &["--version"],
            Some("make"),
            None,
            false,
            &["source_build", "native_bindings"],
        ),
        dependency(
            "node",
            "build",
            false,
            &["node"],
            &["--version"],
            Some("nodejs"),
            Some("node"),
            false,
            &["ui_build"],
        ),
        dependency(
            "npm",
            "build",
            false,
            &["npm"],
            &["--version"],
            Some("npm"),
            Some("node"),
            false,
            &["ui_build"],
        ),
        dependency(
            "npx",
            "skill",
            false,
            &["npx"],
            &["--version"],
            Some("npm"),
            Some("node"),
            false,
            &["image_vision"],
        ),
        dependency(
            "browser_playwright",
            "skill",
            false,
            &[],
            &[],
            None,
            None,
            false,
            &["browser_web"],
        ),
        dependency(
            "go",
            "build",
            false,
            &["go"],
            &["version"],
            Some("golang-go"),
            Some("go"),
            false,
            &["install_module"],
        ),
        dependency(
            "ripgrep",
            "tool",
            false,
            &["rg"],
            &["--version"],
            Some("ripgrep"),
            Some("ripgrep"),
            false,
            &["fs_search", "code_index"],
        ),
        dependency(
            "zip",
            "tool",
            false,
            &["zip"],
            &["-v"],
            Some("zip"),
            Some("zip"),
            false,
            &["archive_basic"],
        ),
        dependency(
            "unzip",
            "tool",
            false,
            &["unzip"],
            &["-v"],
            Some("unzip"),
            Some("unzip"),
            false,
            &["archive_basic"],
        ),
        dependency(
            "pdf_tools",
            "skill",
            false,
            &["pdftotext"],
            &["-v"],
            Some("poppler-utils"),
            Some("poppler"),
            false,
            &["doc_parse"],
        ),
        dependency(
            "lsof",
            "tool",
            false,
            &["lsof"],
            &["-v"],
            Some("lsof"),
            Some("lsof"),
            false,
            &["process_basic", "health_check"],
        ),
        dependency(
            "ffmpeg",
            "skill",
            false,
            &["ffmpeg"],
            &["-version"],
            Some("ffmpeg"),
            Some("ffmpeg"),
            false,
            &["audio_transcribe", "video_generate", "music_generate"],
        ),
        dependency(
            "docker",
            "skill",
            false,
            &[
                "docker",
                "/Applications/Docker.app/Contents/Resources/bin/docker",
            ],
            &["--version"],
            Some("docker.io"),
            Some("docker"),
            true,
            &["docker_basic"],
        ),
        dependency(
            "chromium",
            "skill",
            false,
            &[
                "chromium",
                "chromium-browser",
                "google-chrome",
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            ],
            &["--version"],
            Some("chromium"),
            Some("google-chrome"),
            true,
            &["browser_web"],
        ),
        dependency(
            "libreoffice",
            "skill",
            false,
            &[
                "libreoffice",
                "/Applications/LibreOffice.app/Contents/MacOS/soffice",
            ],
            &["--version"],
            Some("libreoffice"),
            Some("libreoffice"),
            true,
            &["office_workspace"],
        ),
        dependency(
            "nginx",
            "optional",
            false,
            &["nginx", "/opt/homebrew/bin/nginx", "/usr/local/bin/nginx"],
            &["-v"],
            Some("nginx"),
            Some("nginx"),
            false,
            &["web_entry"],
        ),
        dependency(
            "rsync",
            "optional",
            false,
            &["rsync"],
            &["--version"],
            Some("rsync"),
            Some("rsync"),
            false,
            &["deployment"],
        ),
    ]
}

fn dependency(
    id: &'static str,
    category: &'static str,
    required: bool,
    commands: &'static [&'static str],
    version_args: &'static [&'static str],
    linux_package: Option<&'static str>,
    macos_package: Option<&'static str>,
    macos_cask: bool,
    used_by: &'static [&'static str],
) -> HostDependencyDefinition {
    HostDependencyDefinition {
        id,
        category,
        required,
        commands,
        version_args,
        linux_package,
        macos_package,
        macos_cask,
        used_by,
    }
}
