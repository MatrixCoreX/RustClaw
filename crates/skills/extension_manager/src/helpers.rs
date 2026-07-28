use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use rustclaw_skill_sdk::{
    InstallOutcome, InstallReceiptStore, InstallRequest, PackageManifest, SkillInstaller,
    SkillRuntimeResolver,
};
use serde_json::{json, Map, Value};

use super::{
    CommandRunRecord, ExternalSkillEnableReport, ExternalSkillImplementation,
    ExternalSkillRegistrationReport, ExternalSkillValidationReport, TemporaryFixPlan,
};

pub(crate) fn write_plan_files(
    workspace_root: &Path,
    plan: &TemporaryFixPlan,
) -> Result<Vec<String>, String> {
    let mut written = Vec::new();
    for file in &plan.files {
        let abs = resolve_workspace_path(workspace_root, &file.path)?;
        if let Some(parent) = abs.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create temporary fix dir failed: {err}"))?;
        }
        fs::write(&abs, &file.content)
            .map_err(|err| format!("write temporary fix file failed: {err}"))?;
        written.push(path_string(&abs));
    }
    Ok(written)
}

pub(crate) fn write_external_skill_implementation(
    skill_dir: &Path,
    skill_name: &str,
    _capability_summary: &str,
    _actions: &[String],
    implementation: &ExternalSkillImplementation,
) -> Result<Vec<String>, String> {
    let manifest = PackageManifest::load(&skill_dir.join("skill.toml"))
        .map_err(|error| format!("read external skill manifest failed: {error}"))?;
    if manifest.package.name != skill_name {
        return Err(format!(
            "external skill identity mismatch: expected={skill_name} actual={}",
            manifest.package.name
        ));
    }
    let (source_entrypoint, scaffold_marker) = implementation_source_target(&manifest)?;
    let readme_path = skill_dir.join("README.md");
    let interface_path = skill_dir.join("INTERFACE.md");
    let source_path = skill_dir.join(source_entrypoint);

    ensure_sdk_scaffold_file(&readme_path, "rustclaw-skill validate")?;
    ensure_sdk_scaffold_file(&interface_path, "## Error Contract")?;
    ensure_sdk_scaffold_file(&source_path, scaffold_marker)?;

    if let Some(parent) = source_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create external skill src dir failed: {err}"))?;
    }

    fs::write(&readme_path, &implementation.readme_md)
        .map_err(|err| format!("write external skill README.md failed: {err}"))?;
    fs::write(&interface_path, &implementation.interface_md)
        .map_err(|err| format!("write external skill INTERFACE.md failed: {err}"))?;
    fs::write(&source_path, &implementation.entrypoint_source)
        .map_err(|err| format!("write external skill source entrypoint failed: {err}"))?;

    Ok(vec![
        path_string(&readme_path),
        path_string(&interface_path),
        path_string(&source_path),
    ])
}

pub(crate) fn implementation_source_target(
    manifest: &PackageManifest,
) -> Result<(&'static str, &'static str), String> {
    use rustclaw_skill_sdk::BuildAdapter;
    match manifest.build.adapter {
        BuildAdapter::Cargo => Ok(("src/main.rs", "fn respond(request: Request)")),
        BuildAdapter::Python => Ok(("src/main.py", "def respond(request: dict)")),
        BuildAdapter::Node => Ok(("src/main.mjs", "export function respond(request)")),
        BuildAdapter::Go => Ok(("main.go", "func respond(input request)")),
        adapter => Err(format!(
            "implement_external_skill requires developer-supplied artifacts for adapter={}",
            adapter.as_token()
        )),
    }
}

fn ensure_sdk_scaffold_file(path: &Path, marker: &str) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let current = fs::read_to_string(path)
        .map_err(|error| format!("read existing scaffold file failed: {error}"))?;
    if current.contains(marker) {
        return Ok(());
    }
    Err(format!(
        "refusing to overwrite non-scaffold file: {}",
        path.display()
    ))
}

