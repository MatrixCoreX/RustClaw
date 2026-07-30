fn active_runtime_config_path(state: &AppState) -> PathBuf {
    let configured = state.reload_ctx.config_path_for_reload.trim();
    if configured.is_empty() {
        return state.skill_rt.workspace_root.join("configs/config.toml");
    }
    let path = Path::new(configured);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(path)
    }
}

fn read_skill_config_file(state: &AppState) -> anyhow::Result<(String, toml::Value)> {
    let path = active_runtime_config_path(state);
    let raw = std::fs::read_to_string(&path)?;
    let parsed = toml::from_str::<toml::Value>(&raw)?;
    Ok((raw, parsed))
}

fn write_workspace_and_mounted_file(
    workspace_root: &Path,
    relative_path: &str,
    raw: &str,
) -> std::io::Result<()> {
    let active_path = workspace_root.join(relative_path);
    if let Some(parent) = active_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&active_path, raw)?;

    let mounted_relative = relative_path
        .strip_prefix("configs/")
        .unwrap_or(relative_path);
    let mounted_path = workspace_root.join("docker/config").join(mounted_relative);
    if let Some(parent) = mounted_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mounted_path, raw)?;
    Ok(())
}

fn write_runtime_config_file(state: &AppState, raw: &str) -> std::io::Result<()> {
    let active_path = active_runtime_config_path(state);
    let persisted_path = claw_core::product_identity::env_os("CONFIG_PERSIST_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                state.skill_rt.workspace_root.join(path)
            }
        });
    write_runtime_config_to_paths(&active_path, persisted_path.as_deref(), raw)
}

fn write_runtime_config_to_paths(
    active_path: &Path,
    persisted_path: Option<&Path>,
    raw: &str,
) -> std::io::Result<()> {
    if let Some(parent) = active_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(active_path, raw)?;
    if let Some(path) = persisted_path.filter(|path| *path != active_path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, raw)?;
    }
    Ok(())
}

#[derive(Debug, Default)]
struct ParsedSkillFrontmatter {
    name: String,
    description: String,
}

#[derive(Debug)]
struct ImportedSkillPlan {
    canonical_name: String,
    display_name: String,
    description: String,
    build_adapter: String,
    launcher: String,
    package_version: String,
    package_manifest_rel_path: String,
    supported_os: Vec<String>,
    supported_arch: Vec<String>,
    aliases: Vec<String>,
    bundle_rel_dir: String,
    entry_file: String,
    source_url: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct UninstallExternalSkillRequest {
    skill_name: String,
}

fn normalize_remote_skill_source(source: &str) -> String {
    let trimmed = source.trim();
    if let Some(rest) = trimmed.strip_prefix("https://github.com/") {
        if let Some((repo_part, path_part)) = rest.split_once("/blob/") {
            if let Some((branch, file_path)) = path_part.split_once('/') {
                return format!(
                    "https://raw.githubusercontent.com/{repo_part}/{branch}/{file_path}"
                );
            }
        }
    }
    trimmed.to_string()
}

fn imported_skill_machine_alias(display_name: &str, canonical_name: &str) -> Option<String> {
    let alias = display_name.trim().to_ascii_lowercase();
    let is_machine_token = !alias.is_empty()
        && alias.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '_' | '.' | '-')
        });
    (is_machine_token && alias != canonical_name).then_some(alias)
}

fn parse_skill_frontmatter(skill_md: &str) -> ParsedSkillFrontmatter {
    let mut parsed = ParsedSkillFrontmatter::default();
    let mut lines = skill_md.lines();
    if lines.next().map(str::trim) != Some("---") {
        return parsed;
    }
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.is_empty() {
            continue;
        }
        let Some((key, value)) = trimmed.split_once(':') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim().trim_matches('"').trim_matches('\'');
        match key {
            "name" => parsed.name = value.to_string(),
            "description" => parsed.description = value.to_string(),
            _ => {}
        }
    }
    parsed
}

