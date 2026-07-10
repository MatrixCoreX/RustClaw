use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use claw_core::prompt_layers;
use hmac::{Hmac, Mac};
use reqwest::blocking::{multipart, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Sha1;
mod i18n;
mod providers;

use i18n::*;
use providers::*;

const IMAGE_REFERENCE_RESOLVER_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/image_reference_resolver.schema.json");
const SKILL_NAME: &str = "image_edit";

static IMAGE_REFERENCE_RESOLVER_SCHEMA: OnceLock<Value> = OnceLock::new();

#[derive(Debug, Deserialize)]
struct Req {
    request_id: String,
    args: Value,
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
    image_edit: ImageSkillConfig,
    #[serde(default)]
    command_intent: CommandIntentConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct CommandIntentConfig {
    #[serde(default)]
    default_locale: Option<String>,
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
    #[serde(default)]
    api_key: String,
    model: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct ImageSkillConfig {
    #[serde(default)]
    default_output_dir: Option<String>,
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
    native_models: Option<Vec<String>>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    max_input_bytes: Option<usize>,
    #[serde(default)]
    allow_compat_adapters: bool,
    #[serde(default)]
    adapter_mode: Option<String>,
    #[serde(default)]
    qwen_native_base_url: Option<String>,
    #[serde(default)]
    qwen_native_function: Option<String>,
    #[serde(default)]
    local_auto_upload_enabled: bool,
    #[serde(default)]
    oss_access_key_id: Option<String>,
    #[serde(default)]
    oss_access_key_secret: Option<String>,
    #[serde(default)]
    oss_bucket: Option<String>,
    #[serde(default)]
    oss_endpoint: Option<String>,
    #[serde(default)]
    oss_object_prefix: Option<String>,
    #[serde(default)]
    oss_url_ttl_seconds: Option<u64>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    i18n_path: Option<String>,
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
        "error_kind": error_kind,
        "message_key": format!("skill.{}.{}", SKILL_NAME, error_kind),
        "retryable": false,
    })
}

fn object_has_image_source(obj: &serde_json::Map<String, Value>) -> bool {
    ["path", "url", "base64"].iter().any(|key| {
        obj.get(*key)
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    })
}

fn first_image_from_images_array(obj: &serde_json::Map<String, Value>) -> Option<Value> {
    let arr = obj.get("images")?.as_array()?;
    for it in arr {
        if let Some(source) = it.as_object().filter(|m| object_has_image_source(m)) {
            return Some(Value::Object(source.clone()));
        }
        if let Some(p) = it
            .as_str()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            return Some(Value::String(p));
        }
    }
    None
}

fn image_edit_args_has_image(obj: &serde_json::Map<String, Value>) -> bool {
    let image_obj_has_source = obj
        .get("image")
        .and_then(|v| v.as_object())
        .map(object_has_image_source)
        .unwrap_or(false);
    let image_str = obj
        .get("image")
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    let images_array_has_path = obj
        .get("images")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|it| {
                it.as_object().map(object_has_image_source).unwrap_or(false)
                    || it.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)
            })
        })
        .unwrap_or(false);
    image_obj_has_source || image_str || images_array_has_path
}

fn recent_image_paths_from_context(ctx: Option<&Value>) -> Vec<String> {
    let Some(ctx) = ctx else {
        return Vec::new();
    };
    let Some(arr) = ctx.get("recent_image_paths").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn memory_snippet_for_resolver(obj: &serde_json::Map<String, Value>) -> String {
    obj.get("_memory")
        .and_then(|m| m.get("context"))
        .and_then(|v| v.as_str())
        .unwrap_or("<none>")
        .to_string()
}

fn load_image_reference_resolver_prompt(workspace_root: &Path, vendor: &str) -> String {
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        vendor,
        "prompts/image_reference_resolver_prompt.md",
        include_str!("../../../../prompts/layers/overlays/image_reference_resolver_prompt.md"),
    )
    .0
}

