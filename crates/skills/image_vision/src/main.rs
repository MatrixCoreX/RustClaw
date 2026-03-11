use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
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
}

#[derive(Debug, Clone, Deserialize)]
struct VendorConfig {
    base_url: String,
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
    timeout_seconds: Option<u64>,
    #[serde(default)]
    max_images: Option<usize>,
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

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let cfg = load_root_config();
    let workspace_root = workspace_root();

    for line in stdin.lock().lines() {
        let line = line?;
        let parsed: Result<Req, _> = serde_json::from_str(&line);
        let resp = match parsed {
            Ok(req) => match execute(&cfg, &workspace_root, req.args) {
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

fn execute(
    cfg: &RootConfig,
    workspace_root: &Path,
    args: Value,
) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;

    let action = parse_action(obj)?;
    let images = parse_images(obj, workspace_root)?;
    let max_images = cfg.image_vision.max_images.unwrap_or(6).max(1);
    if images.is_empty() {
        return Err("at least one image is required".to_string());
    }
    if images.len() > max_images {
        return Err(format!(
            "too many images: {}, max={max_images}",
            images.len()
        ));
    }
    if action == "compare" && images.len() < 2 {
        return Err("compare requires at least two images".to_string());
    }

    let detail_level = obj
        .get("detail_level")
        .and_then(|v| v.as_str())
        .unwrap_or("normal");
    let response_language = obj
        .get("response_language")
        .or_else(|| obj.get("language"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let schema = obj.get("schema").cloned();
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.image_vision.timeout_seconds.unwrap_or(90))
        .max(5)
        .min(300);
    let max_input_bytes = cfg.image_vision.max_input_bytes.unwrap_or(10 * 1024 * 1024);

    let requested_vendor = obj.get("vendor").and_then(|v| v.as_str());
    let requested_model = obj.get("model").and_then(|v| v.as_str());
    let vendors = vendor_order(
        requested_vendor,
        cfg.image_vision.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    if vendors.is_empty() {
        return Err("no vendor configured".to_string());
    }

    let mut last_err = String::new();
    for vendor in vendors {
        let prompt = build_prompt(
            workspace_root,
            prompt_vendor_name_for_vendor(vendor),
            &action,
            detail_level,
            schema.as_ref(),
            response_language,
        );
        let config_default_model = first_model_candidate(
            cfg.image_vision.default_model.as_deref(),
            vendor_models(&cfg.image_vision, vendor),
            cfg.image_vision.models.as_ref(),
        );
        match call_vendor_vision(
            vendor,
            cfg,
            requested_model.or(config_default_model),
            timeout_seconds,
            &prompt,
            &images,
            max_input_bytes,
        ) {
            Ok((text, model, model_kind)) => {
                let extra = json!({
                    "provider": vendor_name(vendor),
                    "model": model,
                    "model_kind": model_kind,
                    "latency_ms": 0,
                    "outputs": [{"type":"text","preview": truncate(&text, 800)}]
                });
                return Ok((text, extra));
            }
            Err(err) => last_err = err,
        }
    }

    Err(format!("all providers failed: {last_err}"))
}

fn first_model_candidate<'a>(
    default_model: Option<&'a str>,
    vendor_models: Option<&'a Vec<String>>,
    models: Option<&'a Vec<String>>,
) -> Option<&'a str> {
    if let Some(v) = default_model.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    if let Some(v) =
        vendor_models.and_then(|list| list.iter().map(|s| s.trim()).find(|v| !v.is_empty()))
    {
        return Some(v);
    }
    models.and_then(|list| list.iter().map(|s| s.trim()).find(|v| !v.is_empty()))
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
    }
}

fn parse_action(obj: &Map<String, Value>) -> Result<String, String> {
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("describe")
        .trim()
        .to_ascii_lowercase();
    match action.as_str() {
        "describe" | "extract" | "compare" | "screenshot_summary" => Ok(action),
        _ => Err("unsupported action; use describe|extract|compare|screenshot_summary".to_string()),
    }
}

fn parse_images(
    obj: &Map<String, Value>,
    workspace_root: &Path,
) -> Result<Vec<ImageSource>, String> {
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
    if !joined.starts_with(workspace_root) {
        return Err("image path is outside workspace".to_string());
    }
    Ok(joined)
}

fn build_prompt(
    workspace_root: &Path,
    prompt_vendor: &str,
    action: &str,
    detail_level: &str,
    schema: Option<&Value>,
    response_language: Option<&str>,
) -> String {
    let template = load_image_vision_prompt_template(workspace_root, prompt_vendor);
    let task_instruction =
        action_instruction(workspace_root, prompt_vendor, action, detail_level, schema);
    let schema_hint = schema
        .map(|s| s.to_string())
        .unwrap_or_else(|| "none".to_string());
    let language_hint = response_language
        .map(|s| {
            load_prompt_fragment(
                workspace_root,
                prompt_vendor,
                "prompts/image_vision_language_hint_with_target.md",
                DEFAULT_IMAGE_VISION_LANGUAGE_HINT_WITH_TARGET_TEMPLATE,
            )
            .replace("__RESPONSE_LANGUAGE__", s)
        })
        .unwrap_or_else(|| {
            load_prompt_fragment(
                workspace_root,
                prompt_vendor,
                "prompts/image_vision_language_hint_default.md",
                DEFAULT_IMAGE_VISION_LANGUAGE_HINT_DEFAULT_TEMPLATE,
            )
        });
    template
        .replace("__ACTION__", action)
        .replace("__DETAIL_LEVEL__", detail_level)
        .replace("__TASK_INSTRUCTION__", &task_instruction)
        .replace("__SCHEMA_HINT__", &schema_hint)
        .replace("__LANGUAGE_HINT__", &language_hint)
}

fn action_instruction(
    workspace_root: &Path,
    prompt_vendor: &str,
    action: &str,
    detail_level: &str,
    schema: Option<&Value>,
) -> String {
    match action {
        "describe" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_describe.md",
            DEFAULT_IMAGE_VISION_ACTION_DESCRIBE_TEMPLATE,
        )
        .replace("__DETAIL_LEVEL__", detail_level),
        "compare" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_compare.md",
            DEFAULT_IMAGE_VISION_ACTION_COMPARE_TEMPLATE,
        ),
        "screenshot_summary" => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_screenshot_summary.md",
            DEFAULT_IMAGE_VISION_ACTION_SCREENSHOT_SUMMARY_TEMPLATE,
        ),
        "extract" => {
            if let Some(s) = schema {
                load_prompt_fragment(
                    workspace_root,
                    prompt_vendor,
                    "prompts/image_vision_action_extract_with_schema.md",
                    DEFAULT_IMAGE_VISION_ACTION_EXTRACT_WITH_SCHEMA_TEMPLATE,
                )
                .replace("__SCHEMA__", &s.to_string())
            } else {
                load_prompt_fragment(
                    workspace_root,
                    prompt_vendor,
                    "prompts/image_vision_action_extract_default.md",
                    DEFAULT_IMAGE_VISION_ACTION_EXTRACT_DEFAULT_TEMPLATE,
                )
            }
        }
        _ => load_prompt_fragment(
            workspace_root,
            prompt_vendor,
            "prompts/image_vision_action_fallback.md",
            DEFAULT_IMAGE_VISION_ACTION_FALLBACK_TEMPLATE,
        ),
    }
}

