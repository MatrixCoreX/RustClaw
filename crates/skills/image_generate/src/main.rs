use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    timeout_seconds: Option<u64>,
    #[serde(default)]
    allow_compat_adapters: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum VendorKind {
    OpenAI,
    Google,
    Anthropic,
    Grok,
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

    let mut provider_errors: Vec<String> = Vec::new();
    for vendor in providers {
        match call_generate(
            vendor,
            cfg,
            requested_model.or(cfg.image_generation.default_model.as_deref()),
            timeout_seconds,
            prompt,
            size,
            style,
            quality,
            n,
            &output_path,
        ) {
            Ok(model) => {
                let saved_path = output_path.to_string_lossy().to_string();
                let text = format!(
                    "Generated successfully and saved: {saved_path}\nFILE:{saved_path}"
                );
                let extra = json!({
                    "provider": vendor_name(vendor),
                    "model": model,
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
) -> Result<String, String> {
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
            google_generate(
                &client, vcfg, &model, prompt, size, style, quality, n, output_path,
            )?;
            Ok(model)
        }
        VendorKind::Anthropic => {
            if !cfg.image_generation.allow_compat_adapters {
                return Err(
                    "anthropic native image generation adapter is not available; set image_generation.allow_compat_adapters=true to use compatible endpoint"
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
            Ok(model)
        }
        VendorKind::Grok => {
            if !cfg.image_generation.allow_compat_adapters {
                return Err(
                    "grok native image generation adapter is not available; set image_generation.allow_compat_adapters=true to use compatible endpoint"
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
            openai_compatible_generate(
                &client, "grok", vcfg, &model, prompt, size, style, quality, n, output_path,
            )?;
            Ok(model)
        }
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
    let cfg_path = root.join("configs/config.toml");
    let raw = match std::fs::read_to_string(cfg_path) {
        Ok(v) => v,
        Err(_) => return RootConfig::default(),
    };
    toml::from_str::<RootConfig>(&raw).unwrap_or_default()
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
    for name in [
        requested,
        section_default,
        selected_vendor,
        Some("openai"),
        Some("google"),
        Some("anthropic"),
        Some("grok"),
    ]
    .into_iter()
    .flatten()
    {
        if let Some(v) = parse_vendor(name) {
            if seen.insert(v) {
                out.push(v);
            }
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
        _ => None,
    }
}

fn vendor_name(v: VendorKind) -> &'static str {
    match v {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "anthropic",
        VendorKind::Grok => "grok",
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
    }
}