fn render_image_reference_prompt(
    template: &str,
    memory: &str,
    goal: &str,
    candidates: &[String],
) -> String {
    let lines = candidates
        .iter()
        .enumerate()
        .map(|(i, p)| format!("{i}: {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    template
        .replace("__MEMORY_TEXT__", memory)
        .replace("__GOAL__", goal)
        .replace("__CANDIDATES__", &lines)
}

fn image_reference_resolver_schema() -> &'static Value {
    IMAGE_REFERENCE_RESOLVER_SCHEMA.get_or_init(|| {
        serde_json::from_str::<Value>(IMAGE_REFERENCE_RESOLVER_SCHEMA_RAW)
            .expect("image_reference_resolver schema must be valid JSON")
    })
}

fn schema_requires_field(schema: &Value, name: &str) -> bool {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|fields| fields.iter().any(|field| field.as_str() == Some(name)))
        .unwrap_or(false)
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

fn schema_integer_in_range(schema: &Value, name: &str, value: i64) -> bool {
    let property = match schema.get("properties").and_then(|v| v.get(name)) {
        Some(property) => property,
        None => return false,
    };
    if property.get("type").and_then(|v| v.as_str()) != Some("integer") {
        return false;
    }
    let minimum = property
        .get("minimum")
        .and_then(|v| v.as_i64())
        .unwrap_or(i64::MIN);
    let maximum = property
        .get("maximum")
        .and_then(|v| v.as_i64())
        .unwrap_or(i64::MAX);
    value >= minimum && value <= maximum
}

fn parse_selected_index_from_json_value(value: &Value) -> Option<i64> {
    let schema = image_reference_resolver_schema();
    let object = value.as_object()?;
    if !schema_allows_additional_properties(schema) {
        let declared_fields = schema_declared_fields(schema)?;
        if object.keys().any(|key| !declared_fields.contains_key(key)) {
            return None;
        }
    }
    if schema_requires_field(schema, "selected_index") && !object.contains_key("selected_index") {
        return None;
    }
    let selected_index = object.get("selected_index")?.as_i64()?;
    schema_integer_in_range(schema, "selected_index", selected_index).then_some(selected_index)
}

fn parse_llm_selected_index(raw: &str) -> Option<i64> {
    let t = raw.trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) {
        return parse_selected_index_from_json_value(&v);
    }
    let start = t.find('{')?;
    let end = t.rfind('}')?;
    if end > start {
        let slice = t.get(start..=end)?;
        let v: Value = serde_json::from_str(slice).ok()?;
        return parse_selected_index_from_json_value(&v);
    }
    None
}

fn openai_compat_chat_completion(
    vcfg: &VendorConfig,
    model: &str,
    user_prompt: &str,
    timeout_secs: u64,
) -> Result<String, String> {
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_secs.clamp(5, 120)))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let url = format!(
        "{}/v1/chat/completions",
        trim_trailing_slash(&vcfg.base_url)
    );
    let body = json!({
        "model": model,
        "messages": [{"role": "user", "content": user_prompt}],
        "temperature": 0.0
    });
    let resp = client
        .post(&url)
        .bearer_auth(&vcfg.api_key.trim())
        .json(&body)
        .send()
        .map_err(|e| format!("chat completion request failed: {e}"))?;
    if !resp.status().is_success() {
        let t = resp.text().unwrap_or_default();
        return Err(format!("chat completion HTTP error: {t}"));
    }
    let v: Value = resp
        .json()
        .map_err(|e| format!("parse chat response: {e}"))?;
    Ok(v.get("choices")
        .and_then(|c| c.as_array())
        .and_then(|a| a.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string())
}