pub(crate) fn validate_external_skill(
    repo_root: &Path,
    skill_name: &str,
    _actions: &[String],
) -> Result<ExternalSkillValidationReport, String> {
    let skill_dir = repo_root.join("external_skills").join(skill_name);
    let manifest_path = skill_dir.join("skill.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "external skill skill.toml does not exist: {}",
            manifest_path.display()
        ));
    }

    let sync = run_command_capture(repo_root, "python3", &["scripts/sync_skill_docs.py"], None)?;
    if sync.exit_code != 0 {
        return Err(format!(
            "sync_skill_docs.py failed: {}",
            best_process_output(&sync)
        ));
    }

    let staging_root = prepare_validation_staging_dir(skill_name)?;
    let staged_workspace = staging_root.join("workspace");
    let staged_skill = staged_workspace.join("external_skills").join(skill_name);
    fs::create_dir_all(
        staged_skill
            .parent()
            .ok_or_else(|| "external skill staging parent is unavailable".to_string())?,
    )
    .map_err(|error| format!("create validation workspace failed: {error}"))?;
    copy_dir_recursive(&skill_dir, &staged_skill)?;
    let staged_manifest = staged_skill.join("skill.toml");

    let validation_result = (|| -> Result<ExternalSkillValidationReport, String> {
        let manifest = PackageManifest::load(&staged_manifest)
            .map_err(|error| format!("manifest validation failed: {error}"))?;
        if manifest.package.name != skill_name || manifest.registry.name != skill_name {
            return Err(format!(
                "external skill identity mismatch: directory={skill_name} package={} registry={}",
                manifest.package.name, manifest.registry.name
            ));
        }
        let outcome = SkillInstaller
            .install(&InstallRequest {
                manifest_path: staged_manifest,
                workspace_root: staged_workspace,
                package_root: staging_root.join("packages"),
                target: None,
                allow_network: false,
                control: None,
            })
            .map_err(|error| {
                format!(
                    "external skill validation failed: phase={:?} code={} detail={}",
                    error.phase, error.code, error.detail
                )
            })?;

        Ok(ExternalSkillValidationReport {
            synced_docs: true,
            manifest_valid: true,
            adapter: outcome.adapter.as_token().to_string(),
            build_ok: true,
            smoke_test_ok: true,
            smoke_status: "ok".to_string(),
            receipt_digest: outcome.receipt_digest,
        })
    })();

    let _ = fs::remove_dir_all(&staging_root);
    validation_result
}

pub(crate) fn register_external_skill(
    repo_root: &Path,
    skill_name: &str,
) -> Result<ExternalSkillRegistrationReport, String> {
    let registry_path = repo_root.join("configs/skills_registry.toml");
    let config_path = repo_root.join("configs/config.toml");
    let manifest_relative = format!("external_skills/{skill_name}/skill.toml");
    PackageManifest::load(&repo_root.join(&manifest_relative))
        .map_err(|error| format!("external skill manifest invalid: {error}"))?;
    SkillRuntimeResolver::new(external_package_root(repo_root))
        .resolve(skill_name)
        .map_err(|error| format!("external skill verified install missing: {error}"))?;
    let registry_raw = fs::read_to_string(&registry_path)
        .map_err(|err| format!("read skills_registry.toml failed: {err}"))?;
    let (registry_updated, registry_entry_added) =
        add_registry_entry_text(&registry_raw, skill_name, &manifest_relative);

    let config_raw = fs::read_to_string(&config_path)
        .map_err(|err| format!("read config.toml failed: {err}"))?;
    let mut switches = collect_skill_switches_from_text(&config_raw);
    let (config_updated, switch_recorded_enabled) = match switches.get(skill_name).copied() {
        Some(true) => (config_raw.clone(), false),
        _ => {
            switches.insert(skill_name.to_string(), true);
            let rendered = render_switches_inline_table(&switches);
            (upsert_skill_switches_line(&config_raw, &rendered), true)
        }
    };

    if registry_entry_added {
        if let Err(err) = fs::write(&registry_path, &registry_updated) {
            return Err(format!("write skills_registry.toml failed: {err}"));
        }
    }

    if switch_recorded_enabled {
        if let Err(err) = fs::write(&config_path, &config_updated) {
            if registry_entry_added {
                let _ = fs::write(&registry_path, &registry_raw);
            }
            return Err(format!(
                "write config.toml failed: {err}; rolled back prior registry metadata changes"
            ));
        }
    }

    Ok(ExternalSkillRegistrationReport {
        registry_entry_added,
        switch_recorded_enabled,
        package_manifest: manifest_relative,
        matrix_admission_eligible: false,
    })
}

