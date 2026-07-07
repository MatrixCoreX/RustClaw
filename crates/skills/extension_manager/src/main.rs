use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

const TEMP_FIX_SYSTEM_PROMPT: &str = include_str!(
    "../../../../prompts/layers/overlays/extension_manager_temporary_fix_system_prompt.md"
);
const PERMANENT_EXTENSION_SYSTEM_PROMPT: &str = include_str!(
    "../../../../prompts/layers/overlays/extension_manager_permanent_extension_system_prompt.md"
);
const SKILL_IMPLEMENTATION_SYSTEM_PROMPT: &str = include_str!(
    "../../../../prompts/layers/overlays/extension_manager_skill_implementation_system_prompt.md"
);
const TEMPORARY_FIX_PLAN_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/temporary_fix_plan.schema.json");
const PERMANENT_EXTENSION_PLAN_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/permanent_extension_plan.schema.json");
const EXTERNAL_SKILL_IMPLEMENTATION_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/external_skill_implementation.schema.json");
const SKILL_NAME: &str = "extension_manager";

static TEMPORARY_FIX_PLAN_SCHEMA: OnceLock<Value> = OnceLock::new();
static PERMANENT_EXTENSION_PLAN_SCHEMA: OnceLock<Value> = OnceLock::new();
static EXTERNAL_SKILL_IMPLEMENTATION_SCHEMA: OnceLock<Value> = OnceLock::new();

