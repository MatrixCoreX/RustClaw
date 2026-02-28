use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

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
    audio_transcribe: AudioTranscribeConfig,
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
struct AudioTranscribeConfig {
    #[serde(default)]
    default_vendor: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    max_input_bytes: Option<usize>,
}

#[derive(Debug, Clone, Copy)]
enum VendorKind {
    OpenAI,
    Google,
    Anthropic,
    Grok,
}

const DEFAULT_AUDIO_TRANSCRIBE_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/audio_transcribe_prompt.md");

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
    let audio_path = parse_audio_path(&args, workspace_root)?;
    let max_input_bytes = cfg.audio_transcribe.max_input_bytes.unwrap_or(25 * 1024 * 1024);
    let metadata = std::fs::metadata(&audio_path).map_err(|err| format!("read audio metadata failed: {err}"))?;
    if metadata.len() as usize > max_input_bytes {
        return Err(format!(
            "audio file too large: {} bytes, max={max_input_bytes}",
            metadata.len()
        ));
    }

    let args_obj = args.as_object();
    let transcribe_hint = args_obj
        .and_then(|v| v.get("transcribe_hint"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let transcribe_prompt_template = load_prompt_template(
        workspace_root,
        "prompts/audio_transcribe_prompt.md",
        DEFAULT_AUDIO_TRANSCRIBE_PROMPT_TEMPLATE,
    );
    let transcribe_prompt = render_transcribe_prompt(&transcribe_prompt_template, transcribe_hint);
    let requested_vendor = args_obj
        .and_then(|v| v.get("vendor"))
        .and_then(|v| v.as_str());
    let vendor = select_vendor(
        requested_vendor,
        cfg.audio_transcribe.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let (vendor_name, provider_cfg) = resolve_vendor_config(cfg, vendor)?;
    check_api_key(vendor_name, &provider_cfg.api_key)?;
    let model = cfg
        .audio_transcribe
        .default_model
        .as_deref()
        .or_else(|| args_obj.and_then(|v| v.get("model")).and_then(|v| v.as_str()))
        .unwrap_or(&provider_cfg.model)
        .to_string();
    let timeout_seconds = cfg
        .audio_transcribe
        .timeout_seconds
        .unwrap_or(provider_cfg.timeout_seconds.unwrap_or(60))
        .clamp(5, 300);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
    let text = openai_compatible_transcribe(
        &client,
        provider_cfg,
        vendor_name,
        &model,
        &audio_path,
        &transcribe_prompt,
    )?;
    let extra = json!({
        "provider": vendor_name,
        "model": model,
        "audio_path": audio_path.to_string_lossy().to_string()
    });
    Ok((text, extra))
}

fn parse_audio_path(args: &Value, workspace_root: &Path) -> Result<PathBuf, String> {
    let path = if let Some(obj) = args.as_object() {
        obj.get("audio")
            .and_then(|v| v.get("path"))
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("path").and_then(|v| v.as_str()))
    } else if let Some(s) = args.as_str() {
        Some(s)
    } else {
        None
    }
    .map(|s| s.trim())
    .filter(|s| !s.is_empty())
    .ok_or_else(|| "audio path is required (args.audio.path or args.path)".to_string())?;

    to_workspace_path(workspace_root, path)
}

fn openai_compatible_transcribe(
    client: &Client,
    cfg: &VendorConfig,
    vendor_name: &str,
    model: &str,
    audio_path: &Path,
    prompt: &str,
) -> Result<String, String> {
    if !audio_path.exists() || !audio_path.is_file() {
        return Err("audio file does not exist".to_string());
    }
    let url = format!("{}/audio/transcriptions", trim_trailing_slash(&cfg.base_url));
    let form = multipart::Form::new()
        .text("model", model.to_string())
        .text("prompt", prompt.to_string())
        .file("file", audio_path)
        .map_err(|err| format!("attach audio file failed: {err}"))?;
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .multipart(form)
        .send()
        .map_err(|err| format!("{vendor_name} transcription request failed: {err}"))?;
    let status = resp.status().as_u16();
    let body = resp
        .text()
        .map_err(|err| format!("read {vendor_name} transcription response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "{vendor_name} transcription failed status={status}: {}",
            truncate(&body, 400)
        ));
    }
    let parsed_json: Result<Value, _> = serde_json::from_str(&body);
    if let Ok(v) = parsed_json {
        if let Some(text) = v.get("text").and_then(|t| t.as_str()) {
            let out = text.trim();
            if !out.is_empty() {
                return Ok(out.to_string());
            }
        }
    }
    let out = body.trim();
    if out.is_empty() {
        return Err("transcription result is empty".to_string());
    }
    Ok(out.to_string())
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

fn select_vendor(
    requested: Option<&str>,
    section_default: Option<&str>,
    selected_vendor: Option<&str>,
) -> VendorKind {
    requested
        .and_then(parse_vendor)
        .or_else(|| section_default.and_then(parse_vendor))
        .or_else(|| selected_vendor.and_then(parse_vendor))
        .unwrap_or(VendorKind::OpenAI)
}

fn resolve_vendor_config<'a>(
    cfg: &'a RootConfig,
    vendor: VendorKind,
) -> Result<(&'static str, &'a VendorConfig), String> {
    match vendor {
        VendorKind::OpenAI => cfg
            .llm
            .openai
            .as_ref()
            .map(|v| ("openai", v))
            .ok_or_else(|| "openai config missing".to_string()),
        VendorKind::Google => cfg
            .llm
            .google
            .as_ref()
            .map(|v| ("google", v))
            .ok_or_else(|| "google config missing".to_string()),
        VendorKind::Anthropic => cfg
            .llm
            .anthropic
            .as_ref()
            .map(|v| ("anthropic", v))
            .ok_or_else(|| "anthropic config missing".to_string()),
        VendorKind::Grok => cfg
            .llm
            .grok
            .as_ref()
            .map(|v| ("grok", v))
            .ok_or_else(|| "grok config missing".to_string()),
    }
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

fn load_prompt_template(workspace_root: &Path, rel_path: &str, default_template: &str) -> String {
    let path = workspace_root.join(rel_path);
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => default_template.to_string(),
    }
}

fn render_transcribe_prompt(template: &str, hint: &str) -> String {
    template.replace("__TRANSCRIBE_HINT__", hint.trim())
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn to_workspace_path(workspace_root: &Path, input: &str) -> Result<PathBuf, String> {
    let p = Path::new(input);
    let joined = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    if !joined.starts_with(workspace_root) {
        return Err("audio path is outside workspace".to_string());
    }
    Ok(joined)
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