pub(crate) fn external_package_root(repo_root: &Path) -> PathBuf {
    repo_root.join("data/skill-packages")
}

pub(crate) fn install_external_skill(
    repo_root: &Path,
    skill_name: &str,
) -> Result<InstallOutcome, String> {
    SkillInstaller
        .install(&InstallRequest {
            manifest_path: repo_root
                .join("external_skills")
                .join(skill_name)
                .join("skill.toml"),
            workspace_root: repo_root.to_path_buf(),
            package_root: external_package_root(repo_root),
            target: None,
            allow_network: false,
            control: None,
        })
        .map_err(|error| {
            format!(
                "external skill install failed: phase={:?} code={} detail={}",
                error.phase, error.code, error.detail
            )
        })
}

pub(crate) fn enable_external_skill(
    repo_root: &Path,
    skill_name: &str,
) -> Result<ExternalSkillEnableReport, String> {
    let config_path = repo_root.join("configs/config.toml");
    let config_raw = fs::read_to_string(&config_path)
        .map_err(|err| format!("read config.toml failed: {err}"))?;
    let mut switches = collect_skill_switches_from_text(&config_raw);
    let (config_updated, switch_enabled) = match switches.get(skill_name).copied() {
        Some(true) => (config_raw.clone(), false),
        _ => {
            switches.insert(skill_name.to_string(), true);
            let rendered = render_switches_inline_table(&switches);
            (upsert_skill_switches_line(&config_raw, &rendered), true)
        }
    };

    let had_verified_install = SkillRuntimeResolver::new(external_package_root(repo_root))
        .resolve(skill_name)
        .is_ok();
    let install = install_external_skill(repo_root, skill_name)?;

    if switch_enabled {
        if let Err(err) = fs::write(&config_path, &config_updated) {
            restore_external_install(repo_root, skill_name, had_verified_install);
            return Err(format!(
                "write config.toml failed: {err}; rolled back installed package and left the skill disabled"
            ));
        }
    }

    Ok(ExternalSkillEnableReport {
        switch_enabled,
        install_ok: true,
        adapter: install.adapter.as_token().to_string(),
        installed_version: install.version,
        receipt_digest: install.receipt_digest,
        install_root: path_string(&install.install_root),
        reload_required: true,
    })
}

fn restore_external_install(repo_root: &Path, skill_name: &str, had_verified_install: bool) {
    let store = InstallReceiptStore::new(external_package_root(repo_root));
    if had_verified_install {
        let _ = store.rollback(skill_name);
    } else {
        let _ = store.remove_installed_versions(skill_name);
    }
}

pub(crate) fn prepare_validation_staging_dir(skill_name: &str) -> Result<PathBuf, String> {
    prepare_staging_dir("validate", skill_name)
}