fn normalize_prompt_vendor_name(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "claude".to_string(),
        "google" | "gemini" => "google".to_string(),
        "openai" => "openai".to_string(),
        "grok" | "xai" => "grok".to_string(),
        "deepseek" => "deepseek".to_string(),
        "qwen" => "qwen".to_string(),
        "minimax" => "minimax".to_string(),
        _ => "default".to_string(),
    }
}

fn prompt_vendor_name_for_vendor(vendor: VendorKind) -> &'static str {
    match vendor {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "claude",
        VendorKind::Grok => "grok",
        VendorKind::DeepSeek => "deepseek",
        VendorKind::Qwen => "qwen",
        VendorKind::MiniMax => "minimax",
    }
}

fn resolve_prompt_rel_path_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> String {
    let trimmed = rel_path.trim();
    if trimmed.is_empty() || !trimmed.starts_with("prompts/") {
        return trimmed.to_string();
    }
    let suffix = trimmed.trim_start_matches("prompts/");
    let vendor_name = normalize_prompt_vendor_name(vendor);
    let vendor_candidate = format!("prompts/vendors/{vendor_name}/{suffix}");
    if workspace_root.join(&vendor_candidate).is_file() {
        return vendor_candidate;
    }
    let default_candidate = format!("prompts/vendors/default/{suffix}");
    if vendor_name != "default" && workspace_root.join(&default_candidate).is_file() {
        return default_candidate;
    }
    trimmed.to_string()
}

