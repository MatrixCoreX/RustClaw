use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

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
    capability_summary: &str,
    actions: &[String],
    implementation: &ExternalSkillImplementation,
) -> Result<Vec<String>, String> {
    let readme_path = skill_dir.join("README.md");
    let interface_path = skill_dir.join("INTERFACE.md");
    let main_path = skill_dir.join("src").join("main.rs");

    ensure_scaffold_or_missing(&readme_path, &readme_template(skill_name, actions))?;
    ensure_scaffold_or_missing(
        &interface_path,
        &interface_template(skill_name, capability_summary, actions),
    )?;
    ensure_scaffold_or_missing(&main_path, &main_rs_template(actions))?;

    if let Some(parent) = main_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|err| format!("create external skill src dir failed: {err}"))?;
    }

    fs::write(&readme_path, &implementation.readme_md)
        .map_err(|err| format!("write external skill README.md failed: {err}"))?;
    fs::write(&interface_path, &implementation.interface_md)
        .map_err(|err| format!("write external skill INTERFACE.md failed: {err}"))?;
    fs::write(&main_path, &implementation.main_rs)
        .map_err(|err| format!("write external skill src/main.rs failed: {err}"))?;

    Ok(vec![
        path_string(&readme_path),
        path_string(&interface_path),
        path_string(&main_path),
    ])
}

