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
    if let Some(parent) = active_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&active_path, raw)?;

    let workspace_default = state.skill_rt.workspace_root.join("configs/config.toml");
    if active_path == workspace_default {
        let mounted_path = state
            .skill_rt
            .workspace_root
            .join("docker/config/config.toml");
        if let Some(parent) = mounted_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(mounted_path, raw)?;
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
    metadata: Option<Value>,
}

#[derive(Debug)]
struct ImportedSkillPlan {
    canonical_name: String,
    display_name: String,
    description: String,
    external_kind: String,
    aliases: Vec<String>,
    registry_prompt_rel_path: String,
    prompt_body_rel_path: String,
    bundle_rel_dir: String,
    entry_file: String,
    runtime: Option<String>,
    require_bins: Vec<String>,
    require_py_modules: Vec<String>,
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

fn slugify_skill_name(input: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = false;
    for ch in input.trim().chars() {
        let mapped = if ch.is_ascii_alphanumeric() {
            ch.to_ascii_lowercase()
        } else {
            '_'
        };
        if mapped == '_' {
            if out.is_empty() || last_was_sep {
                continue;
            }
            last_was_sep = true;
            out.push('_');
        } else {
            last_was_sep = false;
            out.push(mapped);
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "external_skill".to_string()
    } else if out.chars().next().unwrap_or('a').is_ascii_digit() {
        format!("ext_{out}")
    } else {
        out
    }
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
            "metadata" => {
                if let Ok(meta) = serde_json::from_str::<Value>(value) {
                    parsed.metadata = Some(meta);
                }
            }
            _ => {}
        }
    }
    parsed
}

fn scan_bundle_files(root: &Path, current: &Path, acc: &mut Vec<String>) -> std::io::Result<()> {
    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_bundle_files(root, &path, acc)?;
            continue;
        }
        if file_type.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            acc.push(rel);
        }
    }
    Ok(())
}

fn extract_required_bins(metadata: Option<&Value>) -> Vec<String> {
    let mut bins = Vec::new();
    let sources = [
        metadata,
        metadata.and_then(|m| m.get("openclaw")),
        metadata
            .and_then(|m| m.get("openclaw"))
            .and_then(|m| m.get("requires")),
    ];
    for source in sources.into_iter().flatten() {
        if let Some(arr) = source.get("bins").and_then(|v| v.as_array()) {
            for item in arr.iter().filter_map(|v| v.as_str()) {
                let item = item.trim();
                if !item.is_empty() && !bins.iter().any(|existing| existing == item) {
                    bins.push(item.to_string());
                }
            }
        }
    }
    bins
}

fn infer_python_modules(script_path: &Path) -> Vec<String> {
    let mut modules = Vec::new();
    let Ok(raw) = std::fs::read_to_string(script_path) else {
        return modules;
    };
    for line in raw.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("import ") {
            for item in rest.split(',') {
                let name = item
                    .split_whitespace()
                    .next()
                    .unwrap_or("")
                    .split('.')
                    .next()
                    .unwrap_or("")
                    .trim();
                if name == "akshare" && !modules.iter().any(|m| m == name) {
                    modules.push(name.to_string());
                }
            }
        } else if let Some(rest) = trimmed.strip_prefix("from ") {
            let name = rest
                .split_whitespace()
                .next()
                .unwrap_or("")
                .split('.')
                .next()
                .unwrap_or("")
                .trim();
            if name == "akshare" && !modules.iter().any(|m| m == name) {
                modules.push(name.to_string());
            }
        }
    }
    modules
}