fn detect_import_plan(
    interface_md: &str,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
) -> anyhow::Result<ImportedSkillPlan> {
    let frontmatter = parse_skill_frontmatter(interface_md);
    let manifest_path = bundle_dir.join("skill.toml");
    let mut manifest = skill_sdk::PackageManifest::load(&manifest_path)
        .map_err(|error| anyhow::anyhow!("manifest validation failed: {error}"))?;
    let expected_source_root = Path::new(bundle_rel_dir);
    if manifest.build.source_root == "." {
        manifest.build.source_root = bundle_rel_dir.to_string();
        std::fs::write(&manifest_path, manifest.to_toml_string()?)?;
    } else if Path::new(&manifest.build.source_root) != expected_source_root {
        anyhow::bail!(
            "manifest build.source_root must be `.` or the workspace-relative package directory: expected={} actual={}",
            bundle_rel_dir,
            manifest.build.source_root
        );
    }
    if !bundle_dir.join("INTERFACE.md").is_file() {
        anyhow::bail!("manifest package is missing INTERFACE.md");
    }

    let display_name = if !frontmatter.name.trim().is_empty() {
        frontmatter.name.trim().to_string()
    } else {
        manifest.package.name.clone()
    };
    let canonical_name = manifest.package.name.clone();
    let aliases = imported_skill_machine_alias(&display_name, &canonical_name)
        .into_iter()
        .collect();

    let description = if !frontmatter.description.trim().is_empty() {
        frontmatter.description.trim().to_string()
    } else {
        manifest.package.description.clone()
    };
    Ok(ImportedSkillPlan {
        canonical_name,
        display_name,
        description,
        build_adapter: manifest.build.adapter.as_token().to_string(),
        launcher: format!("{:?}", manifest.run.launcher).to_ascii_lowercase(),
        package_version: manifest.package.version,
        package_manifest_rel_path: format!("{bundle_rel_dir}/skill.toml"),
        supported_os: manifest.package.supported_os,
        supported_arch: manifest.package.supported_arch,
        aliases,
        bundle_rel_dir: bundle_rel_dir.to_string(),
        entry_file: manifest.run.entrypoint,
        source_url: manifest.package.source.unwrap_or_else(|| source.to_string()),
        enabled,
    })
}

fn render_imported_skill_prompt(plan: &ImportedSkillPlan, interface_md: &str) -> String {
    let normalized_interface = interface_md.trim();
    let mut out = String::new();
    out.push_str("<!-- AUTO-GENERATED: external skill importer -->\n");
    out.push_str(&format!("# {}\n\n", plan.display_name));
    out.push_str("Agent runtime verified external skill package.\n\n");
    out.push_str("## Verified Package\n");
    out.push_str(&format!(
        "- This is an imported external skill: `{}`.\n",
        plan.display_name
    ));
    out.push_str(&format!("- Description: {}\n", plan.description));
    out.push_str(&format!("- Version: `{}`\n", plan.package_version));
    out.push_str(&format!("- Build adapter: `{}`\n", plan.build_adapter));
    out.push_str(&format!("- Launcher: `{}`\n", plan.launcher));
    out.push_str(&format!("- Manifest: `{}`\n", plan.package_manifest_rel_path));
    out.push_str(&format!("- Entry file: `{}`\n", plan.entry_file));
    out.push_str(&format!("- Source: `{}`\n", plan.source_url));
    out.push_str("\n## Calling Rules\n");
    out.push_str("- Treat the `INTERFACE.md` contract below as authoritative.\n");
    out.push_str(
        "- Follow its actions, parameter names, types, defaults, and response contract exactly.\n",
    );
    out.push_str(
        "- Do not infer command-line flags, runtimes, dependencies, or action names from source files.\n",
    );
    out.push_str(
        "- Avoid adding internal metadata fields yourself; the agent runtime will inject its own runtime context.\n",
    );
    if !normalized_interface.is_empty() {
        out.push_str("\n## Interface Contract\n\n");
        out.push_str(normalized_interface);
        out.push('\n');
    }
    out.push_str(
        "\n## Multilingual Reinforcement\n\n<!-- MULTILINGUAL-REINFORCEMENT: Keep language-specific nuance concise; preserve machine fields and action names exactly. -->\n",
    );
    out
}

fn admission_service(
    state: &AppState,
) -> Result<crate::skill_admission::SkillAdmissionService, String> {
    let config_path = active_runtime_config_path(state);
    let config = claw_core::config::AppConfig::load(&config_path.to_string_lossy())
        .map_err(|error| format!("load active config failed: {error}"))?;
    crate::skill_admission::SkillAdmissionService::from_config(
        &state.skill_rt.workspace_root,
        &config,
    )
    .map_err(|error| error.to_string())
}

fn project_admission_config_state(
    snapshot: &crate::skill_admission::OverlaySnapshot,
    switches: &mut BTreeMap<String, bool>,
    uninstalled: &mut BTreeSet<String>,
) {
    for name in &snapshot.enabled {
        switches.insert(name.clone(), true);
        uninstalled.remove(name);
    }
    for name in snapshot
        .disabled
        .iter()
        .chain(snapshot.awaiting_policy.iter())
    {
        switches.insert(name.clone(), false);
        uninstalled.remove(name);
    }
    for name in &snapshot.tombstoned {
        switches.insert(name.clone(), false);
        uninstalled.insert(name.clone());
    }
}

