use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use claw_core::prompt_layers;
use regex::Regex;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
mod prompting;
mod providers;

use prompting::*;
use providers::*;

const SKILL_NAME: &str = "image_vision";

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
    /// Generic runner context from `clawd` / `skill-runner` (not `args._memory`).
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Resp {
    request_id: String,
    status: String,
    text: String,
    extra: Option<Value>,
    error_text: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct RootConfig {
    #[serde(default)]
    llm: LlmConfig,
    #[serde(default)]
    image_vision: ImageSkillConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LlmConfig {
    #[serde(default)]
    selected_vendor: Option<String>,
    #[serde(default)]
    openai: Option<VendorConfig>,
    #[serde(default)]
    google: Option<VendorConfig>,
    #[serde(default)]
    anthropic: Option<VendorConfig>,
    #[serde(default)]
    grok: Option<VendorConfig>,
    #[serde(default)]
    deepseek: Option<VendorConfig>,
    #[serde(default)]
    qwen: Option<VendorConfig>,
    #[serde(default)]
    minimax: Option<VendorConfig>,
    #[serde(default)]
    mimo: Option<VendorConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct VendorConfig {
    #[serde(default)]
    base_url: String,
    #[serde(default)]
    api_key: String,
    model: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ImageSkillConfig {
    #[serde(default)]
    default_vendor: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    models: Option<Vec<String>>,
    #[serde(default)]
    openai_models: Option<Vec<String>>,
    #[serde(default)]
    google_models: Option<Vec<String>>,
    #[serde(default)]
    anthropic_models: Option<Vec<String>>,
    #[serde(default)]
    grok_models: Option<Vec<String>>,
    #[serde(default)]
    deepseek_models: Option<Vec<String>>,
    #[serde(default)]
    qwen_models: Option<Vec<String>>,
    #[serde(default)]
    minimax_models: Option<Vec<String>>,
    #[serde(default)]
    mimo_models: Option<Vec<String>>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    max_input_bytes: Option<usize>,
    #[serde(default)]
    adapter_mode: Option<String>,
    #[serde(default)]
    providers: ImageProviderOverrides,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ImageProviderOverrides {
    #[serde(default)]
    openai: Option<VendorConfig>,
    #[serde(default)]
    google: Option<VendorConfig>,
    #[serde(default)]
    anthropic: Option<VendorConfig>,
    #[serde(default)]
    grok: Option<VendorConfig>,
    #[serde(default)]
    deepseek: Option<VendorConfig>,
    #[serde(default)]
    qwen: Option<VendorConfig>,
    #[serde(default)]
    minimax: Option<VendorConfig>,
    #[serde(default)]
    mimo: Option<VendorConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum VendorKind {
    OpenAI,
    Google,
    Anthropic,
    Grok,
    DeepSeek,
    Qwen,
    MiniMax,
    Mimo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterMode {
    Auto,
    Native,
    Compat,
}

#[derive(Debug, Clone)]
enum ImageSource {
    Path(PathBuf),
    Url(String),
    Base64(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageDescribeOut {
    summary: String,
    #[serde(default)]
    objects: Vec<String>,
    #[serde(default)]
    visible_text: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageCompareOut {
    summary: String,
    #[serde(default)]
    similarities: Vec<String>,
    #[serde(default)]
    differences: Vec<String>,
    #[serde(default)]
    notable_changes: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageScreenshotSummaryOut {
    purpose: String,
    #[serde(default)]
    critical_text: Vec<String>,
    #[serde(default)]
    warnings: Vec<String>,
    #[serde(default)]
    next_actions: Vec<String>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageTextPageOut {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ImageTextExtractionOut {
    #[serde(default)]
    pages: Vec<ImageTextPageOut>,
    #[serde(default)]
    uncertainties: Vec<String>,
}

#[derive(Debug, Clone)]
enum StructuredNarrativeActionOutput {
    Describe(ImageDescribeOut),
    Compare(ImageCompareOut),
    ScreenshotSummary(ImageScreenshotSummaryOut),
    ExtractText(ImageTextExtractionOut),
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let cfg = load_root_config();
    let workspace_root = workspace_root();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(&cfg, &workspace_root, req.args, req.context.as_ref()) {
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
        "error_code": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn execute(
    cfg: &RootConfig,
    workspace_root: &Path,
    args: Value,
    runner_context: Option<&Value>,
) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;

    let action = parse_action(obj)?;
    let images = parse_images(obj, workspace_root)?;
    validate_image_request(&action, &images)?;

    let detail_level = obj
        .get("detail_level")
        .and_then(|v| v.as_str())
        .unwrap_or("normal");
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.image_vision.timeout_seconds.unwrap_or(90))
        .max(5)
        .min(300);
    let response_language = resolve_effective_response_language(
        cfg,
        workspace_root,
        obj,
        runner_context,
        timeout_seconds,
    );
    let schema = obj.get("schema").cloned();
    let user_instruction = obj
        .get("instruction")
        .or_else(|| obj.get("query"))
        .or_else(|| obj.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let max_input_bytes = cfg.image_vision.max_input_bytes.unwrap_or(10 * 1024 * 1024);

    let requested_vendor = obj.get("vendor").and_then(|v| v.as_str());
    let requested_model = obj.get("model").and_then(|v| v.as_str());
    let vendors = vendor_order(requested_vendor, cfg.image_vision.default_vendor.as_deref());
    if vendors.is_empty() {
        return Err("no vendor configured".to_string());
    }

    let mut attempt_errors = Vec::new();
    for vendor in vendors {
        let prompt = build_prompt(
            workspace_root,
            prompt_vendor_name_for_vendor(vendor),
            &action,
            detail_level,
            schema.as_ref(),
            response_language.as_deref(),
            user_instruction,
        );
        let config_default_model =
            select_model_override(&cfg.image_vision, vendor, requested_model);
        match call_vendor_vision(
            vendor,
            cfg,
            config_default_model,
            timeout_seconds,
            &prompt,
            &images,
            max_input_bytes,
        ) {
            Ok((text, model, model_kind)) => {
                let structured = parse_structured_narrative_action_output(action.as_str(), &text);
                let text = structured
                    .as_ref()
                    .map(|out| {
                        render_structured_narrative_action_output(
                            out,
                            response_language.as_deref(),
                            action == "describe" && user_instruction.is_none(),
                        )
                    })
                    .unwrap_or(text);
                let text = maybe_rewrite_image_vision_text_for_target_language(
                    cfg,
                    workspace_root,
                    action.as_str(),
                    response_language.as_deref(),
                    text,
                    timeout_seconds,
                );
                let text = strip_think_blocks(&text).trim().to_string();
                let text = if action == "extract_text" {
                    normalize_extracted_text_newlines(&text)
                } else {
                    text
                };
                if action == "extract_text"
                    && !image_text_output_has_visible_text(structured.as_ref(), &text)
                {
                    attempt_errors.push(format!(
                        "{}: multimodal model returned no visible text",
                        vendor_name(vendor)
                    ));
                    continue;
                }
                let (text, recognition_review) = if action == "extract_text" {
                    let raw_recognized_text = text.clone();
                    let (reviewed, metadata) =
                        review_recognized_image_text(cfg, workspace_root, text, timeout_seconds);
                    (
                        normalize_extracted_text_newlines(&reviewed),
                        Some((metadata, raw_recognized_text)),
                    )
                } else {
                    (text, None)
                };
                let mut extra = json!({
                    "provider": vendor_name(vendor),
                    "model": model,
                    "model_kind": model_kind,
                    "latency_ms": 0,
                    "outputs": [{"type":"text","preview": truncate(&text, 800)}],
                    "schema_validated": structured.is_some()
                });
                if let Some(structured) = structured {
                    extra["structured"] = structured.to_json_value();
                }
                if action == "extract_text" {
                    let (mut review, raw_recognized_text) =
                        recognition_review.unwrap_or_else(|| {
                            (
                                json!({"status": "fallback_raw", "reviewed_by_model": false}),
                                text.clone(),
                            )
                        });
                    if review
                        .get("reviewed_by_model")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        attach_raw_text_artifact(
                            runner_context,
                            obj,
                            &raw_recognized_text,
                            &mut review,
                        )?;
                    }
                    extra["recognition"] = json!({
                        "source": "multimodal_model",
                        "provider": vendor_name(vendor),
                        "model": model,
                        "model_kind": model_kind,
                        "reviewed_by_model": review
                            .get("reviewed_by_model")
                            .and_then(Value::as_bool)
                            .unwrap_or(false),
                    });
                    extra["recognition_review"] = review;
                    attach_text_artifact(runner_context, obj, &text, &mut extra)?;
                }
                return Ok((text, extra));
            }
            Err(err) => {
                attempt_errors.push(format!("{}: {}", vendor_name(vendor), truncate(&err, 300)));
            }
        }
    }

    Err(format!(
        "all providers failed: {}",
        attempt_errors.join("; ")
    ))
}

fn image_text_output_has_visible_text(
    structured: Option<&StructuredNarrativeActionOutput>,
    rendered_text: &str,
) -> bool {
    match structured {
        Some(StructuredNarrativeActionOutput::ExtractText(output)) => {
            output.pages.iter().any(|page| !page.text.trim().is_empty())
        }
        _ => !rendered_text.trim().is_empty(),
    }
}

fn vendor_models<'a>(cfg: &'a ImageSkillConfig, vendor: VendorKind) -> Option<&'a Vec<String>> {
    match vendor {
        VendorKind::OpenAI => cfg.openai_models.as_ref(),
        VendorKind::Google => cfg.google_models.as_ref(),
        VendorKind::Anthropic => cfg.anthropic_models.as_ref(),
        VendorKind::Grok => cfg.grok_models.as_ref(),
        VendorKind::DeepSeek => cfg.deepseek_models.as_ref(),
        VendorKind::Qwen => cfg.qwen_models.as_ref(),
        VendorKind::MiniMax => cfg.minimax_models.as_ref(),
        VendorKind::Mimo => cfg.mimo_models.as_ref(),
    }
}

fn select_model_override<'a>(
    cfg: &'a ImageSkillConfig,
    vendor: VendorKind,
    requested_model: Option<&'a str>,
) -> Option<&'a str> {
    if let Some(v) = requested_model.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    if cfg.default_vendor.as_deref().and_then(parse_vendor) == Some(vendor) {
        if let Some(v) = cfg
            .default_model
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Some(v);
        }
        if let Some(v) = first_model_from_list(vendor_models(cfg, vendor)) {
            return Some(v);
        }
        return first_model_from_list(cfg.models.as_ref());
    }
    first_model_from_list(vendor_models(cfg, vendor))
}

fn first_model_from_list(models: Option<&Vec<String>>) -> Option<&str> {
    models.and_then(|list| list.iter().map(|s| s.trim()).find(|v| !v.is_empty()))
}

fn parse_action(obj: &Map<String, Value>) -> Result<String, String> {
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("describe")
        .trim()
        .to_ascii_lowercase();
    match action.as_str() {
        "analyze" => Ok("describe".to_string()),
        "describe" | "extract" | "extract_text" | "compare" | "screenshot_summary" => Ok(action),
        _ => Err(
            "unsupported action; use describe|extract|extract_text|compare|screenshot_summary"
                .to_string(),
        ),
    }
}

fn bool_arg(obj: &Map<String, Value>, name: &str, default: bool) -> Result<bool, String> {
    match obj.get(name) {
        None => Ok(default),
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => Err(format!("{name} must be boolean")),
    }
}

fn text_output_name(obj: &Map<String, Value>) -> Result<String, String> {
    let requested = obj
        .get("output_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("image_text_ai.txt");
    let path = Path::new(requested);
    if requested.len() > 180
        || requested.chars().any(char::is_control)
        || path.file_name().and_then(|value| value.to_str()) != Some(requested)
        || !requested.to_ascii_lowercase().ends_with(".txt")
    {
        return Err("output_name must be a plain .txt filename".to_string());
    }
    Ok(requested.to_string())
}

fn text_artifact_output_directory(runner_context: Option<&Value>) -> Result<PathBuf, String> {
    let raw = runner_context
        .and_then(Value::as_object)
        .and_then(|context| context.get("artifact_output_directory"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "runtime artifact output directory is unavailable".to_string())?;
    let directory = PathBuf::from(raw);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("create text artifact directory failed: {error}"))?;
    Ok(directory)
}

fn attach_text_artifact(
    runner_context: Option<&Value>,
    obj: &Map<String, Value>,
    text: &str,
    extra: &mut Value,
) -> Result<(), String> {
    let save_text_file = bool_arg(obj, "save_text_file", true)?;
    let deliver_to_user = bool_arg(obj, "deliver_to_user", true)?;
    extra["delivery"] = json!({
        "intent": if save_text_file && deliver_to_user {
            "artifact"
        } else if save_text_file {
            "save_only"
        } else {
            "inline"
        },
        "deliver_to_user": save_text_file && deliver_to_user,
    });
    if !save_text_file {
        return Ok(());
    }

    let output_directory = text_artifact_output_directory(runner_context)?;
    let output_name = text_output_name(obj)?;
    let output_path = output_directory.join(&output_name);
    let mut bytes = text.as_bytes().to_vec();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    fs::write(&output_path, &bytes)
        .map_err(|error| format!("write extracted text artifact failed: {error}"))?;
    let artifact = json!({
        "path": output_path,
        "filename": output_name,
        "kind": "text",
        "mime_type": "text/plain; charset=utf-8",
        "size_bytes": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "recognition_source": "multimodal_model",
    });
    extra["output_directory"] = json!(output_directory);
    if deliver_to_user {
        extra["output_path"] = artifact["path"].clone();
        extra["artifacts"] = json!([artifact]);
    } else {
        extra["artifacts"] = json!([]);
        extra["saved_files"] = json!([artifact]);
    }
    Ok(())
}

fn attach_raw_text_artifact(
    runner_context: Option<&Value>,
    obj: &Map<String, Value>,
    raw_text: &str,
    review: &mut Value,
) -> Result<(), String> {
    if !bool_arg(obj, "save_text_file", true)? {
        return Ok(());
    }
    let output_directory = text_artifact_output_directory(runner_context)?;
    let output_name = text_output_name(obj)?;
    let output_path = Path::new(&output_name);
    let stem = output_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("image_text_ai");
    let raw_name = format!("{stem}_raw.txt");
    let raw_path = output_directory.join(&raw_name);
    let mut bytes = raw_text.as_bytes().to_vec();
    if !bytes.ends_with(b"\n") {
        bytes.push(b'\n');
    }
    fs::write(&raw_path, &bytes)
        .map_err(|error| format!("write raw extracted text artifact failed: {error}"))?;
    review["raw_artifact"] = json!({
        "path": raw_path,
        "filename": raw_name,
        "kind": "text",
        "mime_type": "text/plain; charset=utf-8",
        "size_bytes": bytes.len(),
        "sha256": format!("{:x}", Sha256::digest(&bytes)),
        "deliver_to_user": false,
        "recognition_source": "multimodal_model",
    });
    Ok(())
}

fn validate_image_request(action: &str, images: &[ImageSource]) -> Result<(), String> {
    if images.is_empty() {
        return Err("at least one image is required".to_string());
    }
    if action == "compare" && images.len() < 2 {
        return Err("compare requires at least two images".to_string());
    }
    Ok(())
}

fn parse_images(
    obj: &Map<String, Value>,
    workspace_root: &Path,
) -> Result<Vec<ImageSource>, String> {
    if obj.contains_key("images") && obj.contains_key("image") {
        return Err("provide either images or image, not both".to_string());
    }
    let mut out = Vec::new();
    if let Some(arr) = obj.get("images").and_then(|v| v.as_array()) {
        for item in arr {
            out.push(parse_one_image(item, workspace_root)?);
        }
    } else if let Some(v) = obj.get("image") {
        out.push(parse_one_image(v, workspace_root)?);
    }
    Ok(out)
}

fn parse_one_image(v: &Value, workspace_root: &Path) -> Result<ImageSource, String> {
    if let Some(s) = v.as_str() {
        return parse_image_from_str(s, workspace_root);
    }
    let obj = v
        .as_object()
        .ok_or_else(|| "image entry must be string or object".to_string())?;
    if let Some(source) = parse_text_wrapped_path(obj, workspace_root)? {
        return Ok(source);
    }
    if let Some(s) = obj.get("path").and_then(|v| v.as_str()) {
        let p = to_workspace_path(workspace_root, s)?;
        return Ok(ImageSource::Path(p));
    }
    if let Some(s) = obj.get("url").and_then(|v| v.as_str()) {
        if !s.starts_with("http://") && !s.starts_with("https://") {
            return Err("image.url must start with http:// or https://".to_string());
        }
        return Ok(ImageSource::Url(s.to_string()));
    }
    if let Some(s) = obj.get("base64").and_then(|v| v.as_str()) {
        return Ok(ImageSource::Base64(s.to_string()));
    }
    Err("image object requires path|url|base64".to_string())
}

fn parse_text_wrapped_path(
    obj: &Map<String, Value>,
    workspace_root: &Path,
) -> Result<Option<ImageSource>, String> {
    let Some(encoded) = (obj.len() == 1)
        .then(|| obj.get("$text").and_then(Value::as_str))
        .flatten()
    else {
        return Ok(None);
    };
    if encoded.len() > 8192 {
        return Err("image text wrapper is too large".to_string());
    }
    let decoded = serde_json::from_str::<Value>(encoded)
        .map_err(|_| "image text wrapper must contain a JSON path object".to_string())?;
    let decoded = decoded
        .as_object()
        .filter(|decoded| decoded.len() == 1)
        .ok_or_else(|| "image text wrapper must contain only path".to_string())?;
    let path = decoded
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| "image text wrapper must contain only path".to_string())?;
    Ok(Some(ImageSource::Path(to_workspace_path(
        workspace_root,
        path,
    )?)))
}

fn parse_image_from_str(s: &str, workspace_root: &Path) -> Result<ImageSource, String> {
    let t = s.trim();
    if t.starts_with("http://") || t.starts_with("https://") {
        return Ok(ImageSource::Url(t.to_string()));
    }
    if t.starts_with("data:image/") {
        return Ok(ImageSource::Base64(t.to_string()));
    }
    Ok(ImageSource::Path(to_workspace_path(workspace_root, t)?))
}

fn to_workspace_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let p = Path::new(input);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    if !runtime_allows_external_paths() && !joined.starts_with(workspace_root) {
        return Err("image path is outside workspace".to_string());
    }
    Ok(joined)
}

fn non_empty_str_from_value(v: Option<&Value>) -> Option<String> {
    v.and_then(|x| x.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

/// Matches `clawd` `preferred_response_language` ordering: last matching preference wins.

fn load_root_config() -> RootConfig {
    let root = workspace_root();
    let core_cfg = match std::fs::read_to_string(root.join("configs/config.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
    {
        Some(v) => v,
        None => toml::Value::Table(toml::map::Map::new()),
    };
    let image_cfg = match std::fs::read_to_string(root.join("configs/image.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
    {
        Some(v) => v,
        None => toml::Value::Table(toml::map::Map::new()),
    };

    let mut cfg = RootConfig::default();
    if let Some(v) = core_cfg.get("llm").cloned() {
        if let Ok(parsed) = v.try_into::<LlmConfig>() {
            cfg.llm = parsed;
        }
    }
    if let Some(v) = image_cfg.get("image_vision").cloned() {
        if let Ok(parsed) = v.try_into::<ImageSkillConfig>() {
            cfg.image_vision = parsed;
        }
    }
    apply_env_overrides(&mut cfg);
    cfg
}

fn env_non_empty(key: &str) -> Option<String> {
    claw_core::secrets::env_non_empty_resolved_or_none(key)
}

fn apply_vendor_api_key_env(target: &mut Option<VendorConfig>, key: &str) {
    if let (Some(value), Some(cfg)) = (env_non_empty(key), target.as_mut()) {
        cfg.api_key = value;
    }
}

fn apply_env_overrides(cfg: &mut RootConfig) {
    apply_vendor_api_key_env(&mut cfg.llm.openai, "OPENAI_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.google, "GOOGLE_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.anthropic, "ANTHROPIC_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.grok, "GROK_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.deepseek, "DEEPSEEK_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.qwen, "QWEN_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.minimax, "MINIMAX_API_KEY");
    apply_vendor_api_key_env(&mut cfg.llm.mimo, "MIMO_API_KEY");

    apply_selected_openai_compat_env(cfg);

    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.openai,
        "IMAGE_VISION_OPENAI_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.google,
        "IMAGE_VISION_GOOGLE_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.anthropic,
        "IMAGE_VISION_ANTHROPIC_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.grok,
        "IMAGE_VISION_GROK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.deepseek,
        "IMAGE_VISION_DEEPSEEK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.qwen,
        "IMAGE_VISION_QWEN_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.minimax,
        "IMAGE_VISION_MINIMAX_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_vision.providers.mimo,
        "IMAGE_VISION_MIMO_API_KEY",
    );
}

fn apply_selected_openai_compat_env(cfg: &mut RootConfig) {
    let Some(vendor) = cfg.llm.selected_vendor.as_deref().and_then(parse_vendor) else {
        return;
    };
    let base_url = env_non_empty("OPENAI_BASE_URL");
    let model = env_non_empty("OPENAI_MODEL");
    let api_key = env_non_empty("OPENAI_API_KEY");
    let Some(target) = llm_vendor_config_mut(&mut cfg.llm, vendor) else {
        return;
    };
    if let Some(value) = base_url {
        target.base_url = value;
    }
    if let Some(value) = model {
        target.model = value;
    }
    if let Some(value) = api_key {
        target.api_key = value;
    }
}

fn llm_vendor_config_mut(cfg: &mut LlmConfig, vendor: VendorKind) -> Option<&mut VendorConfig> {
    match vendor {
        VendorKind::OpenAI => cfg.openai.as_mut(),
        VendorKind::Google => cfg.google.as_mut(),
        VendorKind::Anthropic => cfg.anthropic.as_mut(),
        VendorKind::Grok => cfg.grok.as_mut(),
        VendorKind::DeepSeek => cfg.deepseek.as_mut(),
        VendorKind::Qwen => cfg.qwen.as_mut(),
        VendorKind::MiniMax => cfg.minimax.as_mut(),
        VendorKind::Mimo => cfg.mimo.as_mut(),
    }
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn runtime_allows_external_paths() -> bool {
    std::env::var("APP_ALLOW_PATH_OUTSIDE_WORKSPACE").is_ok_and(|value| value == "1")
}

fn vendor_order(requested: Option<&str>, section_default: Option<&str>) -> Vec<VendorKind> {
    if let Some(req) = requested.and_then(parse_vendor) {
        return vec![req];
    }
    section_default.and_then(parse_vendor).into_iter().collect()
}

fn parse_vendor(name: &str) -> Option<VendorKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "openai" => Some(VendorKind::OpenAI),
        "google" | "gemini" => Some(VendorKind::Google),
        "anthropic" | "claude" => Some(VendorKind::Anthropic),
        "grok" | "xai" => Some(VendorKind::Grok),
        "deepseek" => Some(VendorKind::DeepSeek),
        "qwen" => Some(VendorKind::Qwen),
        "minimax" => Some(VendorKind::MiniMax),
        "mimo" | "xiaomi" => Some(VendorKind::Mimo),
        _ => None,
    }
}

fn vendor_name(v: VendorKind) -> &'static str {
    match v {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "anthropic",
        VendorKind::Grok => "grok",
        VendorKind::DeepSeek => "deepseek",
        VendorKind::Qwen => "qwen",
        VendorKind::MiniMax => "minimax",
        VendorKind::Mimo => "mimo",
    }
}

fn provider_config_with_shared_connection(
    provider: Option<&VendorConfig>,
    shared: Option<&VendorConfig>,
) -> Option<VendorConfig> {
    match (provider, shared) {
        (Some(provider), Some(shared)) => {
            let mut merged = provider.clone();
            if merged.base_url.trim().is_empty() && !shared.base_url.trim().is_empty() {
                merged.base_url = shared.base_url.clone();
            }
            if check_api_key("", &merged.api_key).is_err()
                && check_api_key("", &shared.api_key).is_ok()
            {
                merged.api_key = shared.api_key.clone();
            }
            Some(merged)
        }
        (Some(provider), None) => Some(provider.clone()),
        (None, Some(shared)) => Some(shared.clone()),
        (None, None) => None,
    }
}

fn resolve_vendor_config(
    cfg: &RootConfig,
    vendor: VendorKind,
) -> Result<(&'static str, VendorConfig), String> {
    let section = &cfg.image_vision.providers;
    match vendor {
        VendorKind::OpenAI => {
            provider_config_with_shared_connection(section.openai.as_ref(), cfg.llm.openai.as_ref())
                .map(|v| ("openai", v))
                .ok_or_else(|| "openai config missing".to_string())
        }
        VendorKind::Google => {
            provider_config_with_shared_connection(section.google.as_ref(), cfg.llm.google.as_ref())
                .map(|v| ("google", v))
                .ok_or_else(|| "google config missing".to_string())
        }
        VendorKind::Anthropic => provider_config_with_shared_connection(
            section.anthropic.as_ref(),
            cfg.llm.anthropic.as_ref(),
        )
        .map(|v| ("anthropic", v))
        .ok_or_else(|| "anthropic config missing".to_string()),
        VendorKind::Grok => {
            provider_config_with_shared_connection(section.grok.as_ref(), cfg.llm.grok.as_ref())
                .map(|v| ("grok", v))
                .ok_or_else(|| "grok config missing".to_string())
        }
        VendorKind::DeepSeek => provider_config_with_shared_connection(
            section.deepseek.as_ref(),
            cfg.llm.deepseek.as_ref(),
        )
        .map(|v| ("deepseek", v))
        .ok_or_else(|| "deepseek config missing".to_string()),
        VendorKind::Qwen => {
            provider_config_with_shared_connection(section.qwen.as_ref(), cfg.llm.qwen.as_ref())
                .map(|v| ("qwen", v))
                .ok_or_else(|| "qwen config missing".to_string())
        }
        VendorKind::MiniMax => provider_config_with_shared_connection(
            section.minimax.as_ref(),
            cfg.llm.minimax.as_ref(),
        )
        .map(|v| ("minimax", v))
        .ok_or_else(|| "minimax config missing".to_string()),
        VendorKind::Mimo => {
            provider_config_with_shared_connection(section.mimo.as_ref(), cfg.llm.mimo.as_ref())
                .map(|v| ("mimo", v))
                .ok_or_else(|| "mimo config missing".to_string())
        }
    }
}

fn check_api_key(vendor: &str, key: &str) -> Result<(), String> {
    let t = key.trim();
    if t.is_empty() || t.starts_with("REPLACE_ME_") {
        return Err(format!("{vendor} api key is not configured"));
    }
    Ok(())
}

fn guess_mime_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/png",
    }
}

fn normalize_base64_image(raw: &str) -> String {
    let t = raw.trim();
    if t.starts_with("data:image/") {
        t.to_string()
    } else {
        format!("data:image/png;base64,{t}")
    }
}

fn split_image_data(raw: &str) -> (String, String) {
    let t = raw.trim();
    if let Some(body) = t.strip_prefix("data:") {
        let mut parts = body.splitn(2, ',');
        let meta = parts.next().unwrap_or("image/png;base64");
        let data = parts.next().unwrap_or("").to_string();
        let mime = meta
            .split(';')
            .next()
            .unwrap_or("image/png")
            .to_string()
            .trim()
            .to_string();
        return (mime, data);
    }
    ("image/png".to_string(), t.to_string())
}

fn trim_trailing_slash(v: &str) -> String {
    v.trim_end_matches('/').to_string()
}

fn provider_error_excerpt(value: &Value, max: usize) -> String {
    truncate(&redact_sensitive_inline(&value.to_string()), max)
}

fn redact_sensitive_inline(text: &str) -> String {
    static SECRET_ASSIGNMENT_RE: OnceLock<Regex> = OnceLock::new();
    static BEARER_RE: OnceLock<Regex> = OnceLock::new();
    static OPENAI_STYLE_KEY_RE: OnceLock<Regex> = OnceLock::new();

    let out = SECRET_ASSIGNMENT_RE
        .get_or_init(|| {
            Regex::new(
                r#"(?i)("?(?:api[_-]?key|api[_-]?secret|access[_-]?token|refresh[_-]?token|authorization|client[_-]?secret|secret|token)"?\s*(?::|=)\s*"?)[A-Za-z0-9_./+=*\-]{8,}"?"#,
            )
            .expect("secret assignment redaction regex compiles")
        })
        .replace_all(text, "${1}[REDACTED]")
        .to_string();
    let out = BEARER_RE
        .get_or_init(|| {
            Regex::new(r#"(?i)(bearer\s+)[A-Za-z0-9_./+=*\-]{8,}"#)
                .expect("bearer redaction regex compiles")
        })
        .replace_all(&out, "${1}[REDACTED]")
        .to_string();
    OPENAI_STYLE_KEY_RE
        .get_or_init(|| {
            Regex::new(r#"sk-[A-Za-z0-9_*\-]{6,}"#)
                .expect("openai-style key redaction regex compiles")
        })
        .replace_all(&out, "[REDACTED_API_KEY]")
        .to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "..."
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