fn detect_import_plan(
    skill_md: &str,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
) -> anyhow::Result<ImportedSkillPlan> {
    let frontmatter = parse_skill_frontmatter(skill_md);
    let mut files = Vec::new();
    scan_bundle_files(bundle_dir, bundle_dir, &mut files)?;
    files.sort();

    let display_name = if !frontmatter.name.trim().is_empty() {
        frontmatter.name.trim().to_string()
    } else {
        bundle_dir
            .file_name()
            .and_then(|v| v.to_str())
            .unwrap_or("external-skill")
            .to_string()
    };
    let canonical_name = slugify_skill_name(&display_name);
    let aliases = imported_skill_machine_alias(&display_name, &canonical_name)
        .into_iter()
        .collect();

    let mut require_bins = extract_required_bins(frontmatter.metadata.as_ref());
    let mut require_py_modules = Vec::new();
    let mut external_kind = "prompt_bundle".to_string();
    let mut entry_file = "SKILL.md".to_string();
    let mut runtime = None;

    let first_python = files.iter().find(|path| path.ends_with(".py")).cloned();
    let first_node = files
        .iter()
        .find(|path| path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".cjs"))
        .cloned();
    if let Some(py_entry) = first_python {
        external_kind = "local_script".to_string();
        entry_file = py_entry.clone();
        runtime = Some("python3".to_string());
        if !require_bins.iter().any(|item| item == "python3") {
            require_bins.push("python3".to_string());
        }
        require_py_modules = infer_python_modules(&bundle_dir.join(&py_entry));
    } else if let Some(node_entry) = first_node {
        external_kind = "local_script".to_string();
        entry_file = node_entry;
        runtime = Some("node".to_string());
        if !require_bins.iter().any(|item| item == "node") {
            require_bins.push("node".to_string());
        }
    } else if skill_md.contains("```bash")
        || skill_md.contains("```sh")
        || !require_bins.is_empty()
        || skill_md.contains("curl ")
        || skill_md.contains("jq ")
    {
        external_kind = "local_shell_recipe".to_string();
    }

    let description = if !frontmatter.description.trim().is_empty() {
        frontmatter.description.trim().to_string()
    } else {
        "Imported external skill".to_string()
    };
    let registry_prompt_rel_path = format!("prompts/skills/{canonical_name}.md");
    let prompt_body_rel_path = format!("prompts/layers/generated/skills/{canonical_name}.md");
    Ok(ImportedSkillPlan {
        canonical_name,
        display_name,
        description,
        external_kind,
        aliases,
        registry_prompt_rel_path,
        prompt_body_rel_path,
        bundle_rel_dir: bundle_rel_dir.to_string(),
        entry_file,
        runtime,
        require_bins,
        require_py_modules,
        source_url: source.to_string(),
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
    lines.push(format!("aliases = {}", render_string_array(&plan.aliases)));
    lines.push("timeout_seconds = 60".to_string());
    lines.push(format!("prompt_file = {:?}", plan.registry_prompt_rel_path));
    lines.push("output_kind = \"text\"".to_string());
    lines.push(format!("external_kind = {:?}", plan.external_kind));
    lines.push(format!("external_bundle_dir = {:?}", plan.bundle_rel_dir));
    lines.push(format!("external_entry_file = {:?}", plan.entry_file));
    if let Some(runtime) = &plan.runtime {
        lines.push(format!("external_runtime = {:?}", runtime));
    }
    lines.push(format!(
        "external_require_bins = {}",
        render_string_array(&plan.require_bins)
    ));
    lines.push(format!(
        "external_require_py_modules = {}",
        render_string_array(&plan.require_py_modules)
    ));
    lines.push(format!("external_source_url = {:?}", plan.source_url));
    lines.join("\n")
}

fn render_imported_skill_prompt(plan: &ImportedSkillPlan, skill_md: &str) -> String {
    let normalized_skill_md = skill_md.trim();
    let mut out = String::new();
    out.push_str("<!-- AUTO-GENERATED: external skill importer -->\n");
    out.push_str(&format!("# {}\n\n", plan.display_name));
    out.push_str("RustClaw imported external skill wrapper.\n\n");
    out.push_str("## RustClaw Wrapper\n");
    out.push_str(&format!(
        "- This is an imported external skill: `{}`.\n",
        plan.display_name
    ));
    out.push_str(&format!("- Description: {}\n", plan.description));
    out.push_str(&format!("- Runtime mode: `{}`\n", plan.external_kind));
    out.push_str(&format!("- Bundle directory: `{}`\n", plan.bundle_rel_dir));
    out.push_str(&format!("- Entry file: `{}`\n", plan.entry_file));
    if let Some(runtime) = &plan.runtime {
        out.push_str(&format!("- Runtime binary: `{runtime}`\n"));
    }
    if !plan.require_bins.is_empty() {
        out.push_str(&format!(
            "- Required local commands: {}\n",
            plan.require_bins.join(", ")
        ));
    }
    if !plan.require_py_modules.is_empty() {
        out.push_str(&format!(
            "- Required Python packages: {}\n",
            plan.require_py_modules.join(", ")
        ));
    }
    out.push_str(&format!("- Source: `{}`\n", plan.source_url));
    out.push_str("\n## Calling Rules\n");
    out.push_str("- Prefer the original `SKILL.md` below over your own guesses.\n");
    out.push_str(
        "- Follow the documented commands, options, examples, and parameter names from the original `SKILL.md` exactly.\n",
    );
    out.push_str(
        "- Do not invent unsupported CLI flags, JSON fields, shell fragments, or action names that are not grounded in the original `SKILL.md` or the entry file.\n",
    );
    match plan.external_kind.as_str() {
        "local_script" => {
            out.push_str(
                "- This skill runs a local script. Stay close to the script's real supported options and examples from the original `SKILL.md`.\n",
            );
            out.push_str(
                "- If the original `SKILL.md` shows a concrete command example, mirror that option shape instead of inventing a higher-level parameter.\n",
            );
        }
        "local_shell_recipe" => {
            out.push_str("- This skill runs shell recipes inside its bundle directory.\n");
            out.push_str(
                "- Keep the command close to the examples shown in the original `SKILL.md`.\n",
            );
            out.push_str(
                "- Prefer short, explicit commands. Reuse the documented recipes instead of inventing unrelated shell pipelines.\n",
            );
        }
        _ => {
            out.push_str(
                "- This prompt file lets the imported skill appear in RustClaw immediately.\n",
            );
            out.push_str(
                "- Runtime execution may still require a dedicated executor for this external kind.\n",
            );
        }
    }
    out.push_str(
        "- Avoid adding internal metadata fields yourself; RustClaw will inject its own runtime context.\n",
    );
    if !normalized_skill_md.is_empty() {
        out.push_str("\n## Original SKILL.md\n\n");
        out.push_str(normalized_skill_md);
        out.push('\n');
    }
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

fn guess_bundle_name_from_path_or_source(source: &str, fallback: &str) -> String {
    let source_hint = Path::new(source);
    let mut raw_name = source_hint
        .file_name()
        .and_then(|v| v.to_str())
        .map(|v| v.trim_end_matches(".md"))
        .map(|v| v.trim_end_matches(".git"))
        .filter(|v| !v.is_empty())
        .unwrap_or(fallback)
        .to_string();
    if raw_name.eq_ignore_ascii_case("skill") {
        if let Some(parent_name) = source_hint
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|v| v.to_str())
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            raw_name = parent_name.to_string();
        }
    }
    raw_name
}