pub(crate) fn validate_external_skill(
    repo_root: &Path,
    skill_name: &str,
    actions: &[String],
) -> Result<ExternalSkillValidationReport, String> {
    let skill_dir = repo_root.join("external_skills").join(skill_name);
    let manifest_path = skill_dir.join("Cargo.toml");
    if !manifest_path.exists() {
        return Err(format!(
            "external skill Cargo.toml does not exist: {}",
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
    copy_dir_recursive(&skill_dir, &staging_root)?;
    let staged_manifest = staging_root.join("Cargo.toml");
    let manifest_string = path_string(&staged_manifest);

    let validation_result = (|| -> Result<ExternalSkillValidationReport, String> {
        let cargo_check = run_command_capture(
            &staging_root,
            "cargo",
            &["check", "--manifest-path", &manifest_string],
            None,
        )?;
        if cargo_check.exit_code != 0 {
            return Err(format!(
                "cargo check for external skill failed: {}",
                best_process_output(&cargo_check)
            ));
        }

        let smoke_action = actions
            .first()
            .cloned()
            .unwrap_or_else(|| "todo_action".to_string());
        let request_id = format!("smoke-{}", skill_name);
        let smoke_request = json!({
            "request_id": request_id,
            "context": null,
            "user_id": 0,
            "chat_id": 0,
            "args": {
                "action": smoke_action
            }
        });
        let smoke = run_command_capture(
            &staging_root,
            "cargo",
            &["run", "--quiet", "--manifest-path", &manifest_string],
            Some(&format!("{}\n", smoke_request)),
        )?;
        if smoke.exit_code != 0 {
            return Err(format!(
                "external skill smoke test process failed: {}",
                best_process_output(&smoke)
            ));
        }
        let smoke_json = parse_single_json_line(&smoke.stdout).ok_or_else(|| {
            format!(
                "external skill smoke test returned non-JSON output: {}",
                smoke.stdout.trim()
            )
        })?;
        let smoke_status = smoke_json
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if smoke_json
            .get("request_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            != request_id
        {
            return Err("external skill smoke test returned mismatched request_id".to_string());
        }
        if smoke_status != "ok" && smoke_status != "error" {
            return Err("external skill smoke test returned invalid status".to_string());
        }
        if smoke_status == "error"
            && smoke_json
                .get("error_text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
        {
            return Err(
                "external skill smoke test returned error without readable error_text".to_string(),
            );
        }
        let smoke_text = smoke_json
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .trim()
            .to_string();

        Ok(ExternalSkillValidationReport {
            synced_docs: true,
            cargo_check_ok: true,
            smoke_test_ok: true,
            smoke_status,
            smoke_text,
        })
    })();

    let _ = fs::remove_dir_all(&staging_root);
    validation_result
}

pub(crate) fn register_external_skill(
    repo_root: &Path,
    skill_name: &str,
) -> Result<ExternalSkillRegistrationReport, String> {
    let cargo_toml_path = repo_root.join("Cargo.toml");
    let registry_path = repo_root.join("configs/skills_registry.toml");
    let config_path = repo_root.join("configs/config.toml");

    let cargo_raw = fs::read_to_string(&cargo_toml_path)
        .map_err(|err| format!("read Cargo.toml failed: {err}"))?;
    let (cargo_updated, workspace_member_added) =
        add_workspace_member_text(&cargo_raw, &format!("external_skills/{skill_name}"))?;
    let registry_raw = fs::read_to_string(&registry_path)
        .map_err(|err| format!("read skills_registry.toml failed: {err}"))?;
    let (registry_updated, registry_entry_added) =
        add_registry_entry_text(&registry_raw, skill_name);

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

    if workspace_member_added {
        fs::write(&cargo_toml_path, &cargo_updated)
            .map_err(|err| format!("write Cargo.toml failed: {err}"))?;
    }

    if registry_entry_added {
        if let Err(err) = fs::write(&registry_path, &registry_updated) {
            if workspace_member_added {
                let _ = fs::write(&cargo_toml_path, &cargo_raw);
            }
            return Err(format!(
                "write skills_registry.toml failed: {err}; rolled back prior workspace metadata changes"
            ));
        }
    }

    if switch_recorded_enabled {
        if let Err(err) = fs::write(&config_path, &config_updated) {
            if registry_entry_added {
                let _ = fs::write(&registry_path, &registry_raw);
            }
            if workspace_member_added {
                let _ = fs::write(&cargo_toml_path, &cargo_raw);
            }
            return Err(format!(
                "write config.toml failed: {err}; rolled back prior workspace metadata changes"
            ));
        }
    }

    Ok(ExternalSkillRegistrationReport {
        workspace_member_added,
        registry_entry_added,
        switch_recorded_enabled,
        matrix_admission_eligible: false,
    })
}

pub(crate) fn external_skill_binary_name(skill_name: &str) -> String {
    format!("{}-skill", skill_name.replace('_', "-"))
}

pub(crate) fn external_skill_release_binary_path(repo_root: &Path, skill_name: &str) -> PathBuf {
    repo_root
        .join("target/release")
        .join(external_skill_binary_name(skill_name))
}

pub(crate) fn build_external_skill_release_binary(
    repo_root: &Path,
    skill_name: &str,
) -> Result<PathBuf, String> {
    let binary_name = external_skill_binary_name(skill_name);
    let skill_dir = repo_root.join("external_skills").join(skill_name);
    let staging_root = prepare_staging_dir("enable", skill_name)?;
    copy_dir_recursive(&skill_dir, &staging_root)?;
    let staged_manifest = staging_root.join("Cargo.toml");
    let manifest_string = path_string(&staged_manifest);
    let release_binary_path = external_skill_release_binary_path(repo_root, skill_name);

    let build_result = (|| -> Result<(), String> {
        let build = run_command_capture(
            &staging_root,
            "cargo",
            &["build", "--release", "--manifest-path", &manifest_string],
            None,
        )?;
        if build.exit_code != 0 {
            return Err(format!(
                "external skill release build failed: {}",
                best_process_output(&build)
            ));
        }
        let staged_binary = staging_root.join("target/release").join(&binary_name);
        if !staged_binary.exists() {
            return Err(format!(
                "external skill release build completed without binary: {}",
                staged_binary.display()
            ));
        }
        if let Some(parent) = release_binary_path.parent() {
            fs::create_dir_all(parent)
                .map_err(|err| format!("create release target dir failed: {err}"))?;
        }
        fs::copy(&staged_binary, &release_binary_path)
            .map_err(|err| format!("copy release binary failed: {err}"))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&staging_root);
    build_result?;

    Ok(release_binary_path)
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

    let release_binary_path = external_skill_release_binary_path(repo_root, skill_name);
    let original_release_binary = fs::read(&release_binary_path).ok();
    let release_binary_path = build_external_skill_release_binary(repo_root, skill_name)?;

    if switch_enabled {
        if let Err(err) = fs::write(&config_path, &config_updated) {
            match original_release_binary {
                Some(bytes) => {
                    let _ = fs::write(&release_binary_path, bytes);
                }
                None => {
                    let _ = fs::remove_file(&release_binary_path);
                }
            }
            return Err(format!(
                "write config.toml failed: {err}; rolled back release binary and left the skill disabled"
            ));
        }
    }

    let release_binary_path = path_string(&release_binary_path);

    Ok(ExternalSkillEnableReport {
        switch_enabled,
        release_build_ok: true,
        release_binary_path,
        reload_required: true,
    })
}

pub(crate) fn ensure_scaffold_or_missing(
    path: &Path,
    scaffold_content: &str,
) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let current = fs::read_to_string(path)
        .map_err(|err| format!("read existing scaffold file failed: {err}"))?;
    if current == scaffold_content {
        return Ok(());
    }
    Err(format!(
        "refusing to overwrite non-scaffold file: {}",
        path.display()
    ))
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

pub(crate) fn parse_single_json_line(raw: &str) -> Option<Value> {
    let non_empty = raw
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if non_empty.len() != 1 {
        return None;
    }
    serde_json::from_str::<Value>(non_empty[0]).ok()
}

pub(crate) fn add_workspace_member_text(
    raw: &str,
    member_path: &str,
) -> Result<(String, bool), String> {
    let target = format!("    \"{member_path}\",");
    if raw.contains(&target) {
        return Ok((raw.to_string(), false));
    }
    let members_pos = raw
        .find("members = [")
        .ok_or_else(|| "cannot find workspace members block in Cargo.toml".to_string())?;
    let search = &raw[members_pos..];
    let insert_rel = search
        .find("\n]")
        .ok_or_else(|| "cannot find workspace members closing bracket in Cargo.toml".to_string())?;
    let insert_at = members_pos + insert_rel;
    let updated = format!("{}{}\n{}", &raw[..insert_at], target, &raw[insert_at..]);
    Ok((updated, true))
}

pub(crate) fn conservative_registry_entry_text(skill_name: &str) -> String {
    format!(
        r#"
[[skills]]
name = "{skill_name}"
enabled = false
kind = "runner"
planner_kind = "skill"
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

pub(crate) fn add_registry_entry_text(raw: &str, skill_name: &str) -> (String, bool) {
    if raw.contains(&format!("name = \"{skill_name}\"")) {
        return (raw.to_string(), false);
    }
    let mut updated = raw.trim_end().to_string();
    updated.push_str(&conservative_registry_entry_text(skill_name));
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
    let skill_dir = repo_root.join("external_skills").join(&skill_name);
    if skill_dir.exists() {
        return Err(format!(
            "skill directory already exists: {}",
            skill_dir.display()
        ));
    }

    let binary_name = format!("{}-skill", skill_name.replace('_', "-"));
    let readme_path = skill_dir.join("README.md");
    let cargo_path = skill_dir.join("Cargo.toml");
    let interface_path = skill_dir.join("INTERFACE.md");
    let src_dir = skill_dir.join("src");
    let main_path = src_dir.join("main.rs");
    fs::create_dir_all(&src_dir).map_err(|err| format!("create scaffold dirs failed: {err}"))?;

    write_new_file(&readme_path, &readme_template(&skill_name, &actions))
        .map_err(|err| format!("write README.md failed: {err}"))?;
    write_new_file(&cargo_path, &cargo_toml_template(&binary_name))
        .map_err(|err| format!("write Cargo.toml failed: {err}"))?;
    write_new_file(
        &interface_path,
        &interface_template(&skill_name, &capability_summary, &actions),
    )
    .map_err(|err| format!("write INTERFACE.md failed: {err}"))?;
    write_new_file(&main_path, &main_rs_template(&actions))
        .map_err(|err| format!("write src/main.rs failed: {err}"))?;

    let created_files = vec![
        path_string(&readme_path),
        path_string(&cargo_path),
        path_string(&interface_path),
        path_string(&main_path),
    ];
    Ok((
        format!(
            "Scaffolded external skill `{skill_name}` at external_skills/{skill_name}. It is not registered or enabled."
        ),
        json!({
            "action": "scaffold_external_skill",
            "skill_name": skill_name,
            "binary_name": binary_name,
            "skill_dir": path_string(&skill_dir),
            "created_files": created_files,
            "actions": actions,
            "default_enabled": false,
            "next_steps": [
                "Fill external_skills/<skill>/INTERFACE.md with the real contract.",
                "Implement the actual logic in src/main.rs.",
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

pub(crate) fn write_new_file(path: &Path, content: &str) -> std::io::Result<()> {
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "file already exists",
        ));
    }
    fs::write(path, content)
}

pub(crate) fn ensure_external_skill_scaffold_ready(
    repo_root: &Path,
    skill_name: &str,
) -> Result<(), String> {
    let skill_dir = repo_root.join("external_skills").join(skill_name);
    for required in ["Cargo.toml", "README.md", "INTERFACE.md", "src/main.rs"] {
        let path = skill_dir.join(required);
        if !path.exists() {
            return Err(format!(
                "external skill scaffold is missing required file: {}",
                path.display()
            ));
        }
    }
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

pub(crate) fn readme_template(skill_name: &str, actions: &[String]) -> String {
    let mut lines = vec![
        format!("# {skill_name} External Skill Scaffold"),
        String::new(),
        "This scaffold was generated by `extension_manager`.".to_string(),
        "It is intentionally isolated under `external_skills/` and stays unregistered until validation passes.".to_string(),
        String::new(),
        "## Proposed Actions".to_string(),
    ];
    for action in actions {
        lines.push(format!("- `{action}`"));
    }
    lines.extend([
        String::new(),
        "## Next Steps".to_string(),
        "1. Complete `INTERFACE.md` with the real action contract.".to_string(),
        "2. Implement the action logic in `src/main.rs`.".to_string(),
        "3. Run `python3 scripts/sync_skill_docs.py`.".to_string(),
        "4. Register the skill explicitly only after compile and smoke tests pass; registration enables it in config.".to_string(),
        String::new(),
    ]);
    lines.join("\n")
}

pub(crate) fn cargo_toml_template(binary_name: &str) -> String {
    format!(
        "[package]\nname = \"{binary_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{binary_name}\"\npath = \"src/main.rs\"\n\n[dependencies]\nanyhow = \"1\"\nserde = {{ version = \"1\", features = [\"derive\"] }}\nserde_json = \"1\"\n"
    )
}

pub(crate) fn interface_template(
    skill_name: &str,
    capability_summary: &str,
    actions: &[String],
) -> String {
    let action_lines = actions
        .iter()
        .map(|action| format!("- `{action}`: TODO: describe what this action should do."))
        .collect::<Vec<_>>()
        .join("\n");
    let param_rows = actions
        .iter()
        .map(|action| {
            format!(
                "| `{action}` | `action` | yes | string | `{action}` | Fixed action selector. |"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let request_examples = actions
        .iter()
        .enumerate()
        .map(|(idx, action)| {
            format!(
                "### Example {}\nRequest:\n```json\n{{\"request_id\":\"demo-{}\",\"context\":null,\"user_id\":1,\"chat_id\":1,\"args\":{{\"action\":\"{}\"}}}}\n```\nResponse:\n```json\n{{\"request_id\":\"demo-{}\",\"status\":\"ok\",\"text\":\"TODO\",\"extra\":{{\"action\":\"{}\"}},\"error_text\":null}}\n```",
                idx + 1,
                idx + 1,
                action,
                idx + 1,
                action
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "# {skill_name} Interface Spec\n\n> This file was scaffolded by `extension_manager`.\n> Keep it aligned with `external_skills/{skill_name}/src/main.rs`.\n\n## Capability Summary\n- {capability_summary}\n- This scaffold stays unregistered until validation passes; registration enables it in config.\n\n## Config Entry Points\n- If this skill has dedicated setup, list the real entry points here: config file, environment variable, local DB/API, login/session state, or dependency.\n- If it does not need dedicated setup, state that explicitly.\n\n## Actions\n{action_lines}\n\n## Parameter Contract\n| Action | Param | Required | Type | Default | Description |\n|---|---|---|---|---|---|\n{param_rows}\n\n## Error Contract\n- Return `status=error` with readable `error_text` when required params are missing.\n- Return `unsupported action: <name>` for unknown actions.\n- Keep request/response payloads as single-line JSON objects over stdin/stdout.\n\n## Structured Evidence Contract\n- Matrix admission status: not eligible by default.\n- To request matrix evidence eligibility, declare stable success `extra` fields per action.\n- For each field, document type, meaning, sensitivity, and which evidence role it can satisfy (`field_value`, `count`, `path`, `results`, `delivery_artifact`, etc.).\n- Error responses should include `extra.error_kind` when feasible.\n- Do not rely on natural-language `text` as strict matrix evidence.\n\n## Request/Response Examples\n{request_examples}\n"
    )
}

pub(crate) fn main_rs_template(actions: &[String]) -> String {
    let supported_actions = actions
        .iter()
        .map(|action| format!("\\\"{action}\\\""))
        .collect::<Vec<_>>()
        .join(" | ");
    let match_arms = actions
        .iter()
        .map(|action| {
            format!(
                "        \"{action}\" => Ok((\"TODO: implement {action}\".to_string(), json!({{\"action\":\"{action}\"}}))),"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "use std::io::{{self, BufRead, Write}};\n\nuse serde::{{Deserialize, Serialize}};\nuse serde_json::{{json, Value}};\n\n#[derive(Debug, Deserialize)]\nstruct Req {{\n    request_id: String,\n    args: Value,\n    #[serde(default, rename = \"context\")]\n    _context: Option<Value>,\n    #[serde(default, rename = \"user_id\")]\n    _user_id: i64,\n    #[serde(default, rename = \"chat_id\")]\n    _chat_id: i64,\n}}\n\n#[derive(Debug, Serialize)]\nstruct Resp {{\n    request_id: String,\n    status: String,\n    text: String,\n    #[serde(skip_serializing_if = \"Option::is_none\")]\n    extra: Option<Value>,\n    error_text: Option<String>,\n}}\n\nfn main() -> anyhow::Result<()> {{\n    let stdin = io::stdin();\n    let mut stdout = io::stdout();\n\n    for line in stdin.lock().lines() {{\n        let line = line?;\n        let parsed: Result<Req, _> = serde_json::from_str(&line);\n        let resp = match parsed {{\n            Ok(req) => match execute(req.args) {{\n                Ok((text, extra)) => Resp {{\n                    request_id: req.request_id,\n                    status: \"ok\".to_string(),\n                    text,\n                    extra: Some(extra),\n                    error_text: None,\n                }},\n                Err(err) => Resp {{\n                    request_id: req.request_id,\n                    status: \"error\".to_string(),\n                    text: String::new(),\n                    extra: None,\n                    error_text: Some(err),\n                }},\n            }},\n            Err(err) => Resp {{\n                request_id: \"unknown\".to_string(),\n                status: \"error\".to_string(),\n                text: String::new(),\n                extra: None,\n                error_text: Some(format!(\"invalid input: {{err}}\")),\n            }},\n        }};\n        writeln!(stdout, \"{{}}\", serde_json::to_string(&resp)?)?;\n        stdout.flush()?;\n    }}\n\n    Ok(())\n}}\n\nfn execute(args: Value) -> Result<(String, Value), String> {{\n    let obj = args\n        .as_object()\n        .ok_or_else(|| \"args must be object\".to_string())?;\n    let action = obj\n        .get(\"action\")\n        .and_then(|v| v.as_str())\n        .ok_or_else(|| \"action is required\".to_string())?;\n\n    match action {{\n{match_arms}\n        _ => Err(format!(\"unsupported action: {{action}}; use {supported_actions}\")),\n    }}\n}}\n"
    )
}
