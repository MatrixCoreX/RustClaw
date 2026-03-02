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
    audio_synthesize: AudioSynthesizeConfig,
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
struct AudioSynthesizeConfig {
    #[serde(default)]
    default_vendor: Option<String>,
    #[serde(default)]
    default_output_dir: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    default_voice: Option<String>,
    #[serde(default)]
    default_format: Option<String>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    max_input_chars: Option<usize>,
    #[serde(default)]
    allow_compat_adapters: bool,
}

#[derive(Debug, Clone, Copy)]
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
    let input = obj
        .get("text")
        .or_else(|| obj.get("input"))
        .and_then(|v| v.as_str())
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "text is required".to_string())?;
    let max_input_chars = cfg.audio_synthesize.max_input_chars.unwrap_or(4000).max(100);
    if input.chars().count() > max_input_chars {
        return Err(format!(
            "text too long: {} chars, max={max_input_chars}",
            input.chars().count()
        ));
    }

    let voice = obj
        .get("voice")
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .or(cfg.audio_synthesize.default_voice.as_deref())
        .unwrap_or("alloy")
        .to_string();
    let response_format = obj
        .get("response_format")
        .or_else(|| obj.get("format"))
        .and_then(|v| v.as_str())
        .unwrap_or_else(|| cfg.audio_synthesize.default_format.as_deref().unwrap_or("opus"));
    let normalized_format = normalize_format(response_format);

    let output_path = resolve_output_path(
        workspace_root,
        cfg.audio_synthesize
            .default_output_dir
            .as_deref()
            .unwrap_or("audio/download"),
        obj.get("output_path").and_then(|v| v.as_str()),
        &normalized_format,
    )?;

    let requested_vendor = obj.get("vendor").and_then(|v| v.as_str());
    let vendor = select_vendor(
        requested_vendor,
        cfg.audio_synthesize.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let (vendor_name, provider_cfg) = resolve_vendor_config(cfg, vendor)?;
    check_api_key(vendor_name, &provider_cfg.api_key)?;
    let requested_model = obj.get("model").and_then(|v| v.as_str());
    let model = requested_model
        .or(cfg.audio_synthesize.default_model.as_deref())
        .unwrap_or(&provider_cfg.model)
        .to_string();
    let timeout_seconds = cfg
        .audio_synthesize
        .timeout_seconds
        .unwrap_or(provider_cfg.timeout_seconds.unwrap_or(60))
        .clamp(5, 300);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| format!("build {vendor_name} client failed: {err}"))?;
    synthesize_by_vendor(
        &client,
        provider_cfg,
        vendor,
        cfg.audio_synthesize.allow_compat_adapters,
        vendor_name,
        &model,
        &voice,
        &normalized_format,
        input,
        &output_path,
    )?;
    let saved_path = output_path.to_string_lossy().to_string();
    let extra = json!({
        "provider": vendor_name,
        "model": model,
        "voice": voice,
        "response_format": normalized_format,
        "output_path": saved_path,
        "outputs": [{"type":"audio_file","path": saved_path}],
        "latency_ms": 0
    });
    Ok((format!("VOICE_FILE:{saved_path}"), extra))
}