mod helpers;
use helpers::*;

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[serde(default)]
    #[serde(rename = "context")]
    _context: Option<Value>,
    #[serde(default)]
    #[serde(rename = "user_id")]
    _user_id: i64,
    #[serde(default)]
    #[serde(rename = "chat_id")]
    _chat_id: i64,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InternalLlmApiResponse {
    ok: bool,
    data: Option<InternalLlmTextData>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InternalLlmTextData {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TemporaryFixPlan {
    summary: String,
    #[serde(default)]
    plan_root: String,
    #[serde(default)]
    packages: Vec<TemporaryFixPackage>,
    #[serde(default)]
    files: Vec<TemporaryFixFile>,
    #[serde(default)]
    commands: Vec<TemporaryFixCommand>,
    #[serde(default)]
    notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TemporaryFixPackage {
    ecosystem: String,
    modules: Vec<String>,
    #[serde(default)]
    version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TemporaryFixFile {
    path: String,
    content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TemporaryFixCommand {
    runtime: String,
    script_path: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CommandRunRecord {
    runtime: String,
    script_path: String,
    cwd: String,
    args: Vec<String>,
    exit_code: i32,
    stdout: String,
    stderr: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PermanentExtensionPlan {
    skill_name: String,
    capability_summary: String,
    #[serde(default)]
    actions: Vec<String>,
    rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExternalSkillImplementation {
    readme_md: String,
    interface_md: String,
    main_rs: String,
}

#[derive(Debug, Clone, Serialize)]
struct ExternalSkillValidationReport {
    synced_docs: bool,
    cargo_check_ok: bool,
    smoke_test_ok: bool,
    smoke_status: String,
    smoke_text: String,
}

#[derive(Debug, Clone, Serialize)]
struct ExternalSkillRegistrationReport {
    workspace_member_added: bool,
    registry_entry_added: bool,
    switch_recorded_enabled: bool,
    matrix_admission_eligible: bool,
}

#[derive(Debug, Clone, Serialize)]
struct ExternalSkillEnableReport {
    switch_enabled: bool,
    release_build_ok: bool,
    release_binary_path: String,
    reload_required: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(&req.request_id, req.args).await {
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
            },
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

fn error_extra(error_kind: &str) -> Value {
    json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

async fn execute(request_id: &str, args: Value) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("assess_gap");

    match action {
        "assess_gap" => assess_gap(obj),
        "enable_external_skill" => enable_external_skill_action(obj),
        "implement_external_skill" => {
            implement_external_skill_action(request_id, obj).await
        }
        "register_external_skill" => register_external_skill_action(workspace_root(), obj),
        "validate_external_skill" => validate_external_skill_action(obj),
        "scaffold_external_skill" => scaffold_external_skill(workspace_root(), obj),
        "permanent_extension_plan" => permanent_extension_plan_action(request_id, obj).await,
        "temporary_fix_plan" => temporary_fix_plan_action(request_id, obj).await,
        "temporary_fix_execute" => temporary_fix_execute_action(request_id, obj).await,
        _ => Err(
            "unsupported action; use assess_gap|enable_external_skill|implement_external_skill|register_external_skill|validate_external_skill|permanent_extension_plan|scaffold_external_skill|temporary_fix_plan|temporary_fix_execute"
                .to_string(),
        ),
    }
}

fn assess_gap(obj: &Map<String, Value>) -> Result<(String, Value), String> {
    let request = required_string(obj, "request")?;
    let mode_hint = obj
        .get("mode_hint")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");
    let recommended_mode = match mode_hint {
        "temporary_fix" => "temporary_fix",
        "permanent_extension" => "permanent_extension",
        "manual_review" => "manual_review",
        "auto" => "manual_review",
        other => {
            return Err(format!(
            "invalid mode_hint: {other}; use auto|temporary_fix|permanent_extension|manual_review"
        ))
        }
    };

    let (text, next_actions): (&str, Vec<&str>) = match recommended_mode {
        "temporary_fix" => (
            "Recommend a temporary fix: use a bounded script/package plan and keep all changes task-local.",
            vec![
                "Use temporary_fix_plan first to generate a structured plan.",
                "Use temporary_fix_execute only with explicit confirm=true.",
                "Prefer task-local files under tmp/extension_manager/ and avoid repo changes.",
            ],
        ),
        "permanent_extension" => (
            "Recommend a permanent extension: scaffold a new isolated skill, keep it unregistered while testing, then register it after validation.",
            vec![
                "Generate a dedicated skill scaffold under external_skills/.",
                "Fill INTERFACE.md before registering the skill.",
                "Run sync_skill_docs.py and compile/smoke-test before registration writes the enabled config switch.",
            ],
        ),
        _ => (
            "Need an explicit extension mode before making changes.",
            vec![
                "Use temporary_fix for one-off execution with bounded scripts or language-level packages.",
                "Use permanent_extension for a reusable new skill.",
            ],
        ),
    };

    Ok((
        text.to_string(),
        json!({
            "action": "assess_gap",
            "request": request,
            "recommended_mode": recommended_mode,
            "default_enabled": false,
            "safe_defaults": {
                "does_not_modify_runtime": true,
                "does_not_enable_new_skill": true
            },
            "existing_execution_skills": ["run_cmd", "install_module", "package_manager", "write_file"],
            "next_actions": next_actions,
        }),
    ))
}

async fn temporary_fix_plan_action(
    request_id: &str,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let request = required_string(obj, "request")?;
    let plan = build_temporary_fix_plan(request_id, &request).await?;
    Ok((
        format!(
            "Temporary fix plan created with {} file(s), {} command(s), and {} package group(s).",
            plan.files.len(),
            plan.commands.len(),
            plan.packages.len()
        ),
        json!({
            "action": "temporary_fix_plan",
            "request": request,
            "plan": plan,
            "default_enabled": false
        }),
    ))
}

async fn permanent_extension_plan_action(
    request_id: &str,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let request = required_string(obj, "request")?;
    let plan = build_permanent_extension_plan(request_id, &request).await?;
    Ok((
        format!(
            "Permanent extension scaffold plan created for external_skills/{} with {} action(s).",
            plan.skill_name,
            plan.actions.len()
        ),
        json!({
            "action": "permanent_extension_plan",
            "request": request,
            "plan": plan,
            "default_enabled": false
        }),
    ))
}

async fn implement_external_skill_action(
    request_id: &str,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let request = required_string(obj, "request")?;
    let skill_name = required_string(obj, "skill_name")?;
    validate_identifier("skill_name", &skill_name)?;
    let capability_summary = required_string(obj, "capability_summary")?;
    let actions = extract_actions(obj)?;
    let repo_root = workspace_root();
    let skill_dir = repo_root.join("external_skills").join(&skill_name);
    if !skill_dir.exists() {
        return Err(format!(
            "skill scaffold does not exist yet: {}",
            skill_dir.display()
        ));
    }

    let implementation = build_external_skill_implementation(
        request_id,
        &request,
        &skill_name,
        &capability_summary,
        &actions,
    )
    .await?;
    let updated_files = write_external_skill_implementation(
        &skill_dir,
        &skill_name,
        &capability_summary,
        &actions,
        &implementation,
    )?;

    Ok((
        format!(
            "Implemented initial files for external_skills/{skill_name}. The skill is still unregistered and unavailable at runtime."
        ),
        json!({
            "action": "implement_external_skill",
            "skill_name": skill_name,
            "updated_files": updated_files,
            "default_enabled": false,
            "next_steps": [
                "Run python3 scripts/sync_skill_docs.py.",
                "Compile and smoke-test the generated skill.",
                "Register it with confirm=true only after verification passes; registration enables it in config."
            ]
        }),
    ))
}

fn validate_external_skill_action(obj: &Map<String, Value>) -> Result<(String, Value), String> {
    let skill_name = required_string(obj, "skill_name")?;
    validate_identifier("skill_name", &skill_name)?;
    let actions = extract_actions(obj)?;
    let repo_root = workspace_root();
    let report = validate_external_skill(&repo_root, &skill_name, &actions)?;
    Ok((
        format!(
            "Validated external_skills/{skill_name}: sync docs ok, cargo check ok, smoke test ok."
        ),
        json!({
            "action": "validate_external_skill",
            "skill_name": skill_name,
            "report": report,
            "default_enabled": false,
            "next_steps": [
                "Review the generated files before registration.",
                "Register the skill with confirm=true after human approval; registration enables it in config."
            ]
        }),
    ))
}

fn register_external_skill_action(
    repo_root: PathBuf,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    require_confirm(obj, "register_external_skill")?;
    let skill_name = required_string(obj, "skill_name")?;
    validate_identifier("skill_name", &skill_name)?;
    ensure_external_skill_scaffold_ready(&repo_root, &skill_name)?;

    let release_binary_path = external_skill_release_binary_path(&repo_root, &skill_name);
    let original_release_binary = fs::read(&release_binary_path).ok();
    let release_binary_path = build_external_skill_release_binary(&repo_root, &skill_name)?;
    let report = match register_external_skill(&repo_root, &skill_name) {
        Ok(report) => report,
        Err(err) => {
            match original_release_binary {
                Some(bytes) => {
                    let _ = fs::write(&release_binary_path, bytes);
                }
                None => {
                    let _ = fs::remove_file(&release_binary_path);
                }
            }
            return Err(err);
        }
    };
    Ok((
        format!(
            "Registered external skill `{skill_name}`, built its release binary, and enabled it in config. Reload skills or restart clawd before using it."
        ),
        json!({
            "action": "register_external_skill",
            "skill_name": skill_name,
            "report": report,
            "default_enabled": true,
            "release_build_ok": true,
            "release_binary_path": path_string(&release_binary_path),
            "reload_required": true,
            "next_steps": [
                "Reload skills via admin endpoint or restart clawd.",
                "Run a run_skill happy path before normal runtime use."
            ]
        }),
    ))
}

fn enable_external_skill_action(obj: &Map<String, Value>) -> Result<(String, Value), String> {
    require_confirm(obj, "enable_external_skill")?;
    let skill_name = required_string(obj, "skill_name")?;
    validate_identifier("skill_name", &skill_name)?;
    let repo_root = workspace_root();
    ensure_external_skill_scaffold_ready(&repo_root, &skill_name)?;

    let report = enable_external_skill(&repo_root, &skill_name)?;
    Ok((
        format!(
            "Enabled external skill `{skill_name}` in config and built its release binary. Reload skills or restart clawd before using it."
        ),
        json!({
            "action": "enable_external_skill",
            "skill_name": skill_name,
            "report": report,
            "default_enabled": true,
            "next_steps": [
                "Reload skills via admin endpoint or restart clawd.",
                "Keep human review in the loop before normal runtime use."
            ]
        }),
    ))
}

async fn temporary_fix_execute_action(
    request_id: &str,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let confirmed = obj
        .get("confirm")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !confirmed {
        return Err("temporary_fix_execute requires confirm=true".to_string());
    }

    let workspace_root = workspace_root();
    let allow_package_install = obj
        .get("allow_package_install")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut plan = if let Some(plan_value) = obj.get("plan") {
        serde_json::from_value::<TemporaryFixPlan>(plan_value.clone())
            .map_err(|err| format!("invalid plan: {err}"))?
    } else {
        let request = required_string(obj, "request")?;
        build_temporary_fix_plan(request_id, &request).await?
    };
    plan = normalize_plan(&workspace_root, request_id, plan)?;

    if !allow_package_install && !plan.packages.is_empty() {
        return Err(
            "temporary_fix_execute plan requires package installation; rerun with allow_package_install=true"
                .to_string(),
        );
    }

    let written_files = write_plan_files(&workspace_root, &plan)?;
    let installed_packages = if allow_package_install {
        install_plan_packages(&plan)?
    } else {
        Vec::new()
    };
    let command_runs = run_plan_commands(&workspace_root, &plan)?;
    let success = command_runs.iter().all(|run| run.exit_code == 0);
    if !success {
        let first_failure = command_runs
            .iter()
            .find(|run| run.exit_code != 0)
            .expect("failure record should exist");
        return Err(format!(
            "temporary fix command failed: runtime={} script={} exit={} stderr={}",
            first_failure.runtime,
            first_failure.script_path,
            first_failure.exit_code,
            truncate_preview(&first_failure.stderr, 240)
        ));
    }

    Ok((
        format!(
            "Temporary fix executed. Wrote {} file(s), installed {} package group(s), ran {} command(s).",
            written_files.len(),
            installed_packages.len(),
            command_runs.len()
        ),
        json!({
            "action": "temporary_fix_execute",
            "plan": plan,
            "written_files": written_files,
            "installed_packages": installed_packages,
            "command_runs": command_runs,
            "default_enabled": false
        }),
    ))
}

async fn build_temporary_fix_plan(
    request_id: &str,
    request: &str,
) -> Result<TemporaryFixPlan, String> {
    let raw = match llm_generate_temporary_fix_plan(request).await {
        Ok(raw) if !raw.trim().is_empty() => raw,
        Ok(_) => {
            return normalize_plan(
                &workspace_root(),
                request_id,
                fallback_temporary_fix_plan("provider_empty_content"),
            )
        }
        Err(_err) => {
            return normalize_plan(
                &workspace_root(),
                request_id,
                fallback_temporary_fix_plan("provider_error"),
            )
        }
    };
    match parse_temporary_fix_plan_from_text(&raw) {
        Ok(parsed) => normalize_plan(&workspace_root(), request_id, parsed),
        Err(_err) => normalize_plan(
            &workspace_root(),
            request_id,
            fallback_temporary_fix_plan("provider_invalid_plan"),
        ),
    }
}

fn fallback_temporary_fix_plan(reason_code: &str) -> TemporaryFixPlan {
    TemporaryFixPlan {
        summary: "temporary_fix_plan_dry_run_fallback".to_string(),
        plan_root: String::new(),
        packages: Vec::new(),
        files: Vec::new(),
        commands: Vec::new(),
        notes: vec![
            format!("reason_code={reason_code}"),
            "dry_run_only=true".to_string(),
            "does_not_scaffold=true".to_string(),
            "does_not_validate=true".to_string(),
            "does_not_register=true".to_string(),
        ],
    }
}

async fn build_permanent_extension_plan(
    request_id: &str,
    request: &str,
) -> Result<PermanentExtensionPlan, String> {
    let raw = llm_generate_permanent_extension_plan(request).await?;
    let parsed = parse_permanent_extension_plan_from_text(&raw)?;
    normalize_permanent_extension_plan(request_id, parsed)
}

async fn build_external_skill_implementation(
    request_id: &str,
    request: &str,
    skill_name: &str,
    capability_summary: &str,
    actions: &[String],
) -> Result<ExternalSkillImplementation, String> {
    let raw = llm_generate_external_skill_implementation(
        request,
        skill_name,
        capability_summary,
        actions,
    )
    .await?;
    let parsed = parse_external_skill_implementation_from_text(&raw)?;
    normalize_external_skill_implementation(request_id, skill_name, parsed)
}

async fn llm_generate_temporary_fix_plan(request: &str) -> Result<String, String> {
    let timeout_secs = extension_manager_timeout_seconds(90);
    let user_prompt = format!(
        "Create a bounded temporary-fix plan for this request.\n\nRequest:\n{}\n",
        request.trim()
    );
    if let Some(result) = internal_llm_generate(
        "skills/extension_manager/temporary_fix_plan",
        TEMP_FIX_SYSTEM_PROMPT,
        &user_prompt,
        0.2,
        2200,
        timeout_secs,
    )
    .await
    {
        return result;
    }

    let base_url = env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = claw_core::secrets::env_non_empty_resolved("OPENAI_API_KEY")
        .map_err(|err| format!("resolve OPENAI_API_KEY failed: {err}"))?
        .ok_or_else(|| "OPENAI_API_KEY is empty".to_string())?;
    let model = env::var("EXTENSION_MANAGER_MODEL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            env::var("OPENAI_MODEL")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| default_model_for_base_url(&base_url).to_string());
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "messages": [
            {"role":"system","content": TEMP_FIX_SYSTEM_PROMPT},
            {"role":"user","content": user_prompt}
        ],
        "temperature": 0.2,
        "max_tokens": 2200
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("build http client failed: {e}"))?;
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("temporary fix llm request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("temporary fix llm failed status={status}: {body}"));
    }
    let parsed: Value = resp
        .json()
        .await
        .map_err(|e| format!("parse llm response failed: {e}"))?;
    extract_assistant_text(&parsed).ok_or_else(|| {
        let raw = serde_json::to_string(&parsed).unwrap_or_default();
        let mut preview = raw.chars().take(320).collect::<String>();
        if raw.chars().count() > 320 {
            preview.push_str("...");
        }
        format!("temporary fix llm returned empty content (preview={preview})")
    })
}

async fn llm_generate_permanent_extension_plan(request: &str) -> Result<String, String> {
    let timeout_secs = extension_manager_timeout_seconds(90);
    let user_prompt = format!(
        "Create a reusable external skill scaffold plan for this request.\n\nRequest:\n{}\n",
        request.trim()
    );
    if let Some(result) = internal_llm_generate(
        "skills/extension_manager/permanent_extension_plan",
        PERMANENT_EXTENSION_SYSTEM_PROMPT,
        &user_prompt,
        0.2,
        1200,
        timeout_secs,
    )
    .await
    {
        return result;
    }

    let base_url = env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = claw_core::secrets::env_non_empty_resolved("OPENAI_API_KEY")
        .map_err(|err| format!("resolve OPENAI_API_KEY failed: {err}"))?
        .ok_or_else(|| "OPENAI_API_KEY is empty".to_string())?;
    let model = env::var("EXTENSION_MANAGER_MODEL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            env::var("OPENAI_MODEL")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| default_model_for_base_url(&base_url).to_string());
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "messages": [
            {"role":"system","content": PERMANENT_EXTENSION_SYSTEM_PROMPT},
            {"role":"user","content": user_prompt}
        ],
        "temperature": 0.2,
        "max_tokens": 1200
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("build http client failed: {e}"))?;
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("permanent extension llm request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "permanent extension llm failed status={status}: {body}"
        ));
    }
    let parsed: Value = resp
        .json()
        .await
        .map_err(|e| format!("parse llm response failed: {e}"))?;
    extract_assistant_text(&parsed).ok_or_else(|| {
        let raw = serde_json::to_string(&parsed).unwrap_or_default();
        let mut preview = raw.chars().take(320).collect::<String>();
        if raw.chars().count() > 320 {
            preview.push_str("...");
        }
        format!("permanent extension llm returned empty content (preview={preview})")
    })
}

async fn llm_generate_external_skill_implementation(
    request: &str,
    skill_name: &str,
    capability_summary: &str,
    actions: &[String],
) -> Result<String, String> {
    let timeout_secs = extension_manager_timeout_seconds(90);
    let user_prompt = format!(
        "Implement the first reusable external skill scaffold for this request.\n\nRequest:\n{}\n\nSkill name: {}\nCapability summary: {}\nActions: {}\n",
        request.trim(),
        skill_name,
        capability_summary.trim(),
        actions.join(", ")
    );
    if let Some(result) = internal_llm_generate(
        "skills/extension_manager/external_skill_implementation",
        SKILL_IMPLEMENTATION_SYSTEM_PROMPT,
        &user_prompt,
        0.2,
        3200,
        timeout_secs,
    )
    .await
    {
        return result;
    }

    let base_url = env::var("OPENAI_BASE_URL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = claw_core::secrets::env_non_empty_resolved("OPENAI_API_KEY")
        .map_err(|err| format!("resolve OPENAI_API_KEY failed: {err}"))?
        .ok_or_else(|| "OPENAI_API_KEY is empty".to_string())?;
    let model = env::var("EXTENSION_MANAGER_MODEL")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            env::var("OPENAI_MODEL")
                .ok()
                .filter(|v| !v.trim().is_empty())
        })
        .unwrap_or_else(|| default_model_for_base_url(&base_url).to_string());
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "messages": [
            {"role":"system","content": SKILL_IMPLEMENTATION_SYSTEM_PROMPT},
            {"role":"user","content": user_prompt}
        ],
        "temperature": 0.2,
        "max_tokens": 3200
    });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| format!("build http client failed: {e}"))?;
    let resp = client
        .post(url)
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("external skill implementation llm request failed: {e}"))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "external skill implementation llm failed status={status}: {body}"
        ));
    }
    let parsed: Value = resp
        .json()
        .await
        .map_err(|e| format!("parse llm response failed: {e}"))?;
    extract_assistant_text(&parsed).ok_or_else(|| {
        let raw = serde_json::to_string(&parsed).unwrap_or_default();
        let mut preview = raw.chars().take(320).collect::<String>();
        if raw.chars().count() > 320 {
            preview.push_str("...");
        }
        format!("external skill implementation llm returned empty content (preview={preview})")
    })
}

