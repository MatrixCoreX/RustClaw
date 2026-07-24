#[derive(Debug, Clone)]
struct SkillStoreInstallSpec {
    package: String,
    binary: String,
}

#[derive(Debug, Clone, Copy)]
enum SkillStoreErrorCode {
    NameRequired,
    UnknownSkill,
    LockedSkill,
    RegistryUnavailable,
    ConfigReadFailed,
    ConfigWriteFailed,
    RuntimeReloadFailed,
    InvalidRunnerName,
    InstallNotOnDemand,
    InvalidInstallPackage,
    UnsafeConfigPath,
    BuildStartFailed,
    BuildFailed,
    #[cfg(not(test))]
    BuildBinaryMissing,
    BinaryRemoveFailed,
    ConfigRemoveFailed,
    DataRemoveFailed,
    OperationBusy,
}

impl SkillStoreErrorCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::NameRequired => "skill_store_name_required",
            Self::UnknownSkill => "skill_store_unknown_skill",
            Self::LockedSkill => "skill_store_locked_skill",
            Self::RegistryUnavailable => "skill_store_registry_unavailable",
            Self::ConfigReadFailed => "skill_store_config_read_failed",
            Self::ConfigWriteFailed => "skill_store_config_write_failed",
            Self::RuntimeReloadFailed => "skill_store_runtime_reload_failed",
            Self::InvalidRunnerName => "skill_store_invalid_runner_name",
            Self::InstallNotOnDemand => "skill_store_install_not_on_demand",
            Self::InvalidInstallPackage => "skill_store_invalid_install_package",
            Self::UnsafeConfigPath => "skill_store_unsafe_config_path",
            Self::BuildStartFailed => "skill_store_build_start_failed",
            Self::BuildFailed => "skill_store_build_failed",
            #[cfg(not(test))]
            Self::BuildBinaryMissing => "skill_store_build_binary_missing",
            Self::BinaryRemoveFailed => "skill_store_binary_remove_failed",
            Self::ConfigRemoveFailed => "skill_store_config_remove_failed",
            Self::DataRemoveFailed => "skill_store_data_remove_failed",
            Self::OperationBusy => "skill_store_operation_busy",
        }
    }
}

#[derive(Debug)]
struct SkillStoreOperationError {
    status: StatusCode,
    code: SkillStoreErrorCode,
    diagnostic: String,
}

impl SkillStoreOperationError {
    fn new(
        status: StatusCode,
        code: SkillStoreErrorCode,
        diagnostic: impl std::fmt::Display,
    ) -> Self {
        Self {
            status,
            code,
            diagnostic: diagnostic.to_string(),
        }
    }
}

type SkillStoreOperationResult<T> = Result<T, SkillStoreOperationError>;

#[derive(Debug, Clone)]
struct SkillStoreActiveOperation {
    skill_name: String,
    action: &'static str,
    started_ts: u64,
}

struct SkillStoreMutationSlot {
    semaphore: Arc<Semaphore>,
    active: Mutex<Option<SkillStoreActiveOperation>>,
}

struct SkillStoreMutationGuard {
    _permit: OwnedSemaphorePermit,
    slot: Arc<SkillStoreMutationSlot>,
}

impl Drop for SkillStoreMutationGuard {
    fn drop(&mut self) {
        let mut active = self
            .slot
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *active = None;
    }
}

fn skill_store_mutation_slot(state: &AppState) -> Arc<SkillStoreMutationSlot> {
    static SLOTS: OnceLock<Mutex<HashMap<PathBuf, Arc<SkillStoreMutationSlot>>>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(state.skill_rt.workspace_root.clone())
        .or_insert_with(|| {
            Arc::new(SkillStoreMutationSlot {
                semaphore: Arc::new(Semaphore::new(1)),
                active: Mutex::new(None),
            })
        })
        .clone()
}

fn begin_skill_store_mutation(
    state: &AppState,
    skill_name: &str,
    action: &'static str,
) -> SkillStoreOperationResult<SkillStoreMutationGuard> {
    let slot = skill_store_mutation_slot(state);
    let permit = slot
        .semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::OperationBusy,
                error,
            )
        })?;
    let operation = SkillStoreActiveOperation {
        skill_name: skill_name.to_string(),
        action,
        started_ts: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0),
    };
    let mut active = slot
        .active
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *active = Some(operation);
    drop(active);
    Ok(SkillStoreMutationGuard {
        _permit: permit,
        slot,
    })
}

fn skill_store_active_operation(state: &AppState) -> Option<SkillStoreActiveOperation> {
    skill_store_mutation_slot(state)
        .active
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

fn skill_store_error_response(
    error: SkillStoreOperationError,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    tracing::warn!(
        error_code = error.code.as_str(),
        diagnostic = %error.diagnostic,
        "skill_store_operation_failed"
    );
    (
        error.status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(error.code.as_str().to_string()),
        }),
    )
}

fn runner_binary_name(raw_name: &str) -> SkillStoreOperationResult<String> {
    let raw_name = raw_name.trim();
    if raw_name.is_empty()
        || raw_name.contains('/')
        || raw_name.contains('\\')
        || !raw_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(SkillStoreOperationError::new(
            StatusCode::BAD_REQUEST,
            SkillStoreErrorCode::InvalidRunnerName,
            format!("runner={raw_name}"),
        ));
    }
    let normalized = raw_name.replace('_', "-");
    Ok(if normalized.ends_with("-skill") {
        normalized
    } else {
        format!("{normalized}-skill")
    })
}