fn finalize_imported_bundle(
    state: &AppState,
    bundle_dir: &Path,
    bundle_rel_dir: &str,
    source: &str,
    enabled: bool,
    skill_md: &str,
) -> (StatusCode, Json<ApiResponse<Value>>) {
    let plan = match detect_import_plan(skill_md, bundle_dir, bundle_rel_dir, source, enabled) {
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
    if let Some(parent) = prompt_body_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("create prompt directory failed: {err}")),
                }),
            );
        }
    }
    if let Err(err) = std::fs::write(
        &prompt_body_path,
        render_imported_skill_prompt(&plan, skill_md),
    ) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write prompt file failed: {err}")),
            }),
        );
    }

    let registry_raw = match read_skills_registry_file(state) {
        Ok(raw) => raw,
        Err(err) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiResponse {
                    ok: false,
                    data: None,
                    error: Some(format!("read skills registry failed: {err}")),
                }),
            );
        }
    };
    let (mut registry_raw, _) = remove_skill_registry_block(&registry_raw, &plan.canonical_name);
    if !registry_raw.ends_with('\n') && !registry_raw.is_empty() {
        registry_raw.push('\n');
    }
    registry_raw.push('\n');
    registry_raw.push_str(&render_imported_skill_registry_block(&plan));
    registry_raw.push('\n');
    if let Err(err) = write_skills_registry_file(state, &registry_raw) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiResponse {
                ok: false,
                data: None,
                error: Some(format!("write skills registry failed: {err}")),
            }),
        );
    }

    let installation = match update_skill_store_installation(state, &plan.canonical_name, true) {
        Ok(result) => result,
        Err(error) => return skill_store_error_response(error),
    };

    (
        StatusCode::OK,
        Json(ApiResponse {
            ok: true,
            data: Some(json!({
                "skill_name": plan.canonical_name,
                "display_name": plan.display_name,
                "description": plan.description,
                "external_kind": plan.external_kind,
                "bundle_dir": plan.bundle_rel_dir,
                "entry_file": plan.entry_file,
                "runtime": plan.runtime,
                "require_bins": plan.require_bins,
                "require_py_modules": plan.require_py_modules,
                "prompt_file": plan.registry_prompt_rel_path,
                "source": plan.source_url,
                "reload": installation.get("reload").cloned(),
                "installed": true,
                "enabled": true
            })),
            error: None,
        }),
    )
}

async fn materialize_import_source(
    state: &AppState,
    source: &str,
    dest_dir: &Path,
) -> Result<String, String> {
    let normalized = normalize_remote_skill_source(source);
    let src_path = Path::new(&normalized);
    if src_path.exists() {
        if src_path.is_dir() {
            copy_dir_recursive(src_path, dest_dir)
                .map_err(|err| format!("copy local bundle failed: {err}"))?;
            let skill_md = dest_dir.join("SKILL.md");
            return std::fs::read_to_string(&skill_md)
                .map_err(|err| format!("read copied SKILL.md failed: {err}"));
        }
        if src_path.is_file() {
            std::fs::create_dir_all(dest_dir)
                .map_err(|err| format!("create import dir failed: {err}"))?;
            std::fs::copy(src_path, dest_dir.join("SKILL.md"))
                .map_err(|err| format!("copy local SKILL.md failed: {err}"))?;
            return std::fs::read_to_string(dest_dir.join("SKILL.md"))
                .map_err(|err| format!("read copied SKILL.md failed: {err}"));
        }
    }

    let res = state
        .core
        .http_client
        .get(&normalized)
        .send()
        .await
        .map_err(|err| format!("download skill source failed: {err}"))?;
    let status = res.status();
    let body = res
        .text()
        .await
        .map_err(|err| format!("read skill source body failed: {err}"))?;
    if !status.is_success() {
        return Err(format!(
            "download skill source returned {status}: {}",
            body.chars().take(200).collect::<String>()
        ));
    }
    std::fs::create_dir_all(dest_dir).map_err(|err| format!("create import dir failed: {err}"))?;
    std::fs::write(dest_dir.join("SKILL.md"), &body)
        .map_err(|err| format!("write downloaded SKILL.md failed: {err}"))?;
    Ok(body)
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
