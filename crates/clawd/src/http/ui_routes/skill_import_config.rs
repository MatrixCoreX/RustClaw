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
    let persisted_path = std::env::var_os("RUSTCLAW_CONFIG_PERSIST_PATH")
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

fn read_skills_registry_file(state: &AppState) -> std::io::Result<String> {
    let path = state
        .skill_rt
        .workspace_root
        .join("configs/skills_registry.toml");
    match std::fs::read_to_string(path) {
        Ok(raw) => Ok(raw),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(err) => Err(err),
    }
}

fn write_skills_registry_file(state: &AppState, raw: &str) -> std::io::Result<()> {
    let active_path = state
        .skill_rt
        .workspace_root
        .join("configs/skills_registry.toml");
    if let Some(parent) = active_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&active_path, raw)?;

    let mounted_path = state
        .skill_rt
        .workspace_root
        .join("docker/config/skills_registry.toml");
    if let Some(parent) = mounted_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&mounted_path, raw)?;
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
    registry_prompt_rel_path: String,
    prompt_body_rel_path: String,
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
    let mut manifest = rustclaw_skill_sdk::PackageManifest::load(&manifest_path)
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
    let registry_prompt_rel_path = format!("prompts/skills/{canonical_name}.md");
    let prompt_body_rel_path = format!("prompts/layers/generated/skills/{canonical_name}.md");
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
        registry_prompt_rel_path,
        prompt_body_rel_path,
        bundle_rel_dir: bundle_rel_dir.to_string(),
        entry_file: manifest.run.entrypoint,
        source_url: manifest.package.source.unwrap_or_else(|| source.to_string()),
        enabled,
    })
}

