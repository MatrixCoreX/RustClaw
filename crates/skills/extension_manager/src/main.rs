use std::env;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

static TEMPORARY_FIX_PLAN_SCHEMA: OnceLock<Value> = OnceLock::new();
static PERMANENT_EXTENSION_PLAN_SCHEMA: OnceLock<Value> = OnceLock::new();
static EXTERNAL_SKILL_IMPLEMENTATION_SCHEMA: OnceLock<Value> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    #[allow(dead_code)]
    #[serde(default)]
    context: Option<Value>,
    #[allow(dead_code)]
    #[serde(default)]
    user_id: i64,
    #[allow(dead_code)]
    #[serde(default)]
    chat_id: i64,
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
                    extra: None,
                    error_text: Some(err),
                },
            },
            Err(err) => Resp {
                request_id: "unknown".to_string(),
                status: "error".to_string(),
                text: String::new(),
                extra: None,
                error_text: Some(format!("invalid input: {err}")),
            },
        };
        writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
        stdout.flush()?;
    }

    Ok(())
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
    let raw = llm_generate_temporary_fix_plan(request).await?;
    let parsed = parse_temporary_fix_plan_from_text(&raw)?;
    normalize_plan(&workspace_root(), request_id, parsed)
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

fn write_plan_files(workspace_root: &Path, plan: &TemporaryFixPlan) -> Result<Vec<String>, String> {
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

fn write_external_skill_implementation(
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

fn validate_external_skill(
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

fn register_external_skill(
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

fn external_skill_binary_name(skill_name: &str) -> String {
    format!("{}-skill", skill_name.replace('_', "-"))
}

fn external_skill_release_binary_path(repo_root: &Path, skill_name: &str) -> PathBuf {
    repo_root
        .join("target/release")
        .join(external_skill_binary_name(skill_name))
}

fn build_external_skill_release_binary(
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

fn enable_external_skill(
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

fn ensure_scaffold_or_missing(path: &Path, scaffold_content: &str) -> Result<(), String> {
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

fn prepare_validation_staging_dir(skill_name: &str) -> Result<PathBuf, String> {
    prepare_staging_dir("validate", skill_name)
}

fn prepare_staging_dir(prefix: &str, skill_name: &str) -> Result<PathBuf, String> {
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

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
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
struct ProcessCapture {
    exit_code: i32,
    stdout: String,
    stderr: String,
}

fn run_command_capture(
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

fn best_process_output(output: &ProcessCapture) -> String {
    if !output.stderr.trim().is_empty() {
        truncate_preview(&output.stderr, 400)
    } else if !output.stdout.trim().is_empty() {
        truncate_preview(&output.stdout, 400)
    } else {
        format!("exit={}", output.exit_code)
    }
}

fn parse_single_json_line(raw: &str) -> Option<Value> {
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

fn add_workspace_member_text(raw: &str, member_path: &str) -> Result<(String, bool), String> {
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

fn conservative_registry_entry_text(skill_name: &str) -> String {
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

fn add_registry_entry_text(raw: &str, skill_name: &str) -> (String, bool) {
    if raw.contains(&format!("name = \"{skill_name}\"")) {
        return (raw.to_string(), false);
    }
    let mut updated = raw.trim_end().to_string();
    updated.push_str(&conservative_registry_entry_text(skill_name));
    updated.push('\n');
    (updated, true)
}

fn collect_skill_switches_from_text(raw: &str) -> std::collections::BTreeMap<String, bool> {
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

fn render_switches_inline_table(switches: &std::collections::BTreeMap<String, bool>) -> String {
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

fn upsert_skill_switches_line(raw: &str, rendered_line: &str) -> String {
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

fn install_plan_packages(plan: &TemporaryFixPlan) -> Result<Vec<Value>, String> {
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

fn run_plan_commands(
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

fn scaffold_external_skill(
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

fn extract_actions(obj: &Map<String, Value>) -> Result<Vec<String>, String> {
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

fn required_string(obj: &Map<String, Value>, key: &str) -> Result<String, String> {
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

fn require_confirm(obj: &Map<String, Value>, action: &str) -> Result<(), String> {
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

fn validate_identifier(label: &str, value: &str) -> Result<(), String> {
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

fn write_new_file(path: &Path, content: &str) -> std::io::Result<()> {
    if path.exists() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "file already exists",
        ));
    }
    fs::write(path, content)
}

fn ensure_external_skill_scaffold_ready(repo_root: &Path, skill_name: &str) -> Result<(), String> {
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

fn repo_root() -> Result<PathBuf, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
    root.canonicalize()
        .map_err(|err| format!("resolve repo root failed: {err}"))
}

fn workspace_root() -> PathBuf {
    env::var("WORKSPACE_ROOT")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            repo_root()
                .unwrap_or_else(|_| env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        })
}

fn build_plan_root(request_id: &str) -> String {
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

fn normalize_plan_root(input: &str) -> Result<String, String> {
    let normalized = normalize_workspace_relative_path(input)?;
    let prefix = Path::new("tmp").join("extension_manager");
    if !normalized.starts_with(&prefix) {
        return Err("temporary fix plan_root must stay under tmp/extension_manager".to_string());
    }
    Ok(path_string(&normalized))
}

fn normalize_plan_member_path(plan_root: &str, input: &str) -> Result<String, String> {
    let normalized = normalize_workspace_relative_path(input)?;
    let root = Path::new(plan_root);
    let final_path = if normalized.starts_with(root) {
        normalized
    } else {
        root.join(normalized)
    };
    Ok(path_string(&final_path))
}

fn normalize_workspace_relative_path(input: &str) -> Result<PathBuf, String> {
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

fn resolve_workspace_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let relative = normalize_workspace_relative_path(input)?;
    let joined = workspace_root.join(relative);
    ensure_within_workspace(workspace_root, &joined)?;
    Ok(joined)
}

fn ensure_within_workspace(workspace_root: &Path, candidate: &Path) -> Result<(), String> {
    if candidate.starts_with(workspace_root) {
        Ok(())
    } else {
        Err("resolved path escapes workspace root".to_string())
    }
}

fn normalize_runtime(input: &str) -> Result<String, String> {
    match input.trim() {
        "python3" | "bash" | "sh" | "node" => Ok(input.trim().to_string()),
        other => Err(format!(
            "unsupported runtime: {other}; use python3|bash|sh|node"
        )),
    }
}

fn normalize_ecosystem(input: &str) -> Result<String, String> {
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

fn ensure_installer_available(ecosystem: &str) -> Result<(), String> {
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

fn run_install_command(
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

fn is_safe_module_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn extract_assistant_text(parsed: &Value) -> Option<String> {
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

fn append_text_candidates(value: &Value, out: &mut Vec<String>) {
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

fn extract_json_object(raw: &str) -> Option<String> {
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

fn default_model_for_base_url(base_url: &str) -> &'static str {
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

fn truncate_preview(raw: &str, max_chars: usize) -> String {
    let mut preview = raw.chars().take(max_chars).collect::<String>();
    if raw.chars().count() > max_chars {
        preview.push_str("...");
    }
    preview
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn readme_template(skill_name: &str, actions: &[String]) -> String {
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

fn cargo_toml_template(binary_name: &str) -> String {
    format!(
        "[package]\nname = \"{binary_name}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[[bin]]\nname = \"{binary_name}\"\npath = \"src/main.rs\"\n\n[dependencies]\nanyhow = \"1\"\nserde = {{ version = \"1\", features = [\"derive\"] }}\nserde_json = \"1\"\n"
    )
}

fn interface_template(skill_name: &str, capability_summary: &str, actions: &[String]) -> String {
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

fn main_rs_template(actions: &[String]) -> String {
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
        "use std::io::{{self, BufRead, Write}};\n\nuse serde::{{Deserialize, Serialize}};\nuse serde_json::{{json, Value}};\n\n#[derive(Debug, Deserialize)]\nstruct Req {{\n    request_id: String,\n    args: Value,\n    #[serde(default)]\n    context: Option<Value>,\n    #[allow(dead_code)]\n    #[serde(default)]\n    user_id: i64,\n    #[allow(dead_code)]\n    #[serde(default)]\n    chat_id: i64,\n}}\n\n#[derive(Debug, Serialize)]\nstruct Resp {{\n    request_id: String,\n    status: String,\n    text: String,\n    #[serde(skip_serializing_if = \"Option::is_none\")]\n    extra: Option<Value>,\n    error_text: Option<String>,\n}}\n\nfn main() -> anyhow::Result<()> {{\n    let stdin = io::stdin();\n    let mut stdout = io::stdout();\n\n    for line in stdin.lock().lines() {{\n        let line = line?;\n        let parsed: Result<Req, _> = serde_json::from_str(&line);\n        let resp = match parsed {{\n            Ok(req) => match execute(req.args) {{\n                Ok((text, extra)) => Resp {{\n                    request_id: req.request_id,\n                    status: \"ok\".to_string(),\n                    text,\n                    extra: Some(extra),\n                    error_text: None,\n                }},\n                Err(err) => Resp {{\n                    request_id: req.request_id,\n                    status: \"error\".to_string(),\n                    text: String::new(),\n                    extra: None,\n                    error_text: Some(err),\n                }},\n            }},\n            Err(err) => Resp {{\n                request_id: \"unknown\".to_string(),\n                status: \"error\".to_string(),\n                text: String::new(),\n                extra: None,\n                error_text: Some(format!(\"invalid input: {{err}}\")),\n            }},\n        }};\n        writeln!(stdout, \"{{}}\", serde_json::to_string(&resp)?)?;\n        stdout.flush()?;\n    }}\n\n    Ok(())\n}}\n\nfn execute(args: Value) -> Result<(String, Value), String> {{\n    let obj = args\n        .as_object()\n        .ok_or_else(|| \"args must be object\".to_string())?;\n    let action = obj\n        .get(\"action\")\n        .and_then(|v| v.as_str())\n        .ok_or_else(|| \"action is required\".to_string())?;\n\n    match action {{\n{match_arms}\n        _ => Err(format!(\"unsupported action: {{action}}; use {supported_actions}\")),\n    }}\n}}\n"
    )
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