pub(crate) fn prepare_staging_dir(prefix: &str, skill_name: &str) -> Result<PathBuf, String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|err| format!("system time error: {err}"))?
        .as_nanos();
    let root = env::temp_dir().join(format!(
        "extension-manager-{prefix}-{}-{}-{nanos}",
        std::process::id(),
        skill_name
    ));
    if root.exists() {
        fs::remove_dir_all(&root)
            .map_err(|err| format!("remove stale validation dir failed: {err}"))?;
    }
    fs::create_dir_all(&root).map_err(|err| format!("create validation dir failed: {err}"))?;
    Ok(root)
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|err| format!("create target dir failed: {err}"))?;
    for entry in fs::read_dir(src).map_err(|err| format!("read dir failed: {err}"))? {
        let entry = entry.map_err(|err| format!("read dir entry failed: {err}"))?;
        let source_path = entry.path();
        let target_path = dst.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|err| format!("read file type failed: {err}"))?;
        if file_type.is_dir() {
            copy_dir_recursive(&source_path, &target_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &target_path)
                .map_err(|err| format!("copy file failed: {err}"))?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub(crate) struct ProcessCapture {
    pub(crate) exit_code: i32,
    pub(crate) stdout: String,
    pub(crate) stderr: String,
}

pub(crate) fn run_command_capture(
    cwd: &Path,
    program: &str,
    args: &[&str],
    stdin_text: Option<&str>,
) -> Result<ProcessCapture, String> {
    let mut command = Command::new(program);
    command.args(args).current_dir(cwd);
    if stdin_text.is_some() {
        command.stdin(std::process::Stdio::piped());
    }
    command.stdout(std::process::Stdio::piped());
    command.stderr(std::process::Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|err| format!("spawn command failed ({program}): {err}"))?;
    if let Some(input) = stdin_text {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write as _;
            stdin
                .write_all(input.as_bytes())
                .map_err(|err| format!("write command stdin failed ({program}): {err}"))?;
        }
    }
    let output = child
        .wait_with_output()
        .map_err(|err| format!("wait command failed ({program}): {err}"))?;
    Ok(ProcessCapture {
        exit_code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

pub(crate) fn best_process_output(output: &ProcessCapture) -> String {
    if !output.stderr.trim().is_empty() {
        truncate_preview(&output.stderr, 400)
    } else if !output.stdout.trim().is_empty() {
        truncate_preview(&output.stdout, 400)
    } else {
        format!("exit={}", output.exit_code)
    }
}

pub(crate) fn conservative_registry_entry_text(skill_name: &str, package_manifest: &str) -> String {
    format!(
        r#"
[[skills]]
name = "{skill_name}"
enabled = false
kind = "external"
planner_kind = "skill"
install_mode = "on_demand"
package_manifest = "{package_manifest}"
aliases = []
description = "External skill {skill_name}; see its INTERFACE.md for the capability contract."
semantic_tags = []
preferred_over_run_cmd = false
validation_actions = []
timeout_seconds = 30
prompt_file = "prompts/skills/{skill_name}.md"
output_kind = "text"
risk_level = "high"
auto_invocable = false
requires_confirmation = true
side_effect = true
retryable = false
matrix_admission = {{ eligible = false, declared_actions = [], evidence_sources = [], required_extra_fields = [], extractor_kind = "structured_json", admission_version = "external-v1" }}
"#
    )
}

pub(crate) fn add_registry_entry_text(
    raw: &str,
    skill_name: &str,
    package_manifest: &str,
) -> (String, bool) {
    if raw.contains(&format!("name = \"{skill_name}\"")) {
        return (raw.to_string(), false);
    }
    let mut updated = raw.trim_end().to_string();
    updated.push_str(&conservative_registry_entry_text(
        skill_name,
        package_manifest,
    ));
    updated.push('\n');
    (updated, true)
}

pub(crate) fn collect_skill_switches_from_text(
    raw: &str,
) -> std::collections::BTreeMap<String, bool> {
    let mut in_skills = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed == "[skills]" {
            in_skills = true;
            continue;
        }
        if in_skills && trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != "[skills]"
        {
            break;
        }
        if in_skills
            && trimmed.starts_with("skill_switches")
            && trimmed.contains('{')
            && trimmed.contains('}')
        {
            let body = trimmed
                .split_once('{')
                .and_then(|(_, rest)| rest.rsplit_once('}').map(|(inner, _)| inner))
                .unwrap_or("");
            let mut out = std::collections::BTreeMap::new();
            for pair in body.split(',') {
                let pair = pair.trim();
                if pair.is_empty() {
                    continue;
                }
                let Some((key, value)) = pair.split_once('=') else {
                    continue;
                };
                let key = key.trim().to_string();
                match value.trim() {
                    "true" => {
                        out.insert(key, true);
                    }
                    "false" => {
                        out.insert(key, false);
                    }
                    _ => {}
                }
            }
            return out;
        }
    }
    std::collections::BTreeMap::new()
}

pub(crate) fn render_switches_inline_table(
    switches: &std::collections::BTreeMap<String, bool>,
) -> String {
    if switches.is_empty() {
        return "skill_switches = {}".to_string();
    }
    let pairs = switches
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("skill_switches = {{ {pairs} }}")
}

pub(crate) fn upsert_skill_switches_line(raw: &str, rendered_line: &str) -> String {
    let mut lines: Vec<String> = raw.lines().map(|s| s.to_string()).collect();
    let mut in_skills = false;
    let mut inserted_or_replaced = false;
    let mut skills_section_seen = false;
    let mut insert_index_in_skills: Option<usize> = None;
    let mut skills_section_end: Option<usize> = None;

    for idx in 0..lines.len() {
        let trimmed = lines[idx].trim();
        if trimmed == "[skills]" {
            in_skills = true;
            skills_section_seen = true;
            insert_index_in_skills = Some(idx + 1);
            continue;
        }
        if trimmed.starts_with('[') && trimmed.ends_with(']') && trimmed != "[skills]" {
            if in_skills {
                skills_section_end = Some(idx);
                break;
            }
            continue;
        }
        if in_skills && trimmed.starts_with("skill_switches") && trimmed.contains('=') {
            lines[idx] = rendered_line.to_string();
            inserted_or_replaced = true;
            break;
        }
        if in_skills && insert_index_in_skills.is_none() && !trimmed.is_empty() {
            insert_index_in_skills = Some(idx);
        }
        if in_skills && trimmed.starts_with("skills_list") && insert_index_in_skills.is_none() {
            insert_index_in_skills = Some(idx);
        }
    }

    if !inserted_or_replaced && skills_section_seen {
        let idx = insert_index_in_skills
            .or(skills_section_end)
            .unwrap_or(lines.len());
        lines.insert(idx, rendered_line.to_string());
    }

    let mut out = lines.join("\n");
    if raw.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub(crate) fn install_plan_packages(plan: &TemporaryFixPlan) -> Result<Vec<Value>, String> {
    let mut installed = Vec::new();
    for package in &plan.packages {
        ensure_installer_available(&package.ecosystem)?;
        for module in &package.modules {
            let out = run_install_command(&package.ecosystem, module, package.version.as_deref())?;
            if !out.status.success() {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let detail = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else {
                    stdout.trim().to_string()
                };
                return Err(format!(
                    "temporary fix install failed: ecosystem={}, module={}; {}",
                    package.ecosystem, module, detail
                ));
            }
        }
        installed.push(json!({
            "ecosystem": package.ecosystem,
            "modules": package.modules,
            "version": package.version,
        }));
    }
    Ok(installed)
}

pub(crate) fn run_plan_commands(
    workspace_root: &Path,
    plan: &TemporaryFixPlan,
) -> Result<Vec<CommandRunRecord>, String> {
    let mut records = Vec::new();
    for command in &plan.commands {
        let script_abs = resolve_workspace_path(workspace_root, &command.script_path)?;
        let cwd_rel = command.cwd.as_deref().unwrap_or(".");
        let cwd_abs = resolve_workspace_path(workspace_root, cwd_rel)?;
        let mut cmd = Command::new(&command.runtime);
        cmd.arg(&script_abs);
        for arg in &command.args {
            cmd.arg(arg);
        }
        cmd.current_dir(&cwd_abs);
        let out = cmd
            .output()
            .map_err(|err| format!("run temporary fix command failed: {err}"))?;
        records.push(CommandRunRecord {
            runtime: command.runtime.clone(),
            script_path: command.script_path.clone(),
            cwd: cwd_rel.to_string(),
            args: command.args.clone(),
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).trim().to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(records)
}

pub(crate) fn scaffold_external_skill(
    repo_root: PathBuf,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let skill_name = required_string(obj, "skill_name")?;
    validate_identifier("skill_name", &skill_name)?;
    let capability_summary = required_string(obj, "capability_summary")?;
    let actions = extract_actions(obj)?;
    let language_token = obj
        .get("implementation_language")
        .or_else(|| obj.get("build_adapter"))
        .and_then(Value::as_str)
        .unwrap_or("rust");
    let implementation_language = rustclaw_skill_sdk::ImplementationLanguage::parse(language_token)
        .map_err(|error| error.to_string())?;
    let skill_dir = repo_root.join("external_skills").join(&skill_name);
    if skill_dir.exists() {
        return Err(format!(
            "skill directory already exists: {}",
            skill_dir.display()
        ));
    }

    let scaffold = rustclaw_skill_sdk::scaffold_skill(&rustclaw_skill_sdk::ScaffoldRequest {
        destination: skill_dir.clone(),
        skill_name: skill_name.clone(),
        capability_summary,
        actions: actions.clone(),
        implementation_language,
        source_root: format!("external_skills/{skill_name}"),
    })
    .map_err(|error| format!("create external skill scaffold failed: {error}"))?;
    let created_files = scaffold
        .written_files
        .iter()
        .map(|path| path_string(path))
        .collect::<Vec<_>>();
    Ok((
        format!(
            "Scaffolded external skill `{skill_name}` at external_skills/{skill_name}. It is not registered or enabled."
        ),
        json!({
            "action": "scaffold_external_skill",
            "skill_name": skill_name,
            "implementation_language": implementation_language.as_token(),
            "build_adapter": PackageManifest::load(&scaffold.manifest_path)
                .map_err(|error| error.to_string())?
                .build
                .adapter
                .as_token(),
            "skill_dir": path_string(&skill_dir),
            "manifest_path": path_string(&scaffold.manifest_path),
            "created_files": created_files,
            "actions": actions,
            "default_enabled": false,
            "next_steps": [
                "Fill external_skills/<skill>/INTERFACE.md with the real contract.",
                "Implement the actual logic in the generated language entrypoint.",
                "Run python3 scripts/sync_skill_docs.py.",
                "Compile and smoke-test the skill, then register it with confirm=true to enable it in config."
            ]
        }),
    ))
}

pub(crate) fn extract_actions(obj: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut out = match obj.get("actions") {
        None => Vec::new(),
        Some(Value::String(s)) => vec![s.trim().to_string()],
        Some(Value::Array(items)) => {
            let mut values = Vec::new();
            for item in items {
                let Some(s) = item.as_str() else {
                    return Err("actions must be strings".to_string());
                };
                values.push(s.trim().to_string());
            }
            values
        }
        Some(_) => return Err("actions must be a string or string array".to_string()),
    };

    out.retain(|action| !action.is_empty());
    if out.is_empty() {
        out.push("todo_action".to_string());
    }
    if out.len() > 12 {
        return Err("too many actions; limit is 12".to_string());
    }
    for action in &out {
        validate_identifier("action", action)?;
    }
    Ok(out)
}

pub(crate) fn required_string(obj: &Map<String, Value>, key: &str) -> Result<String, String> {
    let value = obj
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if value.is_empty() {
        return Err(format!("{key} is required"));
    }
    Ok(value.to_string())
}

pub(crate) fn require_confirm(obj: &Map<String, Value>, action: &str) -> Result<(), String> {
    let confirmed = obj
        .get("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if confirmed {
        Ok(())
    } else {
        Err(format!("{action} requires confirm=true"))
    }
}

pub(crate) fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
    if value
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_')
    {
        return Ok(());
    }
    Err(format!(
        "invalid {label}: {value}; use snake_case with lowercase letters, digits, and underscores only"
    ))
}

pub(crate) fn ensure_external_skill_scaffold_ready(
    repo_root: &Path,
    skill_name: &str,
) -> Result<(), String> {
    let skill_dir = repo_root.join("external_skills").join(skill_name);
    for required in ["skill.toml", "README.md", "INTERFACE.md"] {
        let path = skill_dir.join(required);
        if !path.exists() {
            return Err(format!(
                "external skill scaffold is missing required file: {}",
                path.display()
            ));
        }
    }
    PackageManifest::load(&skill_dir.join("skill.toml"))
        .map_err(|error| format!("external skill manifest invalid: {error}"))?;
    Ok(())
}

pub(crate) fn repo_root() -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    root.canonicalize()
        .map_err(|err| format!("resolve repo root failed: {err}"))
}

pub(crate) fn workspace_root() -> PathBuf {
    env::var("WORKSPACE_ROOT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo_root()
                .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        })
}

pub(crate) fn build_plan_root(request_id: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let request_slug = request_id
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .take(12)
        .collect::<String>();
    let request_slug = if request_slug.is_empty() {
        "plan".to_string()
    } else {
        request_slug
    };
    format!("tmp/extension_manager/{}-{}", request_slug, now)
}

pub(crate) fn normalize_plan_root(input: &str) -> Result<String, String> {
    let normalized = normalize_workspace_relative_path(input)?;
    let prefix = Path::new("tmp").join("extension_manager");
    if !normalized.starts_with(&prefix) {
        return Err("temporary fix plan_root must stay under tmp/extension_manager".to_string());
    }
    Ok(path_string(&normalized))
}

pub(crate) fn normalize_plan_member_path(plan_root: &str, input: &str) -> Result<String, String> {
    let normalized = normalize_workspace_relative_path(input)?;
    let root = Path::new(plan_root);
    let final_path = if normalized.starts_with(root) {
        normalized
    } else {
        root.join(normalized)
    };
    Ok(path_string(&final_path))
}

pub(crate) fn normalize_workspace_relative_path(input: &str) -> Result<PathBuf, String> {
    let raw = input.trim();
    if raw.is_empty() {
        return Err("path cannot be empty".to_string());
    }
    let candidate = Path::new(raw);
    if candidate.is_absolute() {
        return Err("absolute paths are not allowed".to_string());
    }
    let mut normalized = PathBuf::new();
    for component in candidate.components() {
        match component {
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => return Err("path with '..' is not allowed".to_string()),
            Component::RootDir | Component::Prefix(_) => {
                return Err("absolute paths are not allowed".to_string())
            }
        }
    }
    if normalized.as_os_str().is_empty() {
        return Ok(PathBuf::from("."));
    }
    Ok(normalized)
}

pub(crate) fn resolve_workspace_path(
    workspace_root: &Path,
    input: &str,
) -> Result<PathBuf, String> {
    let relative = normalize_workspace_relative_path(input)?;
    let joined = workspace_root.join(relative);
    ensure_within_workspace(workspace_root, &joined)?;
    Ok(joined)
}

pub(crate) fn ensure_within_workspace(
    workspace_root: &Path,
    candidate: &Path,
) -> Result<(), String> {
    if candidate.starts_with(workspace_root) {
        Ok(())
    } else {
        Err("resolved path escapes workspace root".to_string())
    }
}

pub(crate) fn normalize_runtime(input: &str) -> Result<String, String> {
    match input.trim() {
        "python3" | "bash" | "sh" | "node" => Ok(input.trim().to_string()),
        other => Err(format!(
            "unsupported runtime: {other}; use python3|bash|sh|node"
        )),
    }
}

pub(crate) fn normalize_ecosystem(input: &str) -> Result<String, String> {
    match input.trim().to_ascii_lowercase().as_str() {
        "python" | "pip" => Ok("python".to_string()),
        "node" | "npm" => Ok("node".to_string()),
        "rust" | "cargo" => Ok("rust".to_string()),
        "go" | "golang" => Ok("go".to_string()),
        other => Err(format!(
            "unsupported ecosystem: {other}; use python|node|rust|go"
        )),
    }
}

pub(crate) fn ensure_installer_available(ecosystem: &str) -> Result<(), String> {
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

pub(crate) fn run_install_command(
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
            c.arg("install").arg(render_module_for_go(module, version));
            c
        }
        _ => return Err(format!("unsupported ecosystem: {ecosystem}")),
    };

    cmd.output()
        .map_err(|err| format!("run installer failed: {err}"))
}

pub(crate) fn render_module_for_python(module: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{module}=={v}"),
        None => module.to_string(),
    }
}

