#[derive(Debug, Clone)]
struct SkillStoreInstallSpec {
    skill_name: String,
    manifest_path: PathBuf,
    adapter: rustclaw_skill_sdk::BuildAdapter,
    network_policy: rustclaw_skill_sdk::BuildNetworkPolicy,
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
    InstallNotOnDemand,
    UnsupportedOs,
    ManifestMissing,
    ManifestInvalid,
    NetworkApprovalRequired,
    UnsafeConfigPath,
    #[cfg(not(test))]
    InstallStartFailed,
    InstallFailed,
    PackageRemoveFailed,
    ConfigRemoveFailed,
    DataRemoveFailed,
    OperationBusy,
    OperationStateFailed,
    OperationNotFound,
    RollbackUnavailable,
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
            Self::InstallNotOnDemand => "skill_store_install_not_on_demand",
            Self::UnsupportedOs => "skill_store_unsupported_os",
            Self::ManifestMissing => "skill_store_manifest_missing",
            Self::ManifestInvalid => "skill_store_manifest_invalid",
            Self::NetworkApprovalRequired => "skill_store_network_approval_required",
            Self::UnsafeConfigPath => "skill_store_unsafe_config_path",
            #[cfg(not(test))]
            Self::InstallStartFailed => "skill_store_install_start_failed",
            Self::InstallFailed => "skill_store_install_failed",
            Self::PackageRemoveFailed => "skill_store_package_remove_failed",
            Self::ConfigRemoveFailed => "skill_store_config_remove_failed",
            Self::DataRemoveFailed => "skill_store_data_remove_failed",
            Self::OperationBusy => "skill_store_operation_busy",
            Self::OperationStateFailed => "skill_store_operation_state_failed",
            Self::OperationNotFound => "skill_store_operation_not_found",
            Self::RollbackUnavailable => "skill_store_rollback_unavailable",
        }
    }
}

#[derive(Debug)]
struct SkillStoreOperationError {
    status: StatusCode,
    code: SkillStoreErrorCode,
    diagnostic: String,
    phase: Option<String>,
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
            phase: None,
        }
    }

    #[cfg_attr(test, allow(dead_code))]
    fn with_phase(mut self, phase: Option<String>) -> Self {
        self.phase = phase;
        self
    }
}

type SkillStoreOperationResult<T> = Result<T, SkillStoreOperationError>;

struct SkillStoreMutationSlot {
    build_semaphore: Arc<Semaphore>,
    config_semaphore: Arc<Semaphore>,
    skill_semaphores: Mutex<HashMap<String, Arc<Semaphore>>>,
    controls: Mutex<HashMap<String, rustclaw_skill_sdk::InstallControl>>,
    recovered: AtomicBool,
}

struct SkillStoreMutationGuard {
    _permit: OwnedSemaphorePermit,
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
                build_semaphore: Arc::new(Semaphore::new(1)),
                config_semaphore: Arc::new(Semaphore::new(1)),
                skill_semaphores: Mutex::new(HashMap::new()),
                controls: Mutex::new(HashMap::new()),
                recovered: AtomicBool::new(false),
            })
        })
        .clone()
}

fn begin_skill_store_mutation(
    state: &AppState,
    skill_name: &str,
) -> SkillStoreOperationResult<SkillStoreMutationGuard> {
    let slot = skill_store_mutation_slot(state);
    let skill_semaphore = slot
        .skill_semaphores
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .entry(skill_name.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(1)))
        .clone();
    let permit = skill_semaphore
        .clone()
        .try_acquire_owned()
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::OperationBusy,
                error,
            )
        })?;
    Ok(SkillStoreMutationGuard { _permit: permit })
}

async fn skill_store_build_permit(
    state: &AppState,
    control: &rustclaw_skill_sdk::InstallControl,
) -> Option<OwnedSemaphorePermit> {
    let semaphore = skill_store_mutation_slot(state).build_semaphore.clone();
    loop {
        if control.is_cancelled() {
            return None;
        }
        match semaphore.clone().try_acquire_owned() {
            Ok(permit) => return Some(permit),
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(tokio::sync::TryAcquireError::Closed) => return None,
        }
    }
}

async fn skill_store_config_permit(state: &AppState) -> OwnedSemaphorePermit {
    skill_store_mutation_slot(state)
        .config_semaphore
        .clone()
        .acquire_owned()
        .await
        .expect("skill store config semaphore remains open")
}

fn skill_store_operation_store(state: &AppState) -> rustclaw_skill_sdk::SkillOperationStore {
    rustclaw_skill_sdk::SkillOperationStore::new(skill_package_root(state))
}

fn initialize_skill_store_operations(state: &AppState) -> SkillStoreOperationResult<()> {
    let slot = skill_store_mutation_slot(state);
    if slot
        .recovered
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_ok()
    {
        skill_store_operation_store(state)
            .recover_interrupted()
            .map_err(|error| {
                slot.recovered.store(false, Ordering::Release);
                SkillStoreOperationError::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    SkillStoreErrorCode::OperationStateFailed,
                    error,
                )
            })?;
    }
    Ok(())
}