fn imported_host_policy_grant(
    manifest: &skill_sdk::PackageManifest,
) -> Result<skill_sdk::HostPolicyGrant, String> {
    use skill_sdk::{
        ApprovalSource, GrantedCapability, HostPolicyGrant, HostRiskLevel, RequestedEffect,
        HOST_POLICY_GRANT_SCHEMA_VERSION,
    };

    let request = manifest
        .effective_capability_request()
        .map_err(|error| error.to_string())?;
    let mutating = request.capabilities.iter().any(|capability| {
        matches!(
            capability.effect,
            RequestedEffect::Mutate | RequestedEffect::External
        )
    });
    let high_risk = request.permissions.filesystem_write
        || request.permissions.subprocess
        || request.permissions.package_install
        || request.permissions.privilege_escalation
        || request.permissions.external_publish
        || mutating;
    let medium_risk = request.permissions.network
        || request.permissions.filesystem_read
        || request.permissions.llm_gateway
        || !request.permissions.credential_refs.is_empty();
    let risk_level = if high_risk {
        HostRiskLevel::High
    } else if medium_risk {
        HostRiskLevel::Medium
    } else {
        HostRiskLevel::Low
    };
    let grant = HostPolicyGrant {
        schema_version: HOST_POLICY_GRANT_SCHEMA_VERSION,
        skill_name: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        semantic_contract_digest: manifest
            .capability_request_digest()
            .map_err(|error| error.to_string())?,
        capabilities: request
            .capabilities
            .iter()
            .map(|capability| GrantedCapability {
                name: capability.name.clone(),
                action: capability.action.clone(),
            })
            .collect(),
        permissions: request.permissions,
        risk_level,
        auto_invocable: false,
        requires_confirmation: high_risk,
        approval_source: ApprovalSource::AdminApi,
        approved_at_unix: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            .max(1),
    };
    grant
        .validate_against(manifest)
        .map_err(|error| error.to_string())?;
    Ok(grant)
}

fn restore_import_install_pointer(
    state: &AppState,
    skill_name: &str,
    previous: Option<&skill_sdk::CurrentInstallPointer>,
) -> Result<(), String> {
    let store = skill_sdk::InstallReceiptStore::new(skill_package_root(state));
    match previous {
        Some(previous) => {
            let current = store
                .current_pointer(skill_name)
                .map_err(|error| error.to_string())?;
            if current != *previous {
                store
                    .rollback(skill_name)
                    .map_err(|error| error.to_string())?;
            }
            Ok(())
        }
        None => store
            .remove_installed_versions(skill_name)
            .map(|_| ())
            .map_err(|error| error.to_string()),
    }
}