fn load_prompt_fragment(
    workspace_root: &Path,
    vendor: &str,
    relative_path: &str,
    default_template: &str,
) -> String {
    let resolved_path = resolve_prompt_rel_path_for_vendor(workspace_root, vendor, relative_path);
    match std::fs::read_to_string(workspace_root.join(resolved_path)) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => default_template.to_string(),
    }
}

fn load_image_vision_prompt_template(workspace_root: &Path, vendor: &str) -> String {
    load_prompt_fragment(
        workspace_root,
        vendor,
        "prompts/image_vision_prompt.md",
        DEFAULT_IMAGE_VISION_PROMPT_TEMPLATE,
    )
}

const DEFAULT_IMAGE_VISION_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_prompt.md");
const DEFAULT_IMAGE_VISION_LANGUAGE_HINT_WITH_TARGET_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_language_hint_with_target.md");
const DEFAULT_IMAGE_VISION_LANGUAGE_HINT_DEFAULT_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_language_hint_default.md");
const DEFAULT_IMAGE_VISION_ACTION_DESCRIBE_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_action_describe.md");
const DEFAULT_IMAGE_VISION_ACTION_COMPARE_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_action_compare.md");
const DEFAULT_IMAGE_VISION_ACTION_SCREENSHOT_SUMMARY_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_action_screenshot_summary.md");
const DEFAULT_IMAGE_VISION_ACTION_EXTRACT_WITH_SCHEMA_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_action_extract_with_schema.md");
const DEFAULT_IMAGE_VISION_ACTION_EXTRACT_DEFAULT_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_action_extract_default.md");
const DEFAULT_IMAGE_VISION_ACTION_FALLBACK_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/image_vision_action_fallback.md");

fn call_vendor_vision(
    vendor: VendorKind,
    cfg: &RootConfig,
    requested_model: Option<&str>,
    timeout_seconds: u64,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<(String, String, &'static str), String> {
    let mode = resolve_adapter_mode(&cfg.image_vision);
    let (vendor_name, vcfg) = resolve_vendor_config(cfg, vendor)?;
    check_api_key(vendor_name, &vcfg.api_key)?;
    match vendor {
        VendorKind::OpenAI => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build openai client failed: {err}"))?;
            let text = openai_vision(&client, vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "native"))
        }
        VendorKind::Google => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build google client failed: {err}"))?;
            let text = google_vision(&client, vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "native"))
        }
        VendorKind::Anthropic => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build anthropic client failed: {err}"))?;
            let text = anthropic_vision(&client, vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "native"))
        }
        VendorKind::Grok | VendorKind::DeepSeek => {
            if mode == AdapterMode::Native {
                return Err(format!(
                    "{vendor_name} native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                ));
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
            let text = openai_vision(&client, vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "compat"))
        }
        VendorKind::MiniMax => {
            if mode == AdapterMode::Native {
                return Err(
                    "minimax native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build minimax client failed: {err}"))?;
            let text = openai_vision(&client, vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "compat"))
        }
        VendorKind::Qwen => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(
                    timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30)),
                ))
                .build()
                .map_err(|err| format!("build qwen client failed: {err}"))?;
            if mode == AdapterMode::Native {
                return Err(
                    "qwen native vision adapter is not implemented; use image_vision.adapter_mode=compat"
                        .to_string(),
                );
            }
            let text = openai_vision(&client, vcfg, &model, prompt, images, max_input_bytes)?;
            Ok((text, model, "compat"))
        }
    }
}