fn extension_manager_timeout_seconds(default_secs: u64) -> u64 {
    env::var("EXTENSION_MANAGER_TIMEOUT_SECONDS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
        .or_else(|| {
            env::var("SKILL_TIMEOUT_SECONDS")
                .ok()
                .and_then(|v| v.parse::<u64>().ok())
                .filter(|v| *v > 0)
        })
        .unwrap_or(default_secs)
}

async fn internal_llm_generate(
    prompt_source: &str,
    system_prompt: &str,
    user_prompt: &str,
    temperature: f64,
    max_tokens: u64,
    timeout_secs: u64,
) -> Option<Result<String, String>> {
    let url = env::var("RUSTCLAW_INTERNAL_LLM_URL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let token = env::var("RUSTCLAW_INTERNAL_LLM_TOKEN")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    let body = json!({
        "skill_name": "extension_manager",
        "prompt_source": prompt_source,
        "system": system_prompt,
        "user": user_prompt,
        "temperature": temperature,
        "max_tokens": max_tokens
    });
    let result = async {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(5)))
            .build()
            .map_err(|e| format!("build internal llm http client failed: {e}"))?;
        let resp = client
            .post(url)
            .header("x-rustclaw-internal-llm-token", token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("internal extension llm request failed: {e}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!(
                "internal extension llm failed status={status}: {body}"
            ));
        }
        let parsed: InternalLlmApiResponse = resp
            .json()
            .await
            .map_err(|e| format!("parse internal llm response failed: {e}"))?;
        if !parsed.ok {
            return Err(parsed
                .error
                .unwrap_or_else(|| "internal extension llm failed".to_string()));
        }
        parsed
            .data
            .map(|data| data.text)
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| "internal extension llm returned empty content".to_string())
    }
    .await;
    Some(result)
}