fn resolve_image_path_from_context(
    cfg: &RootConfig,
    workspace_root: &Path,
    instruction: &str,
    obj: &serde_json::Map<String, Value>,
    ctx: Option<&Value>,
) -> Result<String, String> {
    let candidates = recent_image_paths_from_context(ctx);
    if candidates.is_empty() {
        return Err(
            "Could not determine which image to edit: no recent image paths in context. \
             Upload an image, set image.path/url, or name the file explicitly."
                .to_string(),
        );
    }
    if candidates.len() == 1 {
        return Ok(candidates[0].clone());
    }
    let memory = memory_snippet_for_resolver(obj);
    let prompt_vendor = cfg
        .llm
        .selected_vendor
        .as_deref()
        .or(cfg.image_edit.default_vendor.as_deref())
        .unwrap_or("default");
    let template = load_image_reference_resolver_prompt(workspace_root, prompt_vendor);
    let prompt = render_image_reference_prompt(&template, &memory, instruction, &candidates);
    let timeout = 30u64;
    let mut last_err: Option<String> = None;
    for vk in vendor_order(
        None,
        cfg.image_edit.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    ) {
        let Ok((vname, vcfg)) = resolve_vendor_config(cfg, vk) else {
            continue;
        };
        if check_api_key(vname, &vcfg.api_key).is_err() {
            continue;
        }
        let model = vcfg.model.trim();
        if model.is_empty() {
            continue;
        }
        match openai_compat_chat_completion(vcfg, model, &prompt, timeout) {
            Ok(out) => {
                if let Some(idx) = parse_llm_selected_index(&out) {
                    if idx >= 0 {
                        let u = idx as usize;
                        if let Some(p) = candidates.get(u) {
                            return Ok(p.clone());
                        }
                    }
                }
                last_err = Some(format!(
                    "resolver model returned no usable selected_index (output truncated): {}",
                    truncate(out.trim(), 200)
                ));
            }
            Err(e) => last_err = Some(e),
        }
    }
    let preview = candidates
        .iter()
        .take(8)
        .enumerate()
        .map(|(i, p)| format!("  [{i}] {p}"))
        .collect::<Vec<_>>()
        .join("\n");
    let hint = last_err.unwrap_or_else(|| "LLM resolver unavailable".to_string());
    Err(format!(
        "Multiple recent images ({}); could not pick one automatically ({hint}). \
         Set image.path to the file you want, or reply with an index 0..{}.\n{}",
        candidates.len(),
        candidates.len().saturating_sub(1),
        preview
    ))
}

fn execute(
    cfg: &RootConfig,
    workspace_root: &Path,
    args: Value,
    context: Option<&Value>,
) -> Result<(String, Value), String> {
    let mut obj = args
        .as_object()
        .cloned()
        .ok_or_else(|| "args must be object".to_string())?;

    let action_empty = obj
        .get("action")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().is_empty())
        .unwrap_or(true);
    if !obj.contains_key("action") || action_empty {
        obj.insert("action".to_string(), Value::String("edit".to_string()));
    }

    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("edit")
        .trim()
        .to_ascii_lowercase();
    if !matches!(
        action.as_str(),
        "edit" | "outpaint" | "restyle" | "add_remove"
    ) {
        return Err("unsupported action; use edit|outpaint|restyle|add_remove".to_string());
    }
    let instruction = obj
        .get("instruction")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "instruction is required".to_string())?
        .to_string();

    if !image_edit_args_has_image(&obj) {
        let path =
            resolve_image_path_from_context(cfg, workspace_root, &instruction, &obj, context)?;
        obj.insert("image".to_string(), json!({ "path": path }));
    } else if obj.get("image").is_none() {
        if let Some(image) = first_image_from_images_array(&obj) {
            obj.insert("image".to_string(), image);
        }
    }

    let image_source = obj
        .get("image")
        .ok_or_else(|| "image is required".to_string())
        .and_then(|v| parse_image(v, workspace_root))?;
    let mask = obj
        .get("mask")
        .map(|v| parse_image(v, workspace_root))
        .transpose()?;

    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.image_edit.timeout_seconds.unwrap_or(120))
        .clamp(5, 300);
    let max_input_bytes = cfg.image_edit.max_input_bytes.unwrap_or(10 * 1024 * 1024);

    let requested_vendor = obj.get("vendor").and_then(|v| v.as_str());
    let requested_model = obj.get("model").and_then(|v| v.as_str());
    let providers = vendor_order(
        requested_vendor,
        cfg.image_edit.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    if providers.is_empty() {
        return Err("no vendor configured".to_string());
    }

    let output_path = resolve_output_path(
        workspace_root,
        cfg.image_edit
            .default_output_dir
            .as_deref()
            .unwrap_or("image"),
        obj.get("output_path").and_then(|v| v.as_str()),
    )?;
    let output_lang = resolve_output_language(cfg, &obj);
    let i18n = TextCatalog::for_lang(workspace_root, &cfg.image_edit, &output_lang);

    let effective_instruction = rewrite_instruction(&action, instruction.as_str());
    let size = obj
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("1024x1024");
    let quality = obj.get("quality").and_then(|v| v.as_str());
    let n = obj
        .get("n")
        .and_then(|v| v.as_u64())
        .unwrap_or(1)
        .clamp(1, 2);

    let mut provider_errors: Vec<String> = Vec::new();
    for vendor in providers {
        let config_default_model = first_model_candidate(
            cfg.image_edit.default_model.as_deref(),
            vendor_models(&cfg.image_edit, vendor),
            cfg.image_edit.models.as_ref(),
        );
        match call_edit(
            vendor,
            cfg,
            requested_model.or(config_default_model),
            timeout_seconds,
            &effective_instruction,
            &image_source,
            mask.as_ref(),
            size,
            quality,
            n,
            max_input_bytes,
            &output_path,
        ) {
            Ok((model, model_kind)) => {
                return Ok(build_success_response(
                    &i18n,
                    &output_path,
                    vendor_name(vendor),
                    &model,
                    model_kind,
                    &action,
                ));
            }
            Err(err) => provider_errors.push(err),
        }
    }
    Err(format!(
        "all providers failed: {}",
        provider_errors
            .last()
            .cloned()
            .unwrap_or_else(|| "unknown error".to_string())
    ))
}

