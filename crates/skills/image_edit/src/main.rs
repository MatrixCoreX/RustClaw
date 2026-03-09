use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use reqwest::blocking::{multipart, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Sha1;

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
    qwen: Option<VendorConfig>,
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
    qwen_models: Option<Vec<String>>,
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
}

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

impl TextCatalog {
    fn for_lang(workspace_root: &Path, cfg: &ImageSkillConfig, lang: &str) -> Self {
        let mut current = default_i18n_dict(lang);
        let lang_tag = normalize_lang_tag(lang);
        let default_path = workspace_root.join(format!("configs/i18n/image_edit.{lang_tag}.toml"));
        if let Some(external) = load_external_i18n(&default_path) {
            current.extend(external);
        }
        if let Some(custom) = cfg.i18n_path.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
            let custom_path = if Path::new(custom).is_absolute() {
                PathBuf::from(custom)
            } else {
                workspace_root.join(custom)
            };
            if let Some(external) = load_external_i18n(&custom_path) {
                current.extend(external);
            }
        }
        Self { current }
    }

    fn render(&self, key: &str, vars: &[(&str, String)], default: &str) -> String {
        let mut out = self
            .current
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string());
        for (k, v) in vars {
            out = out.replace(&format!("{{{k}}}"), v);
        }
        out
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum VendorKind {
    OpenAI,
    Google,
    Anthropic,
    Grok,
    Qwen,
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

fn execute(cfg: &RootConfig, workspace_root: &Path, args: Value) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("edit")
        .trim()
        .to_ascii_lowercase();
    if !matches!(action.as_str(), "edit" | "outpaint" | "restyle" | "add_remove") {
        return Err("unsupported action; use edit|outpaint|restyle|add_remove".to_string());
    }
    let instruction = obj
        .get("instruction")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "instruction is required".to_string())?;

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
        cfg.image_edit.default_output_dir.as_deref().unwrap_or("image"),
        obj.get("output_path").and_then(|v| v.as_str()),
    )?;
    let output_lang = resolve_output_language(cfg, obj);
    let i18n = TextCatalog::for_lang(workspace_root, &cfg.image_edit, &output_lang);

    let effective_instruction = rewrite_instruction(&action, instruction);
    let size = obj
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("1024x1024");
    let quality = obj.get("quality").and_then(|v| v.as_str());
    let n = obj.get("n").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 2);

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
            Ok(model) => {
                let saved_path = output_path.to_string_lossy().to_string();
                let preface = i18n.render(
                    "image_edit.msg.saved",
                    &[("path", saved_path.clone())],
                    "Edited successfully and saved: {path}",
                );
                let text = format!("{preface}\nFILE:{saved_path}\nEPHEMERAL:IMAGE_SAVED");
                let extra = json!({
                    "provider": vendor_name(vendor),
                    "model": model,
                    "latency_ms": 0,
                    "action": action,
                    "outputs": [{"type":"image_file","path": saved_path}]
                });
                return Ok((text, extra));
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

fn first_model_candidate<'a>(
    default_model: Option<&'a str>,
    vendor_models: Option<&'a Vec<String>>,
    models: Option<&'a Vec<String>>,
) -> Option<&'a str> {
    if let Some(v) = default_model.map(str::trim).filter(|v| !v.is_empty()) {
        return Some(v);
    }
    if let Some(v) = vendor_models.and_then(|list| list.iter().map(|s| s.trim()).find(|v| !v.is_empty())) {
        return Some(v);
    }
    models
        .and_then(|list| list.iter().map(|s| s.trim()).find(|v| !v.is_empty()))
}

fn vendor_models<'a>(cfg: &'a ImageSkillConfig, vendor: VendorKind) -> Option<&'a Vec<String>> {
    match vendor {
        VendorKind::OpenAI => cfg.openai_models.as_ref(),
        VendorKind::Google => cfg.google_models.as_ref(),
        VendorKind::Anthropic => cfg.anthropic_models.as_ref(),
        VendorKind::Grok => cfg.grok_models.as_ref(),
        VendorKind::Qwen => cfg.qwen_models.as_ref(),
    }
}