fn parse_temporary_fix_plan_from_text(raw: &str) -> Result<TemporaryFixPlan, String> {
    let candidate =
        parse_schema_validated_json_object(raw, temporary_fix_plan_schema(), "temporary fix plan")?;
    serde_json::from_value(candidate)
        .map_err(|err| format!("temporary fix plan shape invalid: {err}"))
}

fn parse_permanent_extension_plan_from_text(raw: &str) -> Result<PermanentExtensionPlan, String> {
    let candidate = parse_schema_validated_json_object(
        raw,
        permanent_extension_plan_schema(),
        "permanent extension plan",
    )?;
    serde_json::from_value(candidate)
        .map_err(|err| format!("permanent extension plan shape invalid: {err}"))
}

fn parse_external_skill_implementation_from_text(
    raw: &str,
) -> Result<ExternalSkillImplementation, String> {
    let candidate = parse_schema_validated_json_object(
        raw,
        external_skill_implementation_schema(),
        "external skill implementation",
    )?;
    serde_json::from_value(candidate)
        .map_err(|err| format!("external skill implementation shape invalid: {err}"))
}

fn temporary_fix_plan_schema() -> &'static Value {
    TEMPORARY_FIX_PLAN_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(TEMPORARY_FIX_PLAN_SCHEMA_RAW)
            .expect("temporary_fix_plan schema must be valid JSON")
    })
}