fn register_skill_store_control(
    state: &AppState,
    operation_id: &str,
    control: rustclaw_skill_sdk::InstallControl,
) {
    skill_store_mutation_slot(state)
        .controls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(operation_id.to_string(), control);
}

fn remove_skill_store_control(state: &AppState, operation_id: &str) {
    skill_store_mutation_slot(state)
        .controls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(operation_id);
}

fn request_live_skill_store_cancel(state: &AppState, operation_id: &str) {
    if let Some(control) = skill_store_mutation_slot(state)
        .controls
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(operation_id)
        .cloned()
    {
        control.request_cancel();
    }
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

fn skill_package_root(state: &AppState) -> PathBuf {
    state.skill_rt.workspace_root.join("data/skill-packages")
}

#[cfg(not(test))]
fn precompiled_skill_package_root(state: &AppState) -> PathBuf {
    state.skill_rt.workspace_root.join("prebuilt/skill-packages")
}

fn skill_store_package_available(
    state: &AppState,
    registry: &claw_core::skill_registry::SkillsRegistry,
    skill_name: &str,
) -> bool {
    let _ = registry;
    rustclaw_skill_sdk::SkillRuntimeResolver::new(skill_package_root(state))
        .resolve(skill_name)
        .is_ok()
}

fn skill_store_manifest_metadata(
    state: &AppState,
    registry: &claw_core::skill_registry::SkillsRegistry,
    skill_name: &str,
) -> Option<rustclaw_skill_sdk::PackageManifest> {
    let relative = registry.package_manifest_path(skill_name)?;
    rustclaw_skill_sdk::PackageManifest::load(&state.skill_rt.workspace_root.join(relative)).ok()
}

fn skill_store_install_spec(
    state: &AppState,
    skill_name: &str,
) -> SkillStoreOperationResult<Option<SkillStoreInstallSpec>> {
    let registry = state.get_skills_registry().ok_or_else(|| {
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
    if !matches!(entry.kind, SkillKind::Runner | SkillKind::External) {
        return Ok(None);
    }
    if entry.install_mode.as_deref() != Some("on_demand") {
        return Err(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::InstallNotOnDemand,
            format!("skill={skill_name} install_mode={:?}", entry.install_mode),
        ));
    }
    let availability = crate::skill_availability::evaluate_entry_availability(entry);
    if let Some(supported_os) = availability.unsupported_os {
        return Err(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::UnsupportedOs,
            format!(
                "skill={skill_name} current_os={} supported_os={}",
                availability.current_os,
                supported_os.join(",")
            ),
        ));
    }
    let relative_manifest = registry.package_manifest_path(skill_name).ok_or_else(|| {
        SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::ManifestMissing,
            format!("skill={skill_name}"),
        )
    })?;
    let manifest_path = state.skill_rt.workspace_root.join(relative_manifest);
    let manifest = rustclaw_skill_sdk::PackageManifest::load(&manifest_path).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::ManifestInvalid,
            format!(
                "skill={skill_name} code={} detail={}",
                error.code, error.detail
            ),
        )
    })?;
    if manifest.package.name != skill_name || manifest.registry.name != skill_name {
        return Err(SkillStoreOperationError::new(
            StatusCode::CONFLICT,
            SkillStoreErrorCode::ManifestInvalid,
            format!(
                "registry={skill_name} package={} projection={}",
                manifest.package.name, manifest.registry.name
            ),
        ));
    }
    Ok(Some(SkillStoreInstallSpec {
        skill_name: skill_name.to_string(),
        manifest_path,
        adapter: manifest.build.adapter,
        network_policy: manifest.build.network,
    }))
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
async fn install_skill_store_package(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
    control: rustclaw_skill_sdk::InstallControl,
    allow_network: bool,
) -> SkillStoreOperationResult<rustclaw_skill_sdk::InstallOutcome> {
    let manifest_path = spec.manifest_path.clone();
    let workspace_root = state.skill_rt.workspace_root.clone();
    let package_root = skill_package_root(state);
    let precompiled_root = precompiled_skill_package_root(state);
    tokio::task::spawn_blocking(move || {
        if precompiled_root.is_dir() {
            let precompiled = rustclaw_skill_sdk::PrecompiledInstallRequest {
                manifest_path: manifest_path.clone(),
                workspace_root: workspace_root.clone(),
                package_root: package_root.clone(),
                precompiled_root,
                target: None,
                control: Some(control.clone()),
            };
            match rustclaw_skill_sdk::SkillInstaller.install_precompiled(&precompiled) {
                Ok(outcome) => return Ok(outcome),
                Err(error) if precompiled_source_fallback_allowed(&error.code) => {}
                Err(error) => return Err(error),
            }
        }
        rustclaw_skill_sdk::SkillInstaller.install(&rustclaw_skill_sdk::InstallRequest {
            manifest_path,
            workspace_root,
            package_root,
            target: None,
            allow_network,
            control: Some(control),
        })
    })
        .await
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::InstallStartFailed,
                format!("skill={} error={error}", spec.skill_name),
            )
        })?
        .map_err(|error| {
            let phase = error.phase.clone();
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::InstallFailed,
                format!(
                    "skill={} adapter={} phase={:?} code={} detail={}",
                    spec.skill_name,
                    spec.adapter.as_token(),
                    error.phase,
                    error.code,
                    error.detail
                ),
            )
            .with_phase(phase)
        })
}