fn skill_store_install_spec(
    state: &AppState,
    skill_name: &str,
) -> SkillStoreOperationResult<Option<SkillStoreInstallSpec>> {
    let registry = state
        .get_skills_registry()
        .ok_or_else(|| {
            SkillStoreOperationError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                SkillStoreErrorCode::RegistryUnavailable,
                "registry=unavailable",
            )
        })?;
    let entry = registry.get(skill_name).ok_or_else(|| {
        SkillStoreOperationError::new(
            StatusCode::NOT_FOUND,
            SkillStoreErrorCode::UnknownSkill,
            format!("skill={skill_name}"),
        )
    })?;
    if entry.kind != SkillKind::Runner {
        return Ok(None);
    }
    if entry.install_mode.as_deref() != Some("on_demand") {
        return Err(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::InstallNotOnDemand,
            format!("skill={skill_name} install_mode={:?}", entry.install_mode),
        ));
    }
    let binary = runner_binary_name(&registry.runner_name(skill_name))?;
    let package = entry
        .install_package
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(binary.as_str())
        .to_string();
    if !package
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    {
        return Err(SkillStoreOperationError::new(
            StatusCode::BAD_REQUEST,
            SkillStoreErrorCode::InvalidInstallPackage,
            format!("package={package}"),
        ));
    }
    Ok(Some(SkillStoreInstallSpec { package, binary }))
}

fn declared_skill_config_paths(
    state: &AppState,
    skill_name: &str,
) -> SkillStoreOperationResult<Vec<PathBuf>> {
    let Some(registry) = state.get_skills_registry() else {
        return Ok(Vec::new());
    };
    let Some(entry) = registry.get(skill_name) else {
        return Ok(Vec::new());
    };
    entry
        .config_files
        .iter()
        .map(|relative| {
            let relative_path = Path::new(relative);
            let safe = !relative_path.is_absolute()
                && relative_path
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_)))
                && relative_path.starts_with("configs");
            if !safe {
                return Err(SkillStoreOperationError::new(
                    StatusCode::BAD_REQUEST,
                    SkillStoreErrorCode::UnsafeConfigPath,
                    format!("skill={skill_name} path={relative}"),
                ));
            }
            Ok(state.skill_rt.workspace_root.join(relative_path))
        })
        .collect()
}

fn skill_config_state(state: &AppState, skill_name: &str) -> (Vec<String>, Vec<String>) {
    let Ok(paths) = declared_skill_config_paths(state, skill_name) else {
        return (Vec::new(), Vec::new());
    };
    let declared = paths
        .iter()
        .filter_map(|path| {
            path.strip_prefix(&state.skill_rt.workspace_root)
                .ok()
                .map(|relative| relative.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    let existing = paths
        .iter()
        .filter(|path| path.is_file())
        .filter_map(|path| {
            path.strip_prefix(&state.skill_rt.workspace_root)
                .ok()
                .map(|relative| relative.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>();
    (declared, existing)
}

#[cfg(not(test))]
fn bounded_command_error(bytes: &[u8]) -> String {
    const MAX_CHARS: usize = 4_000;
    let text = String::from_utf8_lossy(bytes);
    let chars = text.chars().count();
    if chars <= MAX_CHARS {
        return text.into_owned();
    }
    text.chars().skip(chars - MAX_CHARS).collect()
}

#[cfg(not(test))]
async fn compile_skill_store_runner(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
) -> SkillStoreOperationResult<PathBuf> {
    let output = Command::new("cargo")
        .args(["build", "--release", "-p", spec.package.as_str()])
        .current_dir(&state.skill_rt.workspace_root)
        .env(
            "CARGO_TARGET_DIR",
            state.skill_rt.workspace_root.join("target"),
        )
        .output()
        .await
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::BuildStartFailed,
                format!("package={} error={error}", spec.package),
            )
        })?;
    if !output.status.success() {
        return Err(SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::BuildFailed,
            format!(
                "package={} stderr={}",
                spec.package,
                bounded_command_error(&output.stderr)
            ),
        ));
    }
    let binary_path = state
        .skill_rt
        .workspace_root
        .join("target/release")
        .join(&spec.binary);
    if !binary_path.is_file() {
        return Err(SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::BuildBinaryMissing,
            format!("path={}", binary_path.display()),
        ));
    }
    Ok(binary_path)
}

#[cfg(test)]
async fn compile_skill_store_runner(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
) -> SkillStoreOperationResult<PathBuf> {
    let _package = &spec.package;
    let binary_path = state
        .skill_rt
        .workspace_root
        .join("target/release")
        .join(&spec.binary);
    if let Some(parent) = binary_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::BuildStartFailed,
                error,
            )
        })?;
    }
    fs::write(&binary_path, b"skill-store-test-binary").map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::BuildFailed,
            error,
        )
    })?;
    Ok(binary_path)
}

fn remove_skill_store_binary(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
) -> SkillStoreOperationResult<bool> {
    let path = state
        .skill_rt
        .workspace_root
        .join("target/release")
        .join(&spec.binary);
    if !path.exists() {
        return Ok(false);
    }
    fs::remove_file(&path).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::BinaryRemoveFailed,
            format!("path={} error={error}", path.display()),
        )
    })?;
    Ok(true)
}

fn delete_declared_skill_configs(
    state: &AppState,
    skill_name: &str,
) -> SkillStoreOperationResult<Vec<String>> {
    let mut deleted = Vec::new();
    for path in declared_skill_config_paths(state, skill_name)? {
        if !path.exists() {
            continue;
        }
        fs::remove_file(&path).map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::ConfigRemoveFailed,
                format!("path={} error={error}", path.display()),
            )
        })?;
        if let Ok(relative) = path.strip_prefix(&state.skill_rt.workspace_root) {
            deleted.push(relative.to_string_lossy().into_owned());
        }
    }
    Ok(deleted)
}