#[cfg(test)]
fn parse_registry_block_name(block: &[&str]) -> Option<String> {
    for line in block {
        let trimmed = line.trim();
        if !trimmed.starts_with("name") {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once('=') else {
            continue;
        };
        if lhs.trim() != "name" {
            continue;
        }
        let rhs = rhs.trim();
        let parsed = toml::from_str::<toml::Value>(&format!("value = {rhs}")).ok()?;
        let value = parsed.get("value")?.as_str()?.trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

#[cfg(test)]
fn remove_skill_registry_block(raw: &str, skill_name: &str) -> (String, bool) {
    let mut out: Vec<String> = Vec::new();
    let lines: Vec<&str> = raw.lines().collect();
    let mut idx = 0usize;
    let mut removed = false;
    while idx < lines.len() {
        if lines[idx].trim() != "[[skills]]" {
            out.push(lines[idx].to_string());
            idx += 1;
            continue;
        }
        let start = idx;
        idx += 1;
        while idx < lines.len() && lines[idx].trim() != "[[skills]]" {
            idx += 1;
        }
        let block = &lines[start..idx];
        let block_name = parse_registry_block_name(block)
            .map(|name| name.to_ascii_lowercase())
            .unwrap_or_default();
        if block_name == skill_name {
            removed = true;
            continue;
        }
        out.extend(block.iter().map(|line| (*line).to_string()));
    }
    let mut rendered = out.join("\n");
    if raw.ends_with('\n') {
        rendered.push('\n');
    }
    (rendered, removed)
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dst.join(entry.file_name());
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            copy_dir_recursive(&path, &target)?;
        } else if file_type.is_file() {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}

fn sanitize_upload_relative_path(input: &str) -> Option<PathBuf> {
    let trimmed = input.trim().replace('\\', "/");
    if trimmed.is_empty() {
        return None;
    }
    let path = Path::new(&trimmed);
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(part) => out.push(part),
            std::path::Component::CurDir => {}
            _ => return None,
        }
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

#[derive(Debug)]
struct ImportedBundleActivation {
    bundle_dir: PathBuf,
    bundle_rel_dir: String,
    backup_dir: Option<PathBuf>,
}

fn imported_bundle_staging_dir(workspace_root: &Path) -> std::io::Result<PathBuf> {
    let root = workspace_root.join("data/skills/imports");
    std::fs::create_dir_all(&root)?;
    let staging = root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging)?;
    Ok(staging)
}

fn activate_imported_bundle(
    workspace_root: &Path,
    staging_dir: &Path,
) -> Result<ImportedBundleActivation, String> {
    let manifest = skill_sdk::PackageManifest::load(&staging_dir.join("skill.toml"))
        .map_err(|error| format!("validate staged skill manifest failed: {error}"))?;
    let canonical_name = manifest.package.name;
    let bundle_rel_dir = format!("data/skills/imports/{canonical_name}");
    let bundle_dir = workspace_root.join(&bundle_rel_dir);
    let backup_dir = bundle_dir.exists().then(|| {
        workspace_root.join(format!(
            "data/skills/imports/.backup-{canonical_name}-{}",
            uuid::Uuid::new_v4()
        ))
    });
    if let Some(backup) = &backup_dir {
        std::fs::rename(&bundle_dir, backup)
            .map_err(|error| format!("backup previous imported bundle failed: {error}"))?;
    }
    if let Err(error) = std::fs::rename(staging_dir, &bundle_dir) {
        if let Some(backup) = &backup_dir {
            let _ = std::fs::rename(backup, &bundle_dir);
        }
        return Err(format!("activate staged imported bundle failed: {error}"));
    }
    Ok(ImportedBundleActivation {
        bundle_dir,
        bundle_rel_dir,
        backup_dir,
    })
}

fn finish_imported_bundle_activation(
    activation: &ImportedBundleActivation,
    success: bool,
) -> std::io::Result<()> {
    if success {
        if let Some(backup) = &activation.backup_dir {
            std::fs::remove_dir_all(backup)?;
        }
        return Ok(());
    }
    if activation.bundle_dir.exists() {
        std::fs::remove_dir_all(&activation.bundle_dir)?;
    }
    if let Some(backup) = &activation.backup_dir {
        std::fs::rename(backup, &activation.bundle_dir)?;
    }
    Ok(())
}

async fn finalize_imported_bundle(
    state: &AppState,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
    allow_network: bool,
    interface_md: &str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let plan = match detect_import_plan(interface_md, bundle_dir, bundle_rel_dir, source, enabled) {
        Ok(plan) => plan,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("analyze imported skill failed: {err}")),
                }),
            );
        }
    };

    let receipt_store = skill_sdk::InstallReceiptStore::new(skill_package_root(state));
    let previous_pointer = receipt_store.current_pointer(&plan.canonical_name).ok();

    let install_request = skill_sdk::InstallRequest {
        manifest_path: bundle_dir.join("skill.toml"),
        workspace_root: state.skill_rt.workspace_root.clone(),
        package_root: skill_package_root(state),
        target: None,
        allow_network,
        control: None,
    };
    let install_outcome = match tokio::task::spawn_blocking(move || {
        skill_sdk::SkillInstaller.install(&install_request)
    })
    .await
    {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(error)) => {
            return (
                StatusCode::CONFLICT,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!(
                        "skill package verification failed: code={} phase={} diagnostic={}",
                        error.code,
                        error.phase.as_deref().unwrap_or("unknown"),
                        skill_sdk::redact_diagnostics(&error.detail)
                    )),
                }),
            );
        }
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("skill package installer task failed: {error}")),
                }),
            );
        }
    };

    let verified = match receipt_store.verified_current_install(&plan.canonical_name) {
        Ok(verified) => verified,
        Err(error) => {
            let _ = restore_import_install_pointer(
                state,
                &plan.canonical_name,
                previous_pointer.as_ref(),
            );
            return (
                StatusCode::CONFLICT,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("verify installed receipt failed: {error}")),
                }),
            );
        }
    };
    let grant = match imported_host_policy_grant(&verified.manifest) {
        Ok(grant) => grant,
        Err(error) => {
            let _ = restore_import_install_pointer(
                state,
                &plan.canonical_name,
                previous_pointer.as_ref(),
            );
            return (
                StatusCode::CONFLICT,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("host policy grant failed: {error}")),
                }),
            );
        }
    };
    let service = match admission_service(state) {
        Ok(service) => service,
        Err(error) => {
            let _ = restore_import_install_pointer(
                state,
                &plan.canonical_name,
                previous_pointer.as_ref(),
            );
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("initialize skill admission failed: {error}")),
                }),
            );
        }
    };
    let admission = match service.admit_external(crate::skill_admission::AdmissionMutation {
        metadata: crate::skill_admission::ExternalSkillMetadata {
            name: plan.canonical_name.clone(),
            source: crate::skill_admission::SkillAdmissionSource::ExternalOverlay,
            package_manifest_path: plan.package_manifest_rel_path.clone(),
            description: plan.description.clone(),
            aliases: plan.aliases.clone(),
            group: "extensions".to_string(),
        },
        prompt: render_imported_skill_prompt(&plan, interface_md),
        state: if plan.enabled {
            skill_sdk::AdmissionState::Enabled
        } else {
            skill_sdk::AdmissionState::InstalledDisabled
        },
        grant: Some(grant.clone()),
    }) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let rollback = restore_import_install_pointer(
                state,
                &plan.canonical_name,
                previous_pointer.as_ref(),
            )
            .err()
            .map(|rollback| format!("; receipt rollback failed: {rollback}"))
            .unwrap_or_default();
            return (
                StatusCode::CONFLICT,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("skill admission failed: {error}{rollback}")),
                }),
            );
        }
    };
    let reload = match reload_skill_views(state) {
        Ok(reload) => reload,
        Err(error) => {
            let receipt_rollback = restore_import_install_pointer(
                state,
                &plan.canonical_name,
                previous_pointer.as_ref(),
            );
            let generation_rollback = service.rollback_generation(admission.generation);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!(
                        "reload admitted skill failed: {error}; receipt_rollback={receipt_rollback:?}; generation_rollback={generation_rollback:?}"
                    )),
                }),
            );
        }
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_name": plan.canonical_name,
                "display_name": plan.display_name,
                "description": plan.description,
                "build_adapter": plan.build_adapter,
                "launcher": plan.launcher,
                "package_version": plan.package_version,
                "receipt_digest": install_outcome.receipt_digest,
                "install_reused": install_outcome.reused,
                "bundle_dir": plan.bundle_rel_dir,
                "entry_file": plan.entry_file,
                "supported_os": plan.supported_os,
                "supported_arch": plan.supported_arch,
                "prompt_file": "runtime_overlay",
                "source": plan.source_url,
                "registry_generation": admission.generation,
                "registry_generation_digest": admission.generation_digest,
                "policy_grant_digest": grant.digest(&verified.manifest).ok(),
                "reload": reload,
                "installed": true,
                "enabled": plan.enabled
            })),
            error: None,
        }),
    )
}

