use std::collections::HashSet;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::{multipart, Client};
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
    image_edit: ImageSkillConfig,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct LlmConfig {
    #[serde(default)]
    selected_vendor: Option<String>,
    #[serde(default)]
    openai: Option<VendorConfig>,
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
    max_input_bytes: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum VendorKind {
    OpenAI,
    Google,
    Anthropic,
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

    let effective_instruction = rewrite_instruction(&action, instruction);
    let size = obj
        .get("size")
        .and_then(|v| v.as_str())
        .unwrap_or("1024x1024");
    let quality = obj.get("quality").and_then(|v| v.as_str());
    let n = obj.get("n").and_then(|v| v.as_u64()).unwrap_or(1).clamp(1, 2);

    let mut provider_errors: Vec<String> = Vec::new();
    let mut unsupported_errors: Vec<String> = Vec::new();
    let mut attempted = 0usize;
    for vendor in providers {
        if !supports_edit(vendor) {
            unsupported_errors.push(format!(
                "vendor {} does not support image_edit",
                vendor_name(vendor)
            ));
            continue;
        }
        attempted += 1;
        match call_edit(
            vendor,
            cfg,
            requested_model.or(cfg.image_edit.default_model.as_deref()),
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
                let text = format!(
                    "Edited successfully and saved: {saved_path}\nFILE:{saved_path}"
                );
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
    if attempted == 0 {
        return Err(format!(
            "no supported provider for image_edit; requested vendor={}; {}",
            requested_vendor.unwrap_or("auto"),
            unsupported_errors.join("; ")
        ));
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
            openai_edit(
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
        VendorKind::Google => Err("google image edit adapter is not available yet".to_string()),
        VendorKind::Anthropic => Err("anthropic image edit adapter is not available yet".to_string()),
    }
}

#[allow(clippy::too_many_arguments)]
fn openai_edit(
    client: &Client,
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
        .map_err(|err| format!("openai request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse openai response failed: {err}"))?;
    if status >= 300 {
        return Err(format!("openai error status={status}: {}", truncate(&v.to_string(), 400)));
    }

    let item = v
        .get("data")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .ok_or_else(|| format!("openai response missing data: {}", truncate(&v.to_string(), 400)))?;
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
        "openai response contains no image payload: {}",
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

fn supports_edit(vendor: VendorKind) -> bool {
    matches!(vendor, VendorKind::OpenAI)
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
        _ => None,
    }
}

fn vendor_name(v: VendorKind) -> &'static str {
    match v {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "anthropic",
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
}