fn render_string_array(items: &[String]) -> String {
    if items.is_empty() {
        "[]".to_string()
    } else {
        let body = items
            .iter()
            .map(|item| format!("{item:?}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("[{body}]")
    }
}

fn render_imported_skill_registry_block(plan: &ImportedSkillPlan) -> String {
    let mut lines = Vec::new();
    lines.push("[[skills]]".to_string());
    lines.push(format!("name = {:?}", plan.canonical_name));
    lines.push(format!("enabled = {}", plan.enabled));
    lines.push("kind = \"external\"".to_string());
    lines.push("install_mode = \"on_demand\"".to_string());
    lines.push(format!(
        "package_manifest = {:?}",
        plan.package_manifest_rel_path
    ));
    lines.push(format!("aliases = {}", render_string_array(&plan.aliases)));
    lines.push("timeout_seconds = 60".to_string());
    lines.push(format!("prompt_file = {:?}", plan.registry_prompt_rel_path));
    lines.push("output_kind = \"text\"".to_string());
    lines.push(format!(
        "supported_os = {}",
        render_string_array(&plan.supported_os)
    ));
    lines.push(format!("description = {:?}", plan.description));
    lines.push("risk_level = \"high\"".to_string());
    lines.push("auto_invocable = false".to_string());
    lines.push("requires_confirmation = true".to_string());
    lines.push("side_effect = true".to_string());
    lines.join("\n")
}

fn render_imported_skill_prompt(plan: &ImportedSkillPlan, interface_md: &str) -> String {
    let normalized_interface = interface_md.trim();
    let mut out = String::new();
    out.push_str("<!-- AUTO-GENERATED: external skill importer -->\n");
    out.push_str(&format!("# {}\n\n", plan.display_name));
    out.push_str("RustClaw verified external skill package.\n\n");
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
        "- Avoid adding internal metadata fields yourself; RustClaw will inject its own runtime context.\n",
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

fn remove_managed_prompt_file(path: &Path) -> std::io::Result<bool> {
    let raw = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(err) => return Err(err),
    };
    if raw.contains("<!-- AUTO-GENERATED: external skill importer -->") {
        std::fs::remove_file(path)?;
        return Ok(true);
    }
    Ok(false)
}

fn remove_runtime_skill_state(raw: &str, state: &AppState, skill_name: &str) -> String {
    let parsed = toml::from_str::<toml::Value>(raw)
        .unwrap_or_else(|_| toml::Value::Table(Default::default()));
    let mut switches = collect_skill_switches(&parsed, state);
    switches.remove(skill_name);
    let mut uninstalled = collect_uninstalled_skills(&parsed, state);
    uninstalled.remove(skill_name);
    let rendered = render_switches_inline_table(&switches);
    let updated = upsert_skill_switches_line(raw, &rendered);
    upsert_section_key_line(
        &updated,
        "skills",
        "uninstalled_skills",
        &render_skill_name_array(&uninstalled),
    )
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
struct ImportedRegistrationSnapshot {
    prompt: Option<Vec<u8>>,
    registry: String,
    runtime_config: String,
    current_pointer: Option<rustclaw_skill_sdk::CurrentInstallPointer>,
}

fn read_optional_file(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn restore_optional_file(path: &Path, previous: Option<&[u8]>) -> std::io::Result<()> {
    match previous {
        Some(value) => {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, value)
        }
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        },
    }
}

fn rollback_imported_registration(
    state: &AppState,
    plan: &ImportedSkillPlan,
    snapshot: &ImportedRegistrationSnapshot,
) -> Vec<String> {
    let mut failures = Vec::new();
    let prompt_path = state
        .skill_rt
        .workspace_root
        .join(&plan.prompt_body_rel_path);
    if let Err(error) = restore_optional_file(&prompt_path, snapshot.prompt.as_deref()) {
        failures.push(format!("prompt={error}"));
    }
    if let Err(error) = write_skills_registry_file(state, &snapshot.registry) {
        failures.push(format!("registry={error}"));
    }
    if let Err(error) = write_runtime_config_file(state, &snapshot.runtime_config) {
        failures.push(format!("config={error}"));
    }
    let receipt_store =
        rustclaw_skill_sdk::InstallReceiptStore::new(skill_package_root(state));
    let current = receipt_store.current_pointer(&plan.canonical_name).ok();
    match (&snapshot.current_pointer, current) {
        (Some(previous), Some(current)) if current != *previous => {
            if let Err(error) = receipt_store.rollback(&plan.canonical_name) {
                failures.push(format!("receipt={error}"));
            }
        }
        (None, Some(_)) => {
            if let Err(error) = receipt_store.remove_installed_versions(&plan.canonical_name) {
                failures.push(format!("receipt={error}"));
            }
        }
        _ => {}
    }
    if let Err(error) = reload_skill_views(state) {
        failures.push(format!("reload={error}"));
    }
    failures
}

fn imported_finalize_failure(
    state: &AppState,
    plan: &ImportedSkillPlan,
    snapshot: &ImportedRegistrationSnapshot,
    status: StatusCode,
    message: impl std::fmt::Display,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let rollback_failures = rollback_imported_registration(state, plan, snapshot);
    let rollback_suffix = if rollback_failures.is_empty() {
        String::new()
    } else {
        format!("; rollback failures: {}", rollback_failures.join(", "))
    };
    (
        status,
        Json(ApiResponse {
            ok: false,
            data: None,
            error: Some(format!("{message}{rollback_suffix}")),
        }),
    )
}

#[derive(Debug)]
struct ImportedBundleActivation {
    bundle_dir: PathBuf,
    bundle_rel_dir: String,
    backup_dir: Option<PathBuf>,
}

fn imported_bundle_staging_dir(workspace_root: &Path) -> std::io::Result<PathBuf> {
    let root = workspace_root.join("third_party/clawhub");
    std::fs::create_dir_all(&root)?;
    let staging = root.join(format!(".staging-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&staging)?;
    Ok(staging)
}

fn activate_imported_bundle(
    workspace_root: &Path,
    staging_dir: &Path,
) -> Result<ImportedBundleActivation, String> {
    let manifest = rustclaw_skill_sdk::PackageManifest::load(&staging_dir.join("skill.toml"))
        .map_err(|error| format!("validate staged skill manifest failed: {error}"))?;
    let canonical_name = manifest.package.name;
    let bundle_rel_dir = format!("third_party/clawhub/{canonical_name}");
    let bundle_dir = workspace_root.join(&bundle_rel_dir);
    let backup_dir = bundle_dir.exists().then(|| {
        workspace_root.join(format!(
            "third_party/clawhub/.backup-{canonical_name}-{}",
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

    let prompt_body_path = state
        .skill_rt
        .workspace_root
        .join(&plan.prompt_body_rel_path);
    let registry_snapshot = match read_skills_registry_file(state) {
        Ok(raw) => raw,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills registry failed: {error}")),
                }),
            );
        }
    };
    let runtime_config_snapshot = match read_skill_config_file(state) {
        Ok((raw, _)) => raw,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read runtime config failed: {error}")),
                }),
            );
        }
    };
    let prompt_snapshot = match read_optional_file(&prompt_body_path) {
        Ok(value) => value,
        Err(error) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read prompt snapshot failed: {error}")),
                }),
            );
        }
    };
    let snapshot = ImportedRegistrationSnapshot {
        prompt: prompt_snapshot,
        registry: registry_snapshot,
        runtime_config: runtime_config_snapshot,
        current_pointer: rustclaw_skill_sdk::InstallReceiptStore::new(skill_package_root(state))
            .current_pointer(&plan.canonical_name)
            .ok(),
    };

    let install_request = rustclaw_skill_sdk::InstallRequest {
        manifest_path: bundle_dir.join("skill.toml"),
        workspace_root: state.skill_rt.workspace_root.clone(),
        package_root: skill_package_root(state),
        target: None,
        allow_network,
        control: None,
    };
    let install_outcome = match tokio::task::spawn_blocking(move || {
        rustclaw_skill_sdk::SkillInstaller.install(&install_request)
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
                        rustclaw_skill_sdk::redact_diagnostics(&error.detail)
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

    if let Some(parent) = prompt_body_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return imported_finalize_failure(
                state,
                &plan,
                &snapshot,
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("create prompt directory failed: {err}"),
            );
        }
    }
    if let Err(err) = std::fs::write(
        &prompt_body_path,
        render_imported_skill_prompt(&plan, interface_md),
    ) {
        return imported_finalize_failure(
            state,
            &plan,
            &snapshot,
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write prompt file failed: {err}"),
        );
    }

    let registry_raw = snapshot.registry.clone();
    let (mut registry_raw, _) = remove_skill_registry_block(&registry_raw, &plan.canonical_name);
    if !registry_raw.ends_with('\n') && !registry_raw.is_empty() {
        registry_raw.push('\n');
    }
    registry_raw.push('\n');
    registry_raw.push_str(&render_imported_skill_registry_block(&plan));
    registry_raw.push('\n');
    if let Err(err) = write_skills_registry_file(state, &registry_raw) {
        return imported_finalize_failure(
            state,
            &plan,
            &snapshot,
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("write skills registry failed: {err}"),
        );
    }

    let mut installation = match update_skill_store_installation(state, &plan.canonical_name, true) {
        Ok(result) => result,
        Err(error) => {
            let response = skill_store_error_response(error);
            let message = response
                .1
                .0
                .error
                .clone()
                .unwrap_or_else(|| "skill store configuration failed".to_string());
            return imported_finalize_failure(
                state,
                &plan,
                &snapshot,
                response.0,
                message,
            );
        }
    };
    if !plan.enabled {
        let (runtime_raw, parsed) = match read_skill_config_file(state) {
            Ok(value) => value,
            Err(error) => {
                return imported_finalize_failure(
                    state,
                    &plan,
                    &snapshot,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("read runtime config failed: {error}"),
                );
            }
        };
        let mut switches = collect_skill_switches(&parsed, state);
        switches.insert(plan.canonical_name.clone(), false);
        let mut uninstalled = collect_uninstalled_skills(&parsed, state);
        uninstalled.remove(&plan.canonical_name);
        let updated = render_skill_store_config(&runtime_raw, &switches, &uninstalled);
        if let Err(error) = write_runtime_config_file(state, &updated) {
            return imported_finalize_failure(
                state,
                &plan,
                &snapshot,
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("write disabled runtime config failed: {error}"),
            );
        }
        match reload_skill_views(state) {
            Ok(reload) => installation = json!({"reload": reload}),
            Err(error) => {
                return imported_finalize_failure(
                    state,
                    &plan,
                    &snapshot,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("reload disabled imported skill failed: {error}"),
                );
            }
        }
    }

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
                "prompt_file": plan.registry_prompt_rel_path,
                "source": plan.source_url,
                "reload": installation.get("reload").cloned(),
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