fn resolve_adapter_mode(cfg: &ImageSkillConfig) -> AdapterMode {
    match cfg
        .adapter_mode
        .as_deref()
        .map(str::trim)
        .unwrap_or("auto")
        .to_ascii_lowercase()
        .as_str()
    {
        "native" => AdapterMode::Native,
        "compat" | "compatible" => AdapterMode::Compat,
        _ => AdapterMode::Auto,
    }
}

fn openai_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    for image in images {
        let url = match image {
            ImageSource::Url(s) => s.to_string(),
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                format!("data:{mime};base64,{}", STANDARD.encode(bytes))
            }
            ImageSource::Base64(s) => normalize_base64_image(s),
        };
        content.push(json!({"type":"image_url","image_url":{"url":url}}));
    }
    let body = json!({
        "model": model,
        "messages": [{"role":"user","content":content}],
        "temperature": 0.2
    });
    let url = format!("{}/chat/completions", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .map_err(|err| format!("openai request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse openai response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "openai error status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
    }
    if let Some(s) = v
        .get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
    {
        return Ok(s.to_string());
    }
    Err(format!(
        "openai response missing text: {}",
        truncate(&v.to_string(), 400)
    ))
}

fn google_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut parts = vec![json!({"text":prompt})];
    for image in images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                parts.push(json!({"inline_data":{"mime_type":mime,"data":STANDARD.encode(bytes)}}));
            }
            ImageSource::Base64(s) => {
                let (mime, data) = split_image_data(s);
                parts.push(json!({"inline_data":{"mime_type":mime,"data":data}}));
            }
            ImageSource::Url(u) => {
                parts.push(json!({"text": format!("Image URL: {u}")}));
            }
        }
    }
    let body = json!({"contents":[{"parts":parts}]});
    let url = format!(
        "{}/models/{}:generateContent?key={}",
        trim_trailing_slash(&cfg.base_url),
        model,
        cfg.api_key
    );
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|err| format!("google request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse google response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "google error status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
    }
    let mut out = String::new();
    if let Some(parts) = v
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "google response missing text: {}",
            truncate(&v.to_string(), 400)
        ));
    }
    Ok(out)
}