fn permanent_extension_plan_schema() -> &'static Value {
    PERMANENT_EXTENSION_PLAN_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(PERMANENT_EXTENSION_PLAN_SCHEMA_RAW)
            .expect("permanent_extension_plan schema must be valid JSON")
    })
}

fn external_skill_implementation_schema() -> &'static Value {
    EXTERNAL_SKILL_IMPLEMENTATION_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(EXTERNAL_SKILL_IMPLEMENTATION_SCHEMA_RAW)
            .expect("external_skill_implementation schema must be valid JSON")
    })
}

fn parse_schema_validated_json_object(
    raw: &str,
    schema: &Value,
    label: &str,
) -> Result<Value, String> {
    let candidate = if let Ok(value) = serde_json::from_str::<Value>(raw) {
        value
    } else {
        let extracted =
            extract_json_object(raw).ok_or_else(|| format!("{label} is not valid JSON"))?;
        serde_json::from_str::<Value>(&extracted)
            .map_err(|err| format!("{label} JSON parse failed: {err}"))?
    };
    validate_value_against_schema(&candidate, schema, "$")
        .map_err(|err| format!("{label} schema invalid: {err}"))?;
    Ok(candidate)
}

fn validate_value_against_schema(value: &Value, schema: &Value, path: &str) -> Result<(), String> {
    if let Some(kind) = schema.get("type").and_then(|v| v.as_str()) {
        match kind {
            "object" => {
                let object = value
                    .as_object()
                    .ok_or_else(|| format!("{path}: expected object"))?;
                let declared_fields = schema_declared_fields(schema);
                if !schema_allows_additional_properties(schema) {
                    let declared = declared_fields
                        .ok_or_else(|| format!("{path}: schema missing properties"))?;
                    if let Some(extra) = object.keys().find(|key| !declared.contains_key(*key)) {
                        return Err(format!("{path}.{extra}: unexpected field"));
                    }
                }
                if let Some(required) = schema.get("required").and_then(|v| v.as_array()) {
                    for field in required.iter().filter_map(|v| v.as_str()) {
                        if !object.contains_key(field) {
                            return Err(format!("{path}.{field}: missing required field"));
                        }
                    }
                }
                if let Some(properties) = declared_fields {
                    for (key, property_schema) in properties {
                        if let Some(child) = object.get(key) {
                            validate_value_against_schema(
                                child,
                                property_schema,
                                &format!("{path}.{key}"),
                            )?;
                        }
                    }
                }
            }
            "array" => {
                let array = value
                    .as_array()
                    .ok_or_else(|| format!("{path}: expected array"))?;
                if let Some(item_schema) = schema.get("items") {
                    for (idx, item) in array.iter().enumerate() {
                        validate_value_against_schema(
                            item,
                            item_schema,
                            &format!("{path}[{idx}]"),
                        )?;
                    }
                }
            }
            "string" => {
                let s = value
                    .as_str()
                    .ok_or_else(|| format!("{path}: expected string"))?;
                let min_length = schema
                    .get("minLength")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                if s.chars().count() < min_length {
                    return Err(format!(
                        "{path}: string shorter than minLength={min_length}"
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn schema_declared_fields(schema: &Value) -> Option<&serde_json::Map<String, Value>> {
    schema.get("properties")?.as_object()
}

fn schema_allows_additional_properties(schema: &Value) -> bool {
    schema
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn normalize_plan(
    workspace_root: &Path,
    request_id: &str,
    mut plan: TemporaryFixPlan,
) -> Result<TemporaryFixPlan, String> {
    if plan.summary.trim().is_empty() {
        return Err("temporary fix plan summary is required".to_string());
    }
    if plan.packages.len() > 2 {
        return Err("temporary fix plan may install at most 2 package groups".to_string());
    }
    if plan.files.len() > 3 {
        return Err("temporary fix plan may create at most 3 files".to_string());
    }
    if plan.commands.len() > 3 {
        return Err("temporary fix plan may run at most 3 commands".to_string());
    }

    let default_root = build_plan_root(request_id);
    let root_rel = if plan.plan_root.trim().is_empty() {
        default_root
    } else {
        normalize_plan_root(&plan.plan_root)?
    };
    plan.plan_root = root_rel.clone();

    for package in &mut plan.packages {
        package.ecosystem = normalize_ecosystem(&package.ecosystem)?;
        if package.modules.is_empty() {
            return Err("temporary fix package list cannot be empty".to_string());
        }
        if package.modules.len() > 8 {
            return Err("temporary fix package group is too large".to_string());
        }
        for module in &package.modules {
            if !is_safe_module_name(module) {
                return Err(format!("invalid module name: {module}"));
            }
        }
        if let Some(version) = package.version.as_deref() {
            if !is_safe_module_name(version) {
                return Err(format!("invalid package version: {version}"));
            }
        }
    }

    let mut normalized_file_paths = Vec::new();
    let mut total_content_bytes = 0usize;
    for file in &mut plan.files {
        file.path = normalize_plan_member_path(&root_rel, &file.path)?;
        total_content_bytes += file.content.len();
        if total_content_bytes > 160_000 {
            return Err("temporary fix plan file content is too large".to_string());
        }
        normalized_file_paths.push(file.path.clone());
        let abs = resolve_workspace_path(workspace_root, &file.path)?;
        ensure_within_workspace(workspace_root, &abs)?;
    }

    for command in &mut plan.commands {
        command.runtime = normalize_runtime(&command.runtime)?;
        command.script_path = normalize_plan_member_path(&root_rel, &command.script_path)?;
        if !normalized_file_paths
            .iter()
            .any(|path| path == &command.script_path)
        {
            return Err(format!(
                "temporary fix command must reference a generated script file: {}",
                command.script_path
            ));
        }
        if command.args.len() > 16 {
            return Err("temporary fix command has too many args".to_string());
        }
        for arg in &command.args {
            if arg.contains('\n') || arg.contains('\r') {
                return Err("temporary fix command args must be single-line strings".to_string());
            }
        }
        let cwd = command.cwd.as_deref().unwrap_or(".");
        let normalized_cwd = normalize_workspace_relative_path(cwd)?;
        let abs_cwd = workspace_root.join(&normalized_cwd);
        ensure_within_workspace(workspace_root, &abs_cwd)?;
        command.cwd = Some(path_string(&normalized_cwd));
    }

    Ok(plan)
}

fn normalize_permanent_extension_plan(
    request_id: &str,
    mut plan: PermanentExtensionPlan,
) -> Result<PermanentExtensionPlan, String> {
    if plan.skill_name.trim().is_empty() {
        let fallback = request_id
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .take(12)
            .collect::<String>();
        plan.skill_name = if fallback.is_empty() {
            "generated_extension".to_string()
        } else {
            format!("generated_{}", fallback.to_ascii_lowercase())
        };
    }
    plan.skill_name = plan.skill_name.trim().to_string();
    validate_identifier("skill_name", &plan.skill_name)?;
    if plan.capability_summary.trim().is_empty() {
        return Err("permanent extension capability_summary is required".to_string());
    }
    let normalized_actions = if plan.actions.is_empty() {
        vec!["todo_action".to_string()]
    } else {
        plan.actions
            .into_iter()
            .map(|action| action.trim().to_string())
            .filter(|action| !action.is_empty())
            .collect::<Vec<_>>()
    };
    if normalized_actions.is_empty() {
        plan.actions = vec!["todo_action".to_string()];
    } else {
        if normalized_actions.len() > 12 {
            return Err("too many actions; limit is 12".to_string());
        }
        for action in &normalized_actions {
            validate_identifier("action", action)?;
        }
        plan.actions = normalized_actions;
    }
    if plan.rationale.trim().is_empty() {
        plan.rationale = "Reusable capability requested.".to_string();
    }
    Ok(plan)
}

fn normalize_external_skill_implementation(
    request_id: &str,
    skill_name: &str,
    mut implementation: ExternalSkillImplementation,
) -> Result<ExternalSkillImplementation, String> {
    for (label, content, limit) in [
        ("readme_md", &mut implementation.readme_md, 16_000usize),
        (
            "interface_md",
            &mut implementation.interface_md,
            48_000usize,
        ),
        ("main_rs", &mut implementation.main_rs, 120_000usize),
    ] {
        let trimmed = content.trim();
        if trimmed.is_empty() {
            return Err(format!("external skill implementation {label} is required"));
        }
        if trimmed.len() > limit {
            return Err(format!(
                "external skill implementation {label} is too large for {}",
                if request_id.trim().is_empty() {
                    skill_name
                } else {
                    request_id
                }
            ));
        }
        *content = trimmed.to_string();
    }
    Ok(implementation)
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
