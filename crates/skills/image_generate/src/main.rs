use std::collections::{HashMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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
    image_generation: ImageSkillConfig,
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
    qwen_native_base_url: Option<String>,
    #[serde(default)]
    adapter_mode: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    allow_compat_adapters: bool,
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

#[derive(Debug, Clone)]
struct TextCatalog {
    current: HashMap<String, String>,
}

impl TextCatalog {
    fn for_lang(workspace_root: &Path, cfg: &ImageSkillConfig, lang: &str) -> Self {
        let mut current = default_i18n_dict(lang);
        let lang_tag = normalize_lang_tag(lang);
        let default_path = workspace_root.join(format!("configs/i18n/image_generate.{lang_tag}.toml"));
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
    let prompt = obj
        .get("prompt")
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "prompt is required".to_string())?;
    let size = obj
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("1024x1024");
    let style = obj.get("style").and_then(|v| v.as_str());
    let quality = obj.get("quality").and_then(|v| v.as_str());
    let n = obj.get("n").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 4);
    let timeout_seconds = obj
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or_else(|| cfg.image_generation.timeout_seconds.unwrap_or(120))
        .clamp(5, 300);

    let requested_vendor = obj.get("vendor").and_then(|v| v.as_str());
    let requested_model = obj.get("model").and_then(|v| v.as_str());
    let providers = vendor_order(
        requested_vendor,
        cfg.image_generation.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    if providers.is_empty() {
        return Err("no vendor configured".to_string());
    }

    let output_path = resolve_output_path(
        workspace_root,
        cfg.image_generation
            .default_output_dir
            .as_deref()
            .unwrap_or("image"),
        obj.get("output_path").and_then(|v| v.as_str()),
    )?;
    let output_lang = resolve_output_language(cfg, obj);
    let i18n = TextCatalog::for_lang(workspace_root, &cfg.image_generation, &output_lang);

    let mut provider_errors: Vec<String> = Vec::new();
    for vendor in providers {
        let config_default_model = first_model_candidate(
            cfg.image_generation.default_model.as_deref(),
            vendor_models(&cfg.image_generation, vendor),
            cfg.image_generation.models.as_ref(),
        );
        match call_generate(
            vendor,
            cfg,
            requested_model.or(config_default_model),
            timeout_seconds,
            prompt,
            size,
            style,
            quality,
            n,
            &output_path,
        ) {
            Ok((model, model_kind)) => {
                let saved_path = output_path.to_string_lossy().to_string();
                let preface = i18n.render(
                    "image_generate.msg.saved",
                    &[("path", saved_path.clone())],
                    "Generated successfully and saved: {path}",
                );
                let text = format!("{preface}\nFILE:{saved_path}\nEPHEMERAL:IMAGE_SAVED");
                let extra = json!({
                    "provider": vendor_name(vendor),
                    "model": model,
                    "model_kind": model_kind,
                    "latency_ms": 0,
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
        VendorKind::DeepSeek => cfg.deepseek_models.as_ref(),
        VendorKind::Qwen => cfg.qwen_models.as_ref(),
        VendorKind::MiniMax => cfg.minimax_models.as_ref(),
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
            cfg.image_generation
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
            "image_generate.msg.saved".to_string(),
            "图片生成成功并已保存：{path}".to_string(),
        );
    } else {
        out.insert(
            "image_generate.msg.saved".to_string(),
            "Generated successfully and saved: {path}".to_string(),
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
fn call_generate(
    vendor: VendorKind,
    cfg: &RootConfig,
    requested_model: Option<&str>,
    timeout_seconds: u64,
    prompt: &str,
    size: &str,
    style: Option<&str>,
    quality: Option<&str>,
    n: u64,
    output_path: &Path,
) -> Result<(String, &'static str), String> {
    let mode = resolve_adapter_mode(&cfg.image_generation);
    let (vendor_name, vcfg) = resolve_vendor_config(cfg, vendor)?;
    check_api_key(vendor_name, &vcfg.api_key)?;
    match vendor {
        VendorKind::OpenAI => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build openai client failed: {err}"))?;
            openai_compatible_generate(
                &client,
                "openai",
                vcfg,
                &model,
                prompt,
                size,
                style,
                quality,
                n,
                output_path,
            )?;
            Ok((model, "compat"))
        }
        VendorKind::Google => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build google client failed: {err}"))?;
            google_generate(
                &client, vcfg, &model, prompt, size, style, quality, n, output_path,
            )?;
            Ok((model, "native"))
        }
        VendorKind::Anthropic => {
            if mode == AdapterMode::Native {
                return Err("anthropic native image generation adapter is not available".to_string());
            }
            if !cfg.image_generation.allow_compat_adapters && mode != AdapterMode::Compat {
                return Err(
                    "anthropic native image generation adapter is not available; set image_generation.allow_compat_adapters=true to use compatible endpoint"
                        .to_string(),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build anthropic client failed: {err}"))?;
            openai_compatible_generate(
                &client,
                "anthropic",
                vcfg,
                &model,
                prompt,
                size,
                style,
                quality,
                n,
                output_path,
            )?;
            Ok((model, "compat"))
        }
        VendorKind::Grok | VendorKind::DeepSeek => {
            if mode == AdapterMode::Native {
                return Err(format!(
                    "{vendor_name} native image generation adapter is not available"
                ));
            }
            if !cfg.image_generation.allow_compat_adapters && mode != AdapterMode::Compat {
                return Err(
                    format!(
                        "{vendor_name} native image generation adapter is not available; set image_generation.allow_compat_adapters=true to use compatible endpoint"
                    ),
                );
            }
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
            openai_compatible_generate(
                &client, vendor_name, vcfg, &model, prompt, size, style, quality, n, output_path,
            )?;
            Ok((model, "compat"))
        }
        VendorKind::MiniMax => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
            minimax_generate(
                &client,
                vcfg,
                &model,
                prompt,
                size,
                style,
                quality,
                n,
                output_path,
            )?;
            Ok((model, "native"))
        }
        VendorKind::Qwen => {
            let model = requested_model.unwrap_or(&vcfg.model).to_string();
            let client = Client::builder()
                .timeout(Duration::from_secs(timeout_seconds.max(vcfg.timeout_seconds.unwrap_or(30))))
                .build()
                .map_err(|err| format!("build qwen client failed: {err}"))?;
            if should_use_qwen_native(
                &cfg.image_generation,
                &model,
                mode,
                cfg.image_generation.allow_compat_adapters,
            ) {
                qwen_native_generate(
                    &client,
                    cfg.image_generation.qwen_native_base_url.as_deref(),
                    &vcfg.api_key,
                    &model,
                    prompt,
                    size,
                    n,
                    timeout_seconds,
                    output_path,
                )?;
                return Ok((model, "native"));
            } else {
                if !cfg.image_generation.allow_compat_adapters {
                    return Err(
                        "qwen native image generation adapter is not available; set image_generation.allow_compat_adapters=true to use compatible endpoint"
                            .to_string(),
                    );
                }
                openai_compatible_generate(
                    &client, "qwen", vcfg, &model, prompt, size, style, quality, n, output_path,
                )?;
                Ok((model, "compat"))
            }
        }
    }
}

fn qwen_uses_native_image_api(cfg: &ImageSkillConfig, model: &str) -> bool {
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

fn should_use_qwen_native(
    cfg: &ImageSkillConfig,
    model: &str,
    mode: AdapterMode,
    allow_compat: bool,
) -> bool {
    match mode {
        AdapterMode::Native => true,
        AdapterMode::Compat => false,
        AdapterMode::Auto => {
            if qwen_uses_native_image_api(cfg, model) {
                true
            } else {
                !allow_compat
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn qwen_native_generate(
    client: &Client,
    native_base_url: Option<&str>,
    api_key: &str,
    model: &str,
    prompt: &str,
    size: &str,
    n: u64,
    timeout_seconds: u64,
    output_path: &Path,
) -> Result<(), String> {
    if is_qwen_wan26_image(model) {
        return qwen_wan26_generate(
            client,
            native_base_url,
            api_key,
            model,
            prompt,
            size,
            n,
            timeout_seconds,
            output_path,
        );
    }
    let base = native_base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("https://dashscope.aliyuncs.com/api/v1");
    let url = format!(
        "{}/services/aigc/text2image/image-synthesis",
        trim_trailing_slash(base)
    );
    let normalized_size = size.trim().replace('x', "*").replace('X', "*");
    let body = json!({
        "model": model,
        "input": { "prompt": prompt },
        "parameters": {
            "size": normalized_size,
            "n": n,
            "prompt_extend": true,
            "watermark": false
        }
    });

    let create_resp = client
        .post(url)
        .bearer_auth(api_key)
        .header("X-DashScope-Async", "enable")
        .json(&body)
        .send()
        .map_err(|err| format!("qwen native request failed: {err}"))?;
    let create_status = create_resp.status().as_u16();
    let create_v: Value = create_resp
        .json()
        .map_err(|err| format!("parse qwen native create response failed: {err}"))?;
    if create_status >= 300 {
        return Err(format!(
            "qwen native create error status={create_status}: {}",
            truncate(&create_v.to_string(), 400)
        ));
    }

    if let Some(url) = extract_qwen_output_image_url(&create_v) {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download generated image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read generated image bytes failed: {err}"))?;
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
                "qwen native response missing task_id/results: {}",
                truncate(&create_v.to_string(), 400)
            )
        })?;

    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(10));
    let task_url = format!("{}/tasks/{task_id}", trim_trailing_slash(base));
    loop {
        if Instant::now() > deadline {
            return Err(format!("qwen native task timeout: task_id={task_id}"));
        }
        let task_resp = client
            .get(&task_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("qwen native poll failed: {err}"))?;
        let task_status = task_resp.status().as_u16();
        let task_v: Value = task_resp
            .json()
            .map_err(|err| format!("parse qwen native task response failed: {err}"))?;
        if task_status >= 300 {
            return Err(format!(
                "qwen native poll error status={task_status}: {}",
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
                    "qwen native success response missing image url: {}",
                    truncate(&task_v.to_string(), 400)
                )
            })?;
            let bytes = client
                .get(url)
                .send()
                .map_err(|err| format!("download generated image failed: {err}"))?
                .bytes()
                .map_err(|err| format!("read generated image bytes failed: {err}"))?;
            ensure_parent_dir(output_path)?;
            std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
            return Ok(());
        }
        if status == "FAILED" || status == "CANCELED" || status == "CANCELLED" {
            return Err(format!(
                "qwen native task failed: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }

        thread::sleep(Duration::from_millis(1200));
    }
}

#[allow(clippy::too_many_arguments)]
fn qwen_wan26_generate(
    client: &Client,
    native_base_url: Option<&str>,
    api_key: &str,
    model: &str,
    prompt: &str,
    size: &str,
    n: u64,
    timeout_seconds: u64,
    output_path: &Path,
) -> Result<(), String> {
    let base = native_base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("https://dashscope.aliyuncs.com/api/v1");
    let url = format!(
        "{}/services/aigc/image-generation/generation",
        trim_trailing_slash(base)
    );
    let body = json!({
        "model": model,
        "input": {
            "messages": [{
                "role": "user",
                "content": [{
                    "text": prompt
                }]
            }]
        },
        "parameters": {
            "size": normalize_wan26_size(size),
            "n": n,
            "prompt_extend": true,
            "watermark": false,
            "enable_interleave": true
        }
    });
    let create_resp = client
        .post(url)
        .bearer_auth(api_key)
        .header("X-DashScope-Async", "enable")
        .json(&body)
        .send()
        .map_err(|err| format!("qwen native request failed: {err}"))?;
    let create_status = create_resp.status().as_u16();
    let create_v: Value = create_resp
        .json()
        .map_err(|err| format!("parse qwen native create response failed: {err}"))?;
    if create_status >= 300 {
        return Err(format!(
            "qwen native create error status={create_status}: {}",
            truncate(&create_v.to_string(), 400)
        ));
    }
    if let Some(url) = extract_qwen_output_image_url(&create_v) {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download generated image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read generated image bytes failed: {err}"))?;
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
                "qwen native response missing task_id/image: {}",
                truncate(&create_v.to_string(), 400)
            )
        })?;
    let deadline = Instant::now() + Duration::from_secs(timeout_seconds.max(10));
    let task_url = format!("{}/tasks/{task_id}", trim_trailing_slash(base));
    loop {
        if Instant::now() > deadline {
            return Err(format!("qwen native task timeout: task_id={task_id}"));
        }
        let task_resp = client
            .get(&task_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("qwen native poll failed: {err}"))?;
        let task_status = task_resp.status().as_u16();
        let task_v: Value = task_resp
            .json()
            .map_err(|err| format!("parse qwen native task response failed: {err}"))?;
        if task_status >= 300 {
            return Err(format!(
                "qwen native poll error status={task_status}: {}",
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
                    "qwen native success response missing image url: {}",
                    truncate(&task_v.to_string(), 400)
                )
            })?;
            let bytes = client
                .get(url)
                .send()
                .map_err(|err| format!("download generated image failed: {err}"))?
                .bytes()
                .map_err(|err| format!("read generated image bytes failed: {err}"))?;
            ensure_parent_dir(output_path)?;
            std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
            return Ok(());
        }
        if status == "FAILED" || status == "CANCELED" || status == "CANCELLED" {
            return Err(format!(
                "qwen native task failed: {}",
                truncate(&task_v.to_string(), 400)
            ));
        }
        thread::sleep(Duration::from_millis(1200));
    }
}

#[allow(clippy::too_many_arguments)]
fn openai_compatible_generate(
    client: &Client,
    vendor_name: &str,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    size: &str,
    style: Option<&str>,
    quality: Option<&str>,
    n: u64,
    output_path: &Path,
) -> Result<(), String> {
    let mut body = json!({
        "model": model,
        "prompt": prompt,
        "size": size,
        "n": n
    });
    if let Some(v) = style {
        let normalized = v.trim().to_ascii_lowercase();
        if normalized == "vivid" || normalized == "natural" {
            body["style"] = Value::String(normalized);
        }
    }
    if let Some(v) = quality {
        body["quality"] = Value::String(v.to_string());
    }

    let url = format!("{}/images/generations", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
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
            .map_err(|err| format!("download generated image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read generated image bytes failed: {err}"))?;
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
fn minimax_generate(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    size: &str,
    _style: Option<&str>,
    _quality: Option<&str>,
    n: u64,
    output_path: &Path,
) -> Result<(), String> {
    let url = format!("{}/image_generation", trim_trailing_slash(&cfg.base_url));
    let mut body = json!({
        "model": model,
        "prompt": prompt,
        "response_format": "url",
        "n": n.max(1),
        "prompt_optimizer": true
    });
    body["aspect_ratio"] = Value::String(size_to_minimax_aspect_ratio(size));

    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .map_err(|err| format!("minimax request failed: {err}"))?;
    let status = resp.status().as_u16();
    let raw = resp
        .text()
        .map_err(|err| format!("read minimax response failed: {err}"))?;
    let v: Value = serde_json::from_str(&raw)
        .map_err(|err| format!("parse minimax response failed: {err}; body={}", truncate(&raw, 400)))?;
    if status >= 300 {
        return Err(format!(
            "minimax error status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
    }

    if let Some(url) = v
        .get("data")
        .and_then(|d| d.get("image_urls"))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
    {
        let bytes = client
            .get(url)
            .send()
            .map_err(|err| format!("download generated image failed: {err}"))?
            .bytes()
            .map_err(|err| format!("read generated image bytes failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, &bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    if let Some(b64) = v
        .get("data")
        .and_then(|d| d.get("image_base64"))
        .and_then(|arr| arr.as_array())
        .and_then(|arr| arr.first())
        .and_then(|v| v.as_str())
    {
        let bytes = STANDARD
            .decode(b64)
            .map_err(|err| format!("decode minimax image base64 failed: {err}"))?;
        ensure_parent_dir(output_path)?;
        std::fs::write(output_path, bytes).map_err(|err| format!("write output failed: {err}"))?;
        return Ok(());
    }
    Err(format!(
        "minimax response contains no image payload: {}",
        truncate(&v.to_string(), 400)
    ))
}

#[allow(clippy::too_many_arguments)]
fn google_generate(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    prompt: &str,
    size: &str,
    style: Option<&str>,
    quality: Option<&str>,
    _n: u64,
    output_path: &Path,
) -> Result<(), String> {
    let mut full_prompt = format!("Generate one image. Size hint: {size}. Prompt: {prompt}");
    if let Some(v) = style {
        full_prompt.push_str(&format!(" Style: {v}."));
    }
    if let Some(v) = quality {
        full_prompt.push_str(&format!(" Quality: {v}."));
    }
    let body = json!({
        "contents": [{"parts":[{"text": full_prompt}]}],
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
    let file_name = format!("gen-{}.png", unix_ts());
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
    if let Some(v) = image_cfg.get("image_generation").cloned() {
        if let Ok(parsed) = v.try_into::<ImageSkillConfig>() {
            cfg.image_generation = parsed;
        }
    }
    if let Some(v) = core_cfg.get("command_intent").cloned() {
        if let Ok(parsed) = v.try_into::<CommandIntentConfig>() {
            cfg.command_intent = parsed;
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
    let section = &cfg.image_generation.providers;
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

fn trim_trailing_slash(v: &str) -> String {
    v.trim_end_matches('/').to_string()
}

fn size_to_minimax_aspect_ratio(size: &str) -> String {
    let normalized = size.trim().replace('X', "x");
    let parts = normalized.split('x').collect::<Vec<_>>();
    if parts.len() == 2 {
        if let (Ok(w), Ok(h)) = (parts[0].trim().parse::<u64>(), parts[1].trim().parse::<u64>()) {
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

fn is_qwen_wan26_image(model: &str) -> bool {
    model.trim().eq_ignore_ascii_case("wan2.6-image")
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