fn build_success_response(
    i18n: &TextCatalog,
    output_path: &Path,
    provider: &str,
    model: &str,
    model_kind: &str,
    action: &str,
) -> (String, Value) {
    let saved_path = output_path.to_string_lossy().to_string();
    let preface = i18n.render(
        "image_edit.msg.saved",
        &[("path", saved_path.clone())],
        "Edited successfully and saved: {path}",
    );
    let text = format!("{preface}\nFILE:{saved_path}\nEPHEMERAL:IMAGE_SAVED");
    let extra = json!({
        "message_key": "image_edit.msg.saved",
        "provider": provider,
        "model": model,
        "model_kind": model_kind,
        "latency_ms": 0,
        "action": action,
        "media_type": "image",
        "output_path": saved_path.clone(),
        "outputs": [{"type":"image_file","path": saved_path}]
    });
    (text, extra)
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

fn load_image_bytes(
    client: &Client,
    source: &ImageSource,
    max_input_bytes: usize,
) -> Result<(Vec<u8>, String), String> {
    match source {
        ImageSource::Path(p) => {
            let bytes = std::fs::read(p).map_err(|err| format!("read image failed: {err}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            Ok((bytes, guess_mime_from_path(p).to_string()))
        }
        ImageSource::Url(url) => {
            let resp = client
                .get(url)
                .send()
                .map_err(|err| format!("download image failed: {err}"))?;
            let mime = resp
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .unwrap_or("image/png")
                .to_string();
            let bytes = resp
                .bytes()
                .map_err(|err| format!("read image bytes failed: {err}"))?
                .to_vec();
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            Ok((bytes, mime))
        }
        ImageSource::Base64(raw) => {
            let (mime, data) = split_image_data(raw);
            let bytes = STANDARD
                .decode(data)
                .map_err(|err| format!("decode base64 image failed: {err}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            Ok((bytes, mime))
        }
    }
}

fn image_source_to_wan26_input(
    client: &Client,
    source: &ImageSource,
    max_input_bytes: usize,
    fallback_name: &str,
) -> Result<String, String> {
    match source {
        ImageSource::Url(url) => Ok(url.trim().to_string()),
        ImageSource::Base64(raw) => {
            let (mime, data) = split_image_data(raw);
            let bytes = STANDARD
                .decode(&data)
                .map_err(|err| format!("decode base64 image failed: {err}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            Ok(format!("data:{mime};base64,{data}"))
        }
        ImageSource::Path(path) => {
            let (bytes, mime) = load_image_bytes(client, source, max_input_bytes)?;
            let data = STANDARD.encode(bytes);
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .unwrap_or(fallback_name);
            let _ = file_name;
            Ok(format!("data:{mime};base64,{data}"))
        }
    }
}

fn resolve_qwen_native_image_url(
    client: &Client,
    cfg: &ImageSkillConfig,
    source: &ImageSource,
    max_input_bytes: usize,
    fallback_name: &str,
    field_name: &str,
) -> Result<String, String> {
    match source {
        ImageSource::Url(url) => Ok(url.trim().to_string()),
        ImageSource::Path(_) | ImageSource::Base64(_) => {
            if !cfg.local_auto_upload_enabled {
                return Err(format!(
                    "qwen native image edit requires args.{field_name}.url (public URL), or enable image_edit.local_auto_upload_enabled with OSS settings"
                ));
            }
            upload_image_to_oss_and_sign_url(client, cfg, source, max_input_bytes, fallback_name)
        }
    }
}

fn upload_image_to_oss_and_sign_url(
    client: &Client,
    cfg: &ImageSkillConfig,
    source: &ImageSource,
    max_input_bytes: usize,
    fallback_name: &str,
) -> Result<String, String> {
    let access_key_id = cfg
        .oss_access_key_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "image_edit.oss_access_key_id is required".to_string())?;
    let access_key_secret = cfg
        .oss_access_key_secret
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "image_edit.oss_access_key_secret is required".to_string())?;
    let bucket = cfg
        .oss_bucket
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "image_edit.oss_bucket is required".to_string())?;
    let endpoint = cfg
        .oss_endpoint
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("oss-cn-beijing.aliyuncs.com");
    let prefix = cfg
        .oss_object_prefix
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("rustclaw/image");
    let ttl_seconds = cfg.oss_url_ttl_seconds.unwrap_or(3600).clamp(60, 24 * 3600);

    let (bytes, content_type, file_name) =
        load_image_for_oss_upload(source, max_input_bytes, fallback_name)?;
    let ts = unix_ts();
    let object_key = format!("{}/{}-{}", prefix.trim_matches('/'), ts, file_name);
    let put_url = format!(
        "https://{}.{}{}",
        bucket,
        endpoint,
        object_path(&object_key)
    );
    let date = httpdate::fmt_http_date(SystemTime::now());
    let canonical_resource = format!("/{}/{}", bucket, object_key);
    let string_to_sign = format!("PUT\n\n{}\n{}\n{}", content_type, date, canonical_resource);
    let put_signature = hmac_sha1_base64(access_key_secret, &string_to_sign)?;
    let authorization = format!("OSS {}:{}", access_key_id, put_signature);
    let put_resp = client
        .put(&put_url)
        .header("Date", date)
        .header("Content-Type", &content_type)
        .header("Authorization", authorization)
        .body(bytes)
        .send()
        .map_err(|err| format!("upload image to OSS failed: {err}"))?;
    let status = put_resp.status().as_u16();
    let body = put_resp.text().unwrap_or_default();
    if status >= 300 {
        return Err(format!(
            "upload image to OSS failed status={status}: {}",
            truncate(&body, 400)
        ));
    }

    let expires = unix_ts() + ttl_seconds;
    let get_string_to_sign = format!("GET\n\n\n{}\n{}", expires, canonical_resource);
    let get_signature = hmac_sha1_base64(access_key_secret, &get_string_to_sign)?;
    Ok(format!(
        "{}?OSSAccessKeyId={}&Expires={}&Signature={}",
        put_url,
        urlencoding::encode(access_key_id),
        expires,
        urlencoding::encode(&get_signature)
    ))
}

fn load_image_for_oss_upload(
    source: &ImageSource,
    max_input_bytes: usize,
    fallback_name: &str,
) -> Result<(Vec<u8>, String, String), String> {
    match source {
        ImageSource::Path(path) => {
            if !path.exists() || !path.is_file() {
                return Err("image file does not exist".to_string());
            }
            let bytes = std::fs::read(path).map_err(|err| format!("read image failed: {err}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            let mime = guess_mime_from_path(path).to_string();
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .map(sanitize_oss_filename)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| sanitize_oss_filename(fallback_name));
            Ok((bytes, mime, file_name))
        }
        ImageSource::Base64(raw) => {
            let (mime, data) = split_image_data(raw);
            let bytes = STANDARD
                .decode(data)
                .map_err(|err| format!("decode base64 image failed: {err}"))?;
            if bytes.len() > max_input_bytes {
                return Err(format!("image too large: {} bytes", bytes.len()));
            }
            let fallback = image_filename_for_mime(fallback_name, &mime);
            Ok((bytes, mime, sanitize_oss_filename(&fallback)))
        }
        ImageSource::Url(_) => Err("image source already has URL".to_string()),
    }
}

fn parse_image(v: &Value, workspace_root: &Path) -> Result<ImageSource, String> {
    if let Some(s) = v.as_str() {
        return parse_image_str(s, workspace_root);
    }
    let obj = v
        .as_object()
        .ok_or_else(|| "image must be string or object".to_string())?;
    if let Some(s) = obj.get("path").and_then(|v| v.as_str()) {
        return Ok(ImageSource::Path(to_workspace_path(workspace_root, s)?));
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

fn parse_image_str(s: &str, workspace_root: &Path) -> Result<ImageSource, String> {
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

fn rewrite_instruction(action: &str, instruction: &str) -> String {
    match action {
        "outpaint" => format!("Outpaint this image. Extend canvas naturally. {instruction}"),
        "restyle" => format!("Keep composition, restyle visual style only. {instruction}"),
        "add_remove" => {
            format!("Add/remove elements as requested while preserving realism. {instruction}")
        }
        _ => instruction.to_string(),
    }
}

fn resolve_output_path(
    workspace_root: &Path,
    default_dir: &str,
    requested: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(path) = requested {
        let p = Path::new(path);
        let out = if p.is_absolute() {
            p.to_path_buf()
        } else {
            workspace_root.join(p)
        };
        if !out.starts_with(workspace_root) {
            return Err("output_path is outside workspace".to_string());
        }
        return Ok(out);
    }
    let file_name = format!("edit-{}.png", unix_ts());
    Ok(workspace_root.join(default_dir).join(file_name))
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "output path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|err| format!("create output dir failed: {err}"))
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
    if let Some(v) = image_cfg.get("image_edit").cloned() {
        if let Ok(parsed) = v.try_into::<ImageSkillConfig>() {
            cfg.image_edit = parsed;
        }
    }
    if let Some(v) = core_cfg.get("command_intent").cloned() {
        if let Ok(parsed) = v.try_into::<CommandIntentConfig>() {
            cfg.command_intent = parsed;
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

fn inherit_provider_api_key_from_llm(
    target: &mut Option<VendorConfig>,
    source: &Option<VendorConfig>,
) {
    let Some(target) = target.as_mut() else {
        return;
    };
    if !target.api_key.trim().is_empty() {
        return;
    }
    if let Some(value) = source
        .as_ref()
        .map(|cfg| cfg.api_key.trim())
        .filter(|value| !value.is_empty())
    {
        target.api_key = value.to_string();
    }
}

fn apply_option_string_env(target: &mut Option<String>, key: &str) {
    if let Some(value) = env_non_empty(key) {
        *target = Some(value);
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

    apply_vendor_api_key_env(
        &mut cfg.image_edit.providers.openai,
        "IMAGE_EDIT_OPENAI_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_edit.providers.google,
        "IMAGE_EDIT_GOOGLE_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_edit.providers.anthropic,
        "IMAGE_EDIT_ANTHROPIC_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_edit.providers.grok,
        "IMAGE_EDIT_GROK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_edit.providers.deepseek,
        "IMAGE_EDIT_DEEPSEEK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_edit.providers.qwen,
        "IMAGE_EDIT_QWEN_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.image_edit.providers.minimax,
        "IMAGE_EDIT_MINIMAX_API_KEY",
    );
    inherit_provider_api_key_from_llm(&mut cfg.image_edit.providers.openai, &cfg.llm.openai);
    inherit_provider_api_key_from_llm(&mut cfg.image_edit.providers.google, &cfg.llm.google);
    inherit_provider_api_key_from_llm(&mut cfg.image_edit.providers.anthropic, &cfg.llm.anthropic);
    inherit_provider_api_key_from_llm(&mut cfg.image_edit.providers.grok, &cfg.llm.grok);
    inherit_provider_api_key_from_llm(&mut cfg.image_edit.providers.deepseek, &cfg.llm.deepseek);
    inherit_provider_api_key_from_llm(&mut cfg.image_edit.providers.qwen, &cfg.llm.qwen);
    inherit_provider_api_key_from_llm(&mut cfg.image_edit.providers.minimax, &cfg.llm.minimax);
    apply_option_string_env(
        &mut cfg.image_edit.oss_access_key_id,
        "IMAGE_EDIT_OSS_ACCESS_KEY_ID",
    );
    apply_option_string_env(
        &mut cfg.image_edit.oss_access_key_secret,
        "IMAGE_EDIT_OSS_ACCESS_KEY_SECRET",
    );
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
    let section = &cfg.image_edit.providers;
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

fn image_filename_for_mime(fallback_name: &str, mime: &str) -> String {
    let sanitized = sanitize_oss_filename(fallback_name);
    if Path::new(&sanitized).extension().is_some() {
        return sanitized;
    }
    format!("{sanitized}.{}", image_extension_from_mime(mime))
}

fn is_qwen_multimodal_edit_model(model: &str) -> bool {
    let model = model.trim();
    model.eq_ignore_ascii_case("wan2.6-image") || model.eq_ignore_ascii_case("qwen-image-edit-max")
}

fn normalize_wan26_size(size: &str) -> String {
    let trimmed = size.trim();
    if trimmed.eq_ignore_ascii_case("1k") || trimmed.eq_ignore_ascii_case("2k") {
        return trimmed.to_ascii_uppercase();
    }
    trimmed.replace('x', "*").replace('X', "*")
}

fn extract_qwen_output_image_url<'a>(v: &'a Value) -> Option<&'a str> {
    v.get("output")
        .and_then(|o| o.get("results"))
        .and_then(|items| items.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("url"))
        .and_then(|url| url.as_str())
        .or_else(|| {
            v.get("output")
                .and_then(|o| o.get("choices"))
                .and_then(|choices| choices.as_array())
                .and_then(|choices| choices.first())
                .and_then(|choice| choice.get("message"))
                .and_then(|msg| msg.get("content"))
                .and_then(|content| content.as_array())
                .and_then(|content| {
                    content.iter().find_map(|item| {
                        item.get("image")
                            .or_else(|| item.get("url"))
                            .and_then(|url| url.as_str())
                    })
                })
        })
}

fn image_extension_from_mime(mime: &str) -> &'static str {
    match mime.trim().to_ascii_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        _ => "png",
    }
}

fn trim_trailing_slash(v: &str) -> String {
    v.trim_end_matches('/').to_string()
}

fn size_to_minimax_aspect_ratio(size: &str) -> String {
    let normalized = size.trim().replace('X', "x");
    let parts = normalized.split('x').collect::<Vec<_>>();
    if parts.len() == 2 {
        if let (Ok(w), Ok(h)) = (
            parts[0].trim().parse::<u64>(),
            parts[1].trim().parse::<u64>(),
        ) {
            if w > 0 && h > 0 {
                let g = gcd_u64(w, h);
                return format!("{}:{}", w / g, h / g);
            }
        }
    }
    "1:1".to_string()
}

fn gcd_u64(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a.max(1)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect::<String>() + "..."
}

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn hmac_sha1_base64(secret: &str, message: &str) -> Result<String, String> {
    type HmacSha1 = Hmac<Sha1>;
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes())
        .map_err(|err| format!("invalid HMAC key: {err}"))?;
    mac.update(message.as_bytes());
    Ok(STANDARD.encode(mac.finalize().into_bytes()))
}

fn object_path(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 1);
    out.push('/');
    out.push_str(key.trim_matches('/'));
    out
}

fn sanitize_oss_filename(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "image.png".to_string()
    } else {
        out
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