pub(crate) fn render_module_for_node(module: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{module}@{v}"),
        None => module.to_string(),
    }
}

pub(crate) fn render_module_for_go(module: &str, version: Option<&str>) -> String {
    match version {
        Some(v) => format!("{module}@{v}"),
        None => format!("{module}@latest"),
    }
}

pub(crate) fn is_safe_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

pub(crate) fn extract_assistant_text(parsed: &Value) -> Option<String> {
    let mut candidates: Vec<String> = Vec::new();

    if let Some(choice) = parsed
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
    {
        if let Some(message) = choice.get("message") {
            if let Some(content) = message.get("content") {
                append_text_candidates(content, &mut candidates);
            }
            if let Some(reasoning) = message.get("reasoning_content") {
                append_text_candidates(reasoning, &mut candidates);
            }
        }
        if let Some(legacy_text) = choice.get("text") {
            append_text_candidates(legacy_text, &mut candidates);
        }
    }

    if let Some(output_text) = parsed.get("output_text") {
        append_text_candidates(output_text, &mut candidates);
    }

    if let Some(output_items) = parsed.get("output") {
        append_text_candidates(output_items, &mut candidates);
    }

    candidates
        .into_iter()
        .find(|candidate| !candidate.trim().is_empty())
}

pub(crate) fn append_text_candidates(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => {
            if !s.trim().is_empty() {
                out.push(s.clone());
            }
        }
        Value::Array(arr) => {
            for item in arr {
                append_text_candidates(item, out);
            }
        }
        Value::Object(obj) => {
            for key in ["text", "content", "input_text", "output_text"] {
                if let Some(v) = obj.get(key) {
                    append_text_candidates(v, out);
                }
            }
        }
        _ => {}
    }
}

pub(crate) fn extract_json_object(raw: &str) -> Option<String> {
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escape = false;
    for (idx, ch) in raw.char_indices() {
        if in_string {
            if escape {
                escape = false;
                continue;
            }
            match ch {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth += 1;
            }
            '}' => {
                if depth == 0 {
                    continue;
                }
                depth -= 1;
                if depth == 0 {
                    if let Some(start_idx) = start {
                        return Some(raw[start_idx..=idx].to_string());
                    }
                }
            }
            _ => {}
        }
    }
    None
}

pub(crate) fn default_model_for_base_url(base_url: &str) -> &'static str {
    let lower = base_url.trim().to_ascii_lowercase();
    if lower.contains("minimax") {
        "MiniMax-M2.5"
    } else if lower.contains("dashscope") || lower.contains("aliyuncs") {
        "qwen-plus-latest"
    } else if lower.contains("deepseek") {
        "deepseek-chat"
    } else if lower.contains("x.ai") {
        "grok-2-latest"
    } else {
        "gpt-4.1"
    }
}

pub(crate) fn truncate_preview(raw: &str, max_chars: usize) -> String {
    let mut preview = raw.chars().take(max_chars).collect::<String>();
    if raw.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

pub(crate) fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