#[allow(clippy::too_many_arguments)]
fn synthesize_by_vendor(
    client: &Client,
    cfg: &VendorConfig,
    vendor: VendorKind,
    allow_compat_adapters: bool,
    vendor_name: &str,
    model: &str,
    voice: &str,
    response_format: &str,
    input: &str,
    output_path: &Path,
) -> Result<(), String> {
    match vendor {
        VendorKind::Google => google_native_synthesize(
            client,
            cfg,
            model,
            voice,
            response_format,
            input,
            output_path,
        ),
        VendorKind::OpenAI => openai_compatible_synthesize(
            client,
            cfg,
            vendor_name,
            model,
            voice,
            response_format,
            input,
            output_path,
        ),
        VendorKind::Anthropic | VendorKind::Grok => {
            if !allow_compat_adapters {
                return Err(format!(
                    "{vendor_name} native tts adapter is not available; set audio_synthesize.allow_compat_adapters=true to use compatible endpoint"
                ));
            }
            openai_compatible_synthesize(
                client,
                cfg,
                vendor_name,
                model,
                voice,
                response_format,
                input,
                output_path,
            )
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn openai_compatible_synthesize(
    client: &Client,
    cfg: &VendorConfig,
    vendor_name: &str,
    model: &str,
    voice: &str,
    response_format: &str,
    input: &str,
    output_path: &Path,
) -> Result<(), String> {
    let url = format!("{}/audio/speech", trim_trailing_slash(&cfg.base_url));
    let body = json!({
        "model": model,
        "voice": voice,
        "input": input,
        "response_format": response_format,
    });
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(&body)
        .send()
        .map_err(|err| format!("{vendor_name} tts request failed: {err}"))?;
    let status = resp.status().as_u16();
    let bytes = resp
        .bytes()
        .map_err(|err| format!("read {vendor_name} tts response failed: {err}"))?;
    if status >= 300 {
        let detail = String::from_utf8_lossy(&bytes).to_string();
        return Err(format!(
            "{vendor_name} tts failed status={status}: {}",
            truncate(&detail, 400)
        ));
    }
    ensure_parent_dir(output_path)?;
    std::fs::write(output_path, &bytes).map_err(|err| format!("write audio output failed: {err}"))?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn google_native_synthesize(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    voice: &str,
    response_format: &str,
    input: &str,
    output_path: &Path,
) -> Result<(), String> {
    let body = json!({
        "contents": [{"parts":[{"text": input}]}],
        "generationConfig": {"responseModalities": ["AUDIO"]},
        "speechConfig": {
            "voiceConfig": {"prebuiltVoiceConfig": {"voiceName": voice}}
        },
        "audioConfig": {"audioEncoding": google_audio_encoding(response_format)}
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
        .map_err(|err| format!("google tts request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse google tts response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "google tts failed status={status}: {}",
            truncate(&v.to_string(), 400)
        ));
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
                    .map_err(|err| format!("decode google tts base64 failed: {err}"))?;
                ensure_parent_dir(output_path)?;
                std::fs::write(output_path, bytes)
                    .map_err(|err| format!("write audio output failed: {err}"))?;
                return Ok(());
            }
        }
    }
    Err(format!(
        "google tts response missing audio payload: {}",
        truncate(&v.to_string(), 400)
    ))
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

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

fn normalize_format(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "mp3" => "mp3".to_string(),
        "aac" => "aac".to_string(),
        "flac" => "flac".to_string(),
        "wav" => "wav".to_string(),
        "pcm" => "pcm".to_string(),
        _ => "opus".to_string(),
    }
}

fn google_audio_encoding(response_format: &str) -> &'static str {
    match response_format {
        "mp3" => "MP3",
        "wav" => "LINEAR16",
        "aac" => "AAC",
        // Keep default compact codec for voice.
        _ => "OGG_OPUS",
    }
}

fn output_ext(response_format: &str) -> &'static str {
    match response_format {
        "mp3" => "mp3",
        "aac" => "aac",
        "flac" => "flac",
        "wav" => "wav",
        "pcm" => "pcm",
        // Telegram voice prefers opus-in-ogg.
        _ => "ogg",
    }
}

fn resolve_output_path(
    workspace_root: &Path,
    default_dir: &str,
    requested: Option<&str>,
    response_format: &str,
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
    let file_name = format!("tts-{}.{}", unix_ts(), output_ext(response_format));
    Ok(workspace_root.join(default_dir).join(file_name))
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "output path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|err| format!("create output dir failed: {err}"))
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
        assert!(matches!(parse_vendor("openai"), Some(VendorKind::OpenAI)));
        assert!(matches!(parse_vendor("gemini"), Some(VendorKind::Google)));
        assert!(matches!(parse_vendor("claude"), Some(VendorKind::Anthropic)));
        assert!(matches!(parse_vendor("xai"), Some(VendorKind::Grok)));
    }

    #[test]
    fn normalize_and_ext() {
        assert_eq!(normalize_format("mp3"), "mp3");
        assert_eq!(normalize_format("unknown"), "opus");
        assert_eq!(google_audio_encoding("mp3"), "MP3");
        assert_eq!(output_ext("opus"), "ogg");
    }
}