fn precompiled_source_fallback_allowed(error_code: &str) -> bool {
    matches!(
        error_code,
        "precompiled_package_unavailable"
            | "precompiled_platform_mismatch"
            | "precompiled_manifest_mismatch"
    )
}

#[cfg(test)]
async fn install_skill_store_package(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
    control: rustclaw_skill_sdk::InstallControl,
    _allow_network: bool,
) -> SkillStoreOperationResult<rustclaw_skill_sdk::InstallOutcome> {
    use rustclaw_skill_sdk::receipt::{LaunchProgramScope, ReceiptLaunch};
    use rustclaw_skill_sdk::{
        ArtifactReceipt, HostPlatform, InstallReceipt, InstallReceiptStore, PackageManifest,
        ProtocolSmokeReceipt, INSTALL_RECEIPT_SCHEMA_VERSION,
    };

    for phase in ["dependencies", "build", "protocol_smoke", "activate"] {
        control.phase(phase).map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::CONFLICT,
                SkillStoreErrorCode::InstallFailed,
                error,
            )
        })?;
    }
    let manifest = PackageManifest::load(&spec.manifest_path).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    let store = InstallReceiptStore::new(skill_package_root(state));
    let identity = manifest.digest().map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    let destination = store
        .version_dir(&spec.skill_name, &manifest.package.version, &identity)
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::InstallFailed,
                error,
            )
        })?;
    fs::create_dir_all(&destination).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    let program_rel = "runtime/bin/skill-test-runner";
    let program = destination.join(program_rel);
    fs::create_dir_all(program.parent().expect("test runner parent")).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    fs::write(&program, b"#!/bin/sh\nexit 0\n").map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::InstallFailed,
                error,
            )
        })?;
    }
    fs::write(
        destination.join("skill.toml"),
        manifest.to_toml_string().map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::InstallFailed,
                error,
            )
        })?,
    )
    .map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    let metadata = fs::metadata(&program).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    let artifact_digest = rustclaw_skill_sdk::receipt::digest_file(&program).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    let receipt = InstallReceipt {
        schema_version: INSTALL_RECEIPT_SCHEMA_VERSION,
        skill_name: spec.skill_name.clone(),
        version: manifest.package.version.clone(),
        manifest_digest: identity,
        source_digest: artifact_digest.clone(),
        lockfile_digests: BTreeMap::new(),
        adapter: spec.adapter,
        adapter_version: "skill-store-test-adapter-v1".to_string(),
        platform: HostPlatform::current(),
        artifacts: vec![ArtifactReceipt {
            path: program_rel.to_string(),
            sha256: artifact_digest,
            size_bytes: metadata.len(),
            executable: true,
        }],
        launch: ReceiptLaunch {
            launcher: manifest.run.launcher,
            program: program_rel.to_string(),
            program_scope: LaunchProgramScope::Package,
            args: Vec::new(),
            working_directory: ".".to_string(),
            environment: BTreeMap::new(),
            environment_allowlist: manifest.run.environment_allowlist.clone(),
            trusted_runtime_sha256: None,
            trusted_runtime_version: None,
            remote_endpoint: None,
        },
        sandbox_profile: manifest.security.sandbox,
        runtime_network: manifest.security.runtime_network,
        protocol_smoke: ProtocolSmokeReceipt {
            protocol: rustclaw_skill_sdk::RUSTCLAW_JSONL_PROTOCOL.to_string(),
            passed: true,
            request_id: "skill-store-test-smoke".to_string(),
            checked_at_unix: 1,
        },
        installed_at_unix: 1,
    };
    store
        .write_receipt(&destination, &receipt)
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::InstallFailed,
                error,
            )
        })?;
    store.activate(&destination, &receipt).map_err(|error| {
        SkillStoreOperationError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            SkillStoreErrorCode::InstallFailed,
            error,
        )
    })?;
    Ok(rustclaw_skill_sdk::InstallOutcome {
        skill_name: spec.skill_name.clone(),
        version: manifest.package.version,
        install_root: destination,
        receipt_digest: receipt.digest().map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::InstallFailed,
                error,
            )
        })?,
        adapter: spec.adapter,
        origin: rustclaw_skill_sdk::InstallOrigin::SourceBuild,
        reused: false,
        phases: vec![
            "preflight".to_string(),
            "protocol_smoke".to_string(),
            "activate".to_string(),
        ],
    })
}

fn remove_skill_store_package(
    state: &AppState,
    spec: &SkillStoreInstallSpec,
) -> SkillStoreOperationResult<bool> {
    rustclaw_skill_sdk::InstallReceiptStore::new(skill_package_root(state))
        .remove_installed_versions(&spec.skill_name)
        .map_err(|error| {
            SkillStoreOperationError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                SkillStoreErrorCode::PackageRemoveFailed,
                format!("skill={} error={error}", spec.skill_name),
            )
        })
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