async fn materialize_import_source(
    source: &str,
    dest_dir: &Path,
) -> Result<String, String> {
    let normalized = normalize_remote_skill_source(source);
    let src_path = Path::new(&normalized);
    if src_path.exists() {
        if src_path.is_dir() {
            copy_dir_recursive(src_path, dest_dir)
                .map_err(|err| format!("copy local bundle failed: {err}"))?;
            let interface_md = dest_dir.join("INTERFACE.md");
            return std::fs::read_to_string(&interface_md)
                .map_err(|err| format!("read copied INTERFACE.md failed: {err}"));
        }
        if src_path.is_file() {
            return Err(
                "skill source must be a canonical package directory containing skill.toml and INTERFACE.md"
                    .to_string(),
            );
        }
    }
    Err(
        "remote single-file imports are unsupported; upload a canonical package bundle containing skill.toml and INTERFACE.md"
            .to_string(),
    )
}

fn upsert_string_key_in_section(
    raw: &str,
    section_name: &str,
    key: &str,
    rendered_line: &str,
) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let section_header = format!("[{section_name}]");
    let mut in_section = false;
    let mut section_seen = false;
    let mut inserted_or_replaced = false;
    let mut insert_index_in_section: Option<usize> = None;
    let mut section_end: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == section_header {
            in_section = true;
            section_seen = true;
            insert_index_in_section = Some(idx + 1);
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != section_header {
            if in_section {
                section_end = Some(idx);
                break;
            }
            continue;
        }
        if in_section && trimmed.starts_with(key) && trimmed.contains('=') {
            lines[idx] = rendered_line.to_string();
            inserted_or_replaced = true;
            break;
        }
    }

    if !inserted_or_replaced && section_seen {
        let idx = insert_index_in_section
            .or(section_end)
            .unwrap_or(lines.len());
        lines.insert(idx, rendered_line.to_string());
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}