fn anthropic_vision(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    images: &[ImageSource],
    max_input_bytes: usize,
) -> Result<String, String> {
    let mut content = vec![json!({"type":"text","text":prompt})];
    for image in images {
        match image {
            ImageSource::Path(p) => {
                let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
                if bytes.len() > max_input_bytes {
                    return Err(format!("image too large: {} bytes", bytes.len()));
                }
                let mime = guess_mime_from_path(p);
                content.push(json!({
                    "type":"image",
                    "source":{"type":"base64","media_type":mime,"data":STANDARD.encode(bytes)}
                }));
            }
            ImageSource::Base64(s) => {
                let (mime, data) = split_image_data(s);
                content.push(json!({
                    "type":"image",
                    "source":{"type":"base64","media_type":mime,"data":data}
                }));
            }
            ImageSource::Url(u) => {
                content.push(json!({"type":"text","text":format!("Image URL reference: {u}")}));
            }
        }
    }
    let body = json!({
        "model": model,
        "max_tokens": 1024,
        "messages": [{"role":"user","content":content}]
    });
    let url = format!("{}/messages", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .header("x-api-key", &cfg.api_key)
        .header("anthropic-version", "2023-06-01")
        .json(&body)
        .send()
        .map_err(|err| format!("anthropic request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse anthropic response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "anthropic error status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
    }
    let mut out = String::new();
    if let Some(arr) = v.get("content").and_then(|c| c.as_array()) {
        for item in arr {
            if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(t);
            }
        }
    }
    if out.is_empty() {
        return Err(format!(
            "anthropic response missing text: {}",
            truncate(&v.to_string(), 400)
        ));
    }
    Ok(out)
}

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
    cfg
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn vendor_order(
    requested: Option<&str>,
    section_default: Option<&str>,
    selected_vendor: Option<&str>,
) -> Vec<VendorKind> {
    if let Some(req) = requested.and_then(parse_vendor) {
        return vec![req];
    }
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for name in [section_default, selected_vendor].into_iter().flatten() {
        if let Some(v) = parse_vendor(name) {
            if seen.insert(v) {
                out.push(v);
            }
        }
    }
    if !out.is_empty() {
        return out;
    }
    for v in [
        VendorKind::OpenAI,
        VendorKind::Google,
        VendorKind::Anthropic,
        VendorKind::Grok,
        VendorKind::DeepSeek,
        VendorKind::Qwen,
        VendorKind::MiniMax,
    ] {
        if seen.insert(v) {
            out.push(v);
        }
    }
    out
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
    }
}

fn resolve_vendor_config<'a>(
    cfg: &'a RootConfig,
    vendor: VendorKind,
) -> Result<(&'static str, &'a VendorConfig), String> {
    let section = &cfg.image_vision.providers;
    match vendor {
        VendorKind::OpenAI => section
            .openai
            .as_ref()
            .or(cfg.llm.openai.as_ref())
            .map(|v| ("openai", v))
            .ok_or_else(|| "openai config missing".to_string()),
        VendorKind::Google => section
            .google
            .as_ref()
            .or(cfg.llm.google.as_ref())
            .map(|v| ("google", v))
            .ok_or_else(|| "google config missing".to_string()),
        VendorKind::Anthropic => section
            .anthropic
            .as_ref()
            .or(cfg.llm.anthropic.as_ref())
            .map(|v| ("anthropic", v))
            .ok_or_else(|| "anthropic config missing".to_string()),
        VendorKind::Grok => section
            .grok
            .as_ref()
            .or(cfg.llm.grok.as_ref())
            .map(|v| ("grok", v))
            .ok_or_else(|| "grok config missing".to_string()),
        VendorKind::DeepSeek => section
            .deepseek
            .as_ref()
            .or(cfg.llm.deepseek.as_ref())
            .map(|v| ("deepseek", v))
            .ok_or_else(|| "deepseek config missing".to_string()),
        VendorKind::Qwen => section
            .qwen
            .as_ref()
            .or(cfg.llm.qwen.as_ref())
            .map(|v| ("qwen", v))
            .ok_or_else(|| "qwen config missing".to_string()),
        VendorKind::MiniMax => section
            .minimax
            .as_ref()
            .or(cfg.llm.minimax.as_ref())
            .map(|v| ("minimax", v))
            .ok_or_else(|| "minimax config missing".to_string()),
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

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_vendor_ok() {
        assert_eq!(parse_vendor("openai"), Some(VendorKind::OpenAI));
        assert_eq!(parse_vendor("gemini"), Some(VendorKind::Google));
        assert_eq!(parse_vendor("claude"), Some(VendorKind::Anthropic));
        assert_eq!(parse_vendor("qwen"), Some(VendorKind::Qwen));
    }

    #[test]
    fn split_data_url() {
        let (mime, data) = split_image_data("data:image/jpeg;base64,abc");
        assert_eq!(mime, "image/jpeg");
        assert_eq!(data, "abc");
    }
}