fn resolve_output_language(cfg: &RootConfig, obj: &serde_json::Map<String, Value>) -> String {
    obj.get("response_language")
        .or_else(|| obj.get("language"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(normalize_lang_tag)
        .or_else(|| {
            obj.get("_memory")
                .and_then(|m| m.get("lang_hint"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(normalize_lang_tag)
        })
        .or_else(|| {
            cfg.image_edit
                .language
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(normalize_lang_tag)
        })
        .or_else(|| {
            cfg.command_intent
                .default_locale
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(normalize_lang_tag)
        })
        .unwrap_or_else(|| "en-US".to_string())
}

fn normalize_lang_tag(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    if lowered.starts_with("zh") || lowered.contains("cn") || lowered.contains("hans") {
        "zh-CN".to_string()
    } else {
        "en-US".to_string()
    }
}

fn default_i18n_dict(lang: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if normalize_lang_tag(lang) == "zh-CN" {
        out.insert(
            "image_edit.msg.saved".to_string(),
            "图片编辑成功并已保存：{path}".to_string(),
        );
    } else {
        out.insert(
            "image_edit.msg.saved".to_string(),
            "Edited successfully and saved: {path}".to_string(),
        );
    }
    out
}

fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value = toml::from_str::<toml::Value>(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        if let Some(s) = v.as_str() {
            out.insert(k.to_string(), s.to_string());
        }
    }
    Some(out)
}

#[allow(clippy::too_many_arguments)]
fn call_edit(
    vendor: VendorKind,
    cfg: &RootConfig,
    requested_model: Option<&str>,
    timeout_seconds: u64,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    quality: Option<&str>,
    n: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<String, String> {
    let mode = resolve_adapter_mode(&cfg.image_edit);
    match vendor {
        VendorKind::OpenAI => {
            let vcfg = cfg
                .llm
                .openai
                .as_ref()
                .ok_or_else(|| "openai config missing".to_string())?;
            check_api_key("openai", &vcfg.api_key)?;
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build openai client failed: {err}"))?;
            openai_compatible_edit(
                &client,
                "openai",
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok(model)
        }
        VendorKind::Google => {
            let vcfg = cfg
                .llm
                .google
                .as_ref()
                .ok_or_else(|| "google config missing".to_string())?;
            check_api_key("google", &vcfg.api_key)?;
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build google client failed: {err}"))?;
            google_edit(
                &client,
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok(model)
        }
        VendorKind::Anthropic => {
            if mode == AdapterMode::Native {
                return Err("anthropic native image edit adapter is not available".to_string());
            }
            if !cfg.image_edit.allow_compat_adapters && mode != AdapterMode::Compat {
                return Err(
                    "anthropic native image edit adapter is not available; set image_edit.allow_compat_adapters=true to use compatible endpoint"
                        .to_string(),
                );
            }
            let vcfg = cfg
                .llm
                .anthropic
                .as_ref()
                .ok_or_else(|| "anthropic config missing".to_string())?;
            check_api_key("anthropic", &vcfg.api_key)?;
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build anthropic client failed: {err}"))?;
            openai_compatible_edit(
                &client,
                "anthropic",
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok(model)
        }
        VendorKind::Grok => {
            if mode == AdapterMode::Native {
                return Err("grok native image edit adapter is not available".to_string());
            }
            if !cfg.image_edit.allow_compat_adapters && mode != AdapterMode::Compat {
                return Err(
                    "grok native image edit adapter is not available; set image_edit.allow_compat_adapters=true to use compatible endpoint"
                        .to_string(),
                );
            }
            let vcfg = cfg
                .llm
                .grok
                .as_ref()
                .ok_or_else(|| "grok config missing".to_string())?;
            check_api_key("grok", &vcfg.api_key)?;
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build grok client failed: {err}"))?;
            openai_compatible_edit(
                &client,
                "grok",
                vcfg,
                &model,
                instruction,
                image,
                mask,
                size,
                quality,
                n,
                max_input_bytes,
                output_path,
            )?;
            Ok(model)
        }
        VendorKind::Qwen => {
            let vcfg = cfg
                .llm
                .qwen
                .as_ref()
                .ok_or_else(|| "qwen config missing".to_string())?;
            check_api_key("qwen", &vcfg.api_key)?;
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build qwen client failed: {err}"))?;
            let can_use_native_inputs =
                qwen_native_edit_inputs_supported(&cfg.image_edit, &model, image, mask);
            if should_use_qwen_native_edit(
                &cfg.image_edit,
                &model,
                mode,
                cfg.image_edit.allow_compat_adapters,
                can_use_native_inputs,
            ) {
                qwen_native_edit(
                    &client,
                    &cfg.image_edit,
                    cfg.image_edit.qwen_native_base_url.as_deref(),
                    cfg.image_edit.qwen_native_function.as_deref(),
                    &vcfg.api_key,
                    &model,
                    instruction,
                    image,
                    mask,
                    size,
                    n,
                    timeout_seconds,
                    max_input_bytes,
                    output_path,
                )?;
            } else {
                if !cfg.image_edit.allow_compat_adapters {
                    return Err(
                        "qwen native image edit adapter is not available; set image_edit.allow_compat_adapters=true to use compatible endpoint"
                            .to_string(),
                    );
                }
                openai_compatible_edit(
                    &client,
                    "qwen",
                    vcfg,
                    &model,
                    instruction,
                    image,
                    mask,
                    size,
                    quality,
                    n,
                    max_input_bytes,
                    output_path,
                )?;
            }
            Ok(model)
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

fn qwen_uses_native_edit_api(cfg: &ImageSkillConfig, model: &str) -> bool {
    let requested = model.trim();
    cfg.native_models
        .as_ref()
        .and_then(|list| {
            list.iter()
                .map(|s| s.trim())
                .find(|candidate| !candidate.is_empty() && candidate.eq_ignore_ascii_case(requested))
        })
        .is_some()
}

fn qwen_native_edit_inputs_supported(
    cfg: &ImageSkillConfig,
    model: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
) -> bool {
    if is_qwen_multimodal_edit_model(model) {
        return true;
    }
    qwen_native_edit_source_supported(cfg, image)
        && mask
            .map(|source| qwen_native_edit_source_supported(cfg, source))
            .unwrap_or(true)
}

fn qwen_native_edit_source_supported(cfg: &ImageSkillConfig, source: &ImageSource) -> bool {
    matches!(source, ImageSource::Url(_))
        || (cfg.local_auto_upload_enabled && matches!(source, ImageSource::Path(_) | ImageSource::Base64(_)))
}

fn should_use_qwen_native_edit(
    cfg: &ImageSkillConfig,
    model: &str,
    mode: AdapterMode,
    allow_compat: bool,
    can_use_native_inputs: bool,
) -> bool {
    match mode {
        AdapterMode::Native => can_use_native_inputs,
        AdapterMode::Compat => false,
        AdapterMode::Auto => {
            if qwen_uses_native_edit_api(cfg, model) && can_use_native_inputs {
                true
            } else {
                !allow_compat
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn qwen_native_edit(
    client: &Client,
    image_cfg: &ImageSkillConfig,
    native_base_url: Option<&str>,
    native_function: Option<&str>,
    api_key: &str,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    n: u64,
    timeout_seconds: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    if is_qwen_multimodal_edit_model(model) {
        return qwen_wan26_edit(
            client,
            api_key,
            model,
            instruction,
            image,
            mask,
            size,
            n,
            timeout_seconds,
            max_input_bytes,
            output_path,
        );
    }
    let base = native_base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("https://dashscope.aliyuncs.com/api/v1");
    let url = format!(
        "{}/services/aigc/image2image/image-synthesis",
        trim_trailing_slash(base)
    );
    let base_image_url =
        resolve_qwen_native_image_url(client, image_cfg, image, max_input_bytes, "image.png", "image")?;
    let normalized_size = size.trim().replace('x', "*").replace('X', "*");
    let function = native_function
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("description_edit");
    let mut input = json!({
        "prompt": instruction,
        "function": function,
        "base_image_url": base_image_url
    });
    if let Some(mask_source) = mask {
        let mask_url = resolve_qwen_native_image_url(
            client,
            image_cfg,
            mask_source,
            max_input_bytes,
            "mask.png",
            "mask",
        )?;
        input["mask_image_url"] = Value::String(mask_url);
    }
    let body = json!({
        "model": model,
        "input": input,
        "parameters": {
            "size": normalized_size,
            "n": n,
            "watermark": false
        }
    });

    let create_resp = client
        .post(url)
        .bearer_auth(api_key)
        .header("X-DashScope-Async", "enable")
        .json(&body)
        .send()
        .map_err(|err| format!("qwen native edit request failed: {err}"))?;
    let create_status = create_resp.status().as_u16();
    let create_v: Value = create_resp
        .json()
        .map_err(|err| format!("parse qwen native edit create response failed: {err}"))?;
    if create_status >= 300 {
        return Err(format!(
            "qwen native edit create error status={create_status}: {}",
            truncate(&create_v.to_string(), 400)
        ));
    }

    let task_id = create_v
        .get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "qwen native edit response missing task_id: {}",
                truncate(&create_v.to_string(), 400)
            )
        })?;

    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(10));
    let task_url = format!("{}/tasks/{task_id}", trim_trailing_slash(base));
    loop {
        if Instant::now() > deadline {
            return Err(format!("qwen native edit task timeout: task_id={task_id}"));
        }
        let task_resp = client
            .get(&task_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("qwen native edit poll failed: {err}"))?;
        let task_status = task_resp.status().as_u16();
        let task_v: Value = task_resp
            .json()
            .map_err(|err| format!("parse qwen native edit task response failed: {err}"))?;
        if task_status >= 300 {
            return Err(format!(
                "qwen native edit poll error status={task_status}: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        let status = task_v
            .get("output")
            .and_then(|o| o.get("task_status"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if status == "SUCCEEDED" {
            let url = task_v
                .get("output")
                .and_then(|o| o.get("results"))
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|item| item.get("url"))
                .and_then(|v| v.as_str())
                .ok_or_else(|| {
                    format!(
                        "qwen native edit success response missing image url: {}",
                        truncate(&task_v.to_string(), 400)
                    )
                })?;
            let bytes = client
                .get(url)
                .send()
                .map_err(|err| format!("download edited image failed: {err}"))?
                .bytes()
                .map_err(|err| format!("read edited image bytes failed: {err}"))?;
            ensure_parent_dir(output_path)?;
            std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
            return Ok(());
        }
        if status == "FAILED" || status == "CANCELED" || status == "CANCELLED" {
            return Err(format!(
                "qwen native edit task failed: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        thread::sleep(Duration::from_millis(1200));
    }
}

#[allow(clippy::too_many_arguments)]
fn qwen_wan26_edit(
    client: &Client,
    api_key: &str,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    n: u64,
    timeout_seconds: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    let url = "https://dashscope.aliyuncs.com/api/v1/services/aigc/image-generation/generation";
    let mut content = vec![json!({ "text": instruction })];
    content.push(json!({
        "image": image_source_to_wan26_input(client, image, max_input_bytes, "image.png")?
    }));
    if let Some(mask_source) = mask {
        content.push(json!({
            "image": image_source_to_wan26_input(client, mask_source, max_input_bytes, "mask.png")?
        }));
    }
    let body = json!({
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": content
            }]
        },
        "parameters": {
            "size": normalize_wan26_size(size),
            "n": n,
            "prompt_extend": true,
            "watermark": false,
            "enable_interleave": false
        }
    });
    let create_resp = client
        .post(url)
        .bearer_auth(api_key)
        .header("X-DashScope-Async", "enable")
        .json(&body)
        .send()
        .map_err(|err| format!("qwen native edit request failed: {err}"))?;
    let create_status = create_resp.status().as_u16();
    let create_v: Value = create_resp
        .json()
        .map_err(|err| format!("parse qwen native edit create response failed: {err}"))?;
    if create_status >= 300 {
        return Err(format!(
            "qwen native edit create error status={create_status}: {}",
            truncate(&create_v.to_string(), 400)
        ));
    }
    if let Some(url) = extract_qwen_output_image_url(&create_v) {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download edited image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read edited image bytes failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    let task_id = create_v
        .get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| {
            format!(
                "qwen native edit response missing task_id/image: {}",
                truncate(&create_v.to_string(), 400)
            )
        })?;
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(10));
    let task_url = format!("https://dashscope.aliyuncs.com/api/v1/tasks/{task_id}");
    loop {
        if Instant::now() > deadline {
            return Err(format!("qwen native edit task timeout: task_id={task_id}"));
        }
        let task_resp = client
            .get(&task_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("qwen native edit poll failed: {err}"))?;
        let task_status = task_resp.status().as_u16();
        let task_v: Value = task_resp
            .json()
            .map_err(|err| format!("parse qwen native edit task response failed: {err}"))?;
        if task_status >= 300 {
            return Err(format!(
                "qwen native edit poll error status={task_status}: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        let status = task_v
            .get("output")
            .and_then(|o| o.get("task_status"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_ascii_uppercase();
        if status == "SUCCEEDED" {
            let url = extract_qwen_output_image_url(&task_v).ok_or_else(|| {
                format!(
                    "qwen native edit success response missing image url: {}",
                    truncate(&task_v.to_string(), 400)
                )
            })?;
            let bytes = client
                .get(url)
                .send()
                .map_err(|err| format!("download edited image failed: {err}"))?
                .bytes()
                .map_err(|err| format!("read edited image bytes failed: {err}"))?;
            ensure_parent_dir(output_path)?;
            std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
            return Ok(());
        }
        if status == "FAILED" || status == "CANCELED" || status == "CANCELLED" {
            return Err(format!(
                "qwen native edit task failed: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        thread::sleep(Duration::from_millis(1200));
    }
}

#[allow(clippy::too_many_arguments)]
fn openai_compatible_edit(
    client: &Client,
    vendor_name: &str,
    cfg: &VendorConfig,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    quality: Option<&str>,
    n: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    let (image_bytes, image_mime) = load_image_bytes(client, image, max_input_bytes)?;
    let image_part = multipart::Part::bytes(image_bytes)
        .file_name("image.png")
        .mime_str(&image_mime)
        .map_err(|err| format!("set image mime failed: {err}"))?;

    let mut form = multipart::Form::new()
        .text("model", model.to_string())
        .text("prompt", instruction.to_string())
        .text("size", size.to_string())
        .text("n", n.to_string())
        .part("image", image_part);

    if let Some(q) = quality {
        form = form.text("quality", q.to_string());
    }
    if let Some(mask_source) = mask {
        let (mask_bytes, mask_mime) = load_image_bytes(client, mask_source, max_input_bytes)?;
        let mask_part = multipart::Part::bytes(mask_bytes)
            .file_name("mask.png")
            .mime_str(&mask_mime)
            .map_err(|err| format!("set mask mime failed: {err}"))?;
        form = form.part("mask", mask_part);
    }

    let url = format!("{}/images/edits", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .multipart(form)
        .send()
        .map_err(|err| format!("{vendor_name} request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse {vendor_name} response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "{vendor_name} error status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
    }

    let item = v
        .get("data")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| {
            format!(
                "{vendor_name} response missing data: {}",
                truncate(&v.to_string(), 400)
            )
        })?;
    if let Some(b64) = item.get("b64_json").and_then(|v| v.as_str()) {
        let bytes = STANDARD
            .decode(b64)
            .map_err(|err| format!("decode image base64 failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download edited image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read edited image bytes failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }

    Err(format!(
        "{vendor_name} response contains no image payload: {}",
        truncate(&v.to_string(), 400)
    ))
}

#[allow(clippy::too_many_arguments)]
fn google_edit(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    instruction: &str,
    image: &ImageSource,
    mask: Option<&ImageSource>,
    size: &str,
    quality: Option<&str>,
    _n: u64,
    max_input_bytes: usize,
    output_path: &Path,
) -> Result<(), String> {
    let mut parts = vec![json!({"text": format!(
        "Edit this image. Size hint: {size}. {}{}",
        instruction,
        quality.map(|q| format!(" Quality: {q}.")).unwrap_or_default()
    )})];
    let (image_bytes, image_mime) = load_image_bytes(client, image, max_input_bytes)?;
    parts.push(json!({"inline_data": {
        "mime_type": image_mime,
        "data": STANDARD.encode(image_bytes)
    }}));
    if let Some(mask_source) = mask {
        let (mask_bytes, mask_mime) = load_image_bytes(client, mask_source, max_input_bytes)?;
        parts.push(json!({"inline_data": {
            "mime_type": mask_mime,
            "data": STANDARD.encode(mask_bytes)
        }}));
        parts.push(json!({"text": "Second image is mask guidance."}));
    }
    let body = json!({
        "contents": [{"parts": parts}],
        "generationConfig": {"responseModalities": ["TEXT", "IMAGE"]}
    });
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
        return Err(format!("google error status={status}: {}", truncate(&v.to_string(), 400)));
    }
    if let Some(parts) = v
        .get("candidates")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("content"))
        .and_then(|c| c.get("parts"))
        .and_then(|p| p.as_array())
    {
        for part in parts {
            if let Some(b64) = part
                .get("inlineData")
                .or_else(|| part.get("inline_data"))
                .and_then(|i| i.get("data"))
                .and_then(|d| d.as_str())
            {
                let bytes = STANDARD
                    .decode(b64)
                    .map_err(|err| format!("decode google image base64 failed: {err}"))?;
                ensure_parent_dir(output_path)?;
                std::fs::write(output_path, bytes)
                    .map_err(|err| format!("write output failed: {err}"))?;
                return Ok(());
            }
        }
    }
    Err(format!(
        "google response contains no image payload: {}",
        truncate(&v.to_string(), 400)
    ))
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
    let put_url = format!("https://{}.{}{}", bucket, endpoint, object_path(&object_key));
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
        "add_remove" => format!("Add/remove elements as requested while preserving realism. {instruction}"),
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
    let mut merged = match std::fs::read_to_string(root.join("configs/config.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
    {
        Some(v) => v,
        None => toml::Value::Table(toml::map::Map::new()),
    };
    if let Some(image_cfg) = std::fs::read_to_string(root.join("configs/image.toml"))
        .ok()
        .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
    {
        // Keep config.toml higher priority if same key exists, and use image.toml as defaults.
        merge_missing_toml(&mut merged, image_cfg);
    }
    let mut cfg = RootConfig::default();
    if let Some(v) = merged.get("llm").cloned() {
        if let Ok(parsed) = v.try_into::<LlmConfig>() {
            cfg.llm = parsed;
        }
    }
    if let Some(v) = merged.get("image_edit").cloned() {
        if let Ok(parsed) = v.try_into::<ImageSkillConfig>() {
            cfg.image_edit = parsed;
        }
    }
    if let Some(v) = merged.get("command_intent").cloned() {
        if let Ok(parsed) = v.try_into::<CommandIntentConfig>() {
            cfg.command_intent = parsed;
        }
    }
    cfg
}

fn merge_missing_toml(dst: &mut toml::Value, src: toml::Value) {
    if let (toml::Value::Table(dst_map), toml::Value::Table(src_map)) = (dst, src) {
        for (key, src_val) in src_map {
            match dst_map.get_mut(&key) {
                Some(dst_val) => merge_missing_toml(dst_val, src_val),
                None => {
                    dst_map.insert(key, src_val);
                }
            }
        }
    }
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
        VendorKind::Qwen,
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
        "qwen" => Some(VendorKind::Qwen),
        _ => None,
    }
}

fn vendor_name(v: VendorKind) -> &'static str {
    match v {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "anthropic",
        VendorKind::Grok => "grok",
        VendorKind::Qwen => "qwen",
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
    let mut mac =
        HmacSha1::new_from_slice(secret.as_bytes()).map_err(|err| format!("invalid HMAC key: {err}"))?;
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
mod tests {
    use super::*;

    #[test]
    fn parse_vendor_aliases() {
        assert_eq!(parse_vendor("openai"), Some(VendorKind::OpenAI));
        assert_eq!(parse_vendor("gemini"), Some(VendorKind::Google));
        assert_eq!(parse_vendor("claude"), Some(VendorKind::Anthropic));
        assert_eq!(parse_vendor("xai"), Some(VendorKind::Grok));
        assert_eq!(parse_vendor("qwen"), Some(VendorKind::Qwen));
    }

    #[test]
    fn rewrite_for_restyle() {
        let v = rewrite_instruction("restyle", "make it watercolor");
        assert!(v.contains("restyle"));
    }

    #[test]
    fn split_data_url() {
        let (mime, data) = split_image_data("data:image/png;base64,abc");
        assert_eq!(mime, "image/png");
        assert_eq!(data, "abc");
    }

    #[test]
    fn native_edit_supports_local_upload_when_enabled() {
        let cfg = ImageSkillConfig {
            local_auto_upload_enabled: true,
            ..Default::default()
        };
        assert!(qwen_native_edit_inputs_supported(
            &cfg,
            "wanx2.1-imageedit",
            &ImageSource::Path(PathBuf::from("/tmp/demo.png")),
            Some(&ImageSource::Base64("data:image/png;base64,abc".to_string()))
        ));
    }

    #[test]
    fn sanitize_oss_name_keeps_safe_chars() {
        assert_eq!(sanitize_oss_filename("a b/c?.png"), "a_b_c_.png");
    }

    #[test]
    fn multimodal_native_edit_supports_local_without_oss() {
        let cfg = ImageSkillConfig::default();
        assert!(qwen_native_edit_inputs_supported(
            &cfg,
            "wan2.6-image",
            &ImageSource::Path(PathBuf::from("/tmp/demo.png")),
            None
        ));
        assert!(qwen_native_edit_inputs_supported(
            &cfg,
            "qwen-image-edit-max",
            &ImageSource::Path(PathBuf::from("/tmp/demo.png")),
            None
        ));
    }

    #[test]
    fn extract_qwen_choice_image_url() {
        let v = json!({
            "output": {
                "choices": [{
                    "message": {
                        "content": [{
                            "type": "image",
                            "image": "https://example.com/demo.png"
                        }]
                    }
                }]
            }
        });
        assert_eq!(extract_qwen_output_image_url(&v), Some("https://example.com/demo.png"));
    }
}
