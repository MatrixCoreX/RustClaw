use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use hmac::{Hmac, Mac};
use reqwest::blocking::{multipart, Client};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha1::Sha1;
use toml::Value as TomlValue;

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
    #[serde(default)]
    deepseek: Option<VendorConfig>,
    #[serde(default)]
    qwen: Option<VendorConfig>,
    #[serde(default)]
    minimax: Option<VendorConfig>,
    #[serde(default)]
    custom: Option<VendorConfig>,
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
    custom_models: Option<Vec<String>>,
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
    providers: AudioProviderOverrides,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct AudioProviderOverrides {
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
    custom: Option<VendorConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VendorKind {
    OpenAI,
    Google,
    Anthropic,
    Grok,
    DeepSeek,
    Qwen,
    MiniMax,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AdapterMode {
    Auto,
    Native,
    Compat,
}

#[derive(Debug, Clone)]
enum AudioInput {
    LocalPath(PathBuf),
    Url(String),
}

const DEFAULT_AUDIO_TRANSCRIBE_PROMPT_TEMPLATE: &str =
    include_str!("../../../../prompts/vendors/default/audio_transcribe_prompt.md");
const AUDIO_TRANSCRIBE_PROMPT_PATH: &str = "prompts/audio_transcribe_prompt.md";

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
    let audio_input = parse_audio_input(&args, workspace_root)?;
    let max_input_bytes = cfg
        .audio_transcribe
        .max_input_bytes
        .unwrap_or(25 * 1024 * 1024);
    if let AudioInput::LocalPath(audio_path) = &audio_input {
        let metadata = std::fs::metadata(audio_path)
            .map_err(|err| format!("read audio metadata failed: {err}"))?;
        if metadata.len() as usize > max_input_bytes {
            return Err(format!(
                "audio file too large: {} bytes, max={max_input_bytes}",
                metadata.len()
            ));
        }
    }

    let args_obj = args.as_object();
    let transcribe_hint = args_obj
        .and_then(|v| v.get("transcribe_hint"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let requested_vendor = args_obj
        .and_then(|v| v.get("vendor"))
        .and_then(|v| v.as_str());
    let vendor = select_vendor(
        requested_vendor,
        cfg.audio_transcribe.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let transcribe_prompt_template = load_prompt_template_for_vendor(
        workspace_root,
        prompt_vendor_name_for_vendor(vendor),
        AUDIO_TRANSCRIBE_PROMPT_PATH,
        DEFAULT_AUDIO_TRANSCRIBE_PROMPT_TEMPLATE,
    );
    let transcribe_prompt = render_transcribe_prompt(&transcribe_prompt_template, transcribe_hint);
    let (vendor_name, provider_cfg) = resolve_vendor_config(cfg, vendor)?;
    check_api_key(vendor_name, &provider_cfg.api_key)?;
    let requested_model = args_obj
        .and_then(|v| v.get("model"))
        .and_then(|v| v.as_str());
    let model = requested_model
        .or(first_model_candidate(
            cfg.audio_transcribe.default_model.as_deref(),
            vendor_models(&cfg.audio_transcribe, vendor),
            cfg.audio_transcribe.models.as_ref(),
        ))
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
    let (text, model_kind) = transcribe_by_vendor(
        &client,
        &cfg.audio_transcribe,
        provider_cfg,
        vendor,
        cfg.audio_transcribe.allow_compat_adapters,
        vendor_name,
        &model,
        &audio_input,
        &transcribe_prompt,
    )?;
    let audio_source = match &audio_input {
        AudioInput::LocalPath(p) => p.to_string_lossy().to_string(),
        AudioInput::Url(url) => url.clone(),
    };
    let extra = json!({
        "provider": vendor_name,
        "model": model,
        "model_kind": model_kind,
        "audio_path": audio_source,
        "outputs": [{"type":"text","preview": truncate(&text, 800)}],
        "latency_ms": 0
    });
    Ok((text, extra))
}

fn transcribe_by_vendor(
    client: &Client,
    audio_cfg: &AudioTranscribeConfig,
    cfg: &VendorConfig,
    vendor: VendorKind,
    allow_compat_adapters: bool,
    vendor_name: &str,
    model: &str,
    audio_input: &AudioInput,
    prompt: &str,
) -> Result<(String, &'static str), String> {
    let mode = resolve_adapter_mode(audio_cfg, vendor);
    match vendor {
        VendorKind::Google => {
            let audio_path = require_local_audio(audio_input)?;
            Ok((google_native_transcribe(client, cfg, model, audio_path, prompt)?, "native"))
        }
        VendorKind::OpenAI => {
            let audio_path = require_local_audio(audio_input)?;
            Ok((openai_compatible_transcribe(client, cfg, vendor_name, model, audio_path, prompt)?, "compat"))
        }
        VendorKind::Anthropic
        | VendorKind::Grok
        | VendorKind::DeepSeek
        | VendorKind::MiniMax
        | VendorKind::Custom => {
            if mode == AdapterMode::Native {
                return Err(format!("{vendor_name} native stt adapter is not available"));
            }
            if !allow_compat_adapters && mode != AdapterMode::Compat {
                return Err(format!(
                    "{vendor_name} native stt adapter is not available; set audio_transcribe.allow_compat_adapters=true to use compatible endpoint"
                ));
            }
            let audio_path = require_local_audio(audio_input)?;
            Ok((openai_compatible_transcribe(client, cfg, vendor_name, model, audio_path, prompt)?, "compat"))
        }
        VendorKind::Qwen => {
            if should_use_qwen_native_asr(audio_cfg, model, mode, allow_compat_adapters) {
                Ok((qwen_native_transcribe(
                    client,
                    audio_cfg,
                    audio_cfg.qwen_native_base_url.as_deref(),
                    &cfg.api_key,
                    model,
                    audio_input,
                    prompt,
                )?, "native"))
            } else {
                if !allow_compat_adapters {
                    return Err(
                        "qwen native stt adapter is not available; set audio_transcribe.allow_compat_adapters=true to use compatible endpoint"
                            .to_string(),
                    );
                }
                let audio_path = require_local_audio(audio_input)?;
                Ok((openai_compatible_transcribe(client, cfg, vendor_name, model, audio_path, prompt)?, "compat"))
            }
        }
    }
}

fn parse_audio_input(args: &Value, workspace_root: &Path) -> Result<AudioInput, String> {
    if let Some(obj) = args.as_object() {
        if let Some(url) = obj
            .get("audio")
            .and_then(|v| v.get("url"))
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("audio_url").and_then(|v| v.as_str()))
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Ok(AudioInput::Url(url.to_string()));
        }
        if let Some(path) = obj
            .get("audio")
            .and_then(|v| v.get("path"))
            .and_then(|v| v.as_str())
            .or_else(|| obj.get("audio_path").and_then(|v| v.as_str()))
            .or_else(|| obj.get("path").and_then(|v| v.as_str()))
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Ok(AudioInput::LocalPath(to_workspace_path(
                workspace_root,
                path,
            )?));
        }
    }
    if let Some(s) = args.as_str().map(str::trim).filter(|v| !v.is_empty()) {
        return Ok(AudioInput::LocalPath(to_workspace_path(workspace_root, s)?));
    }
    Err(
        "audio input is required (args.audio.path / args.path / args.audio.url / args.audio_url)"
            .to_string(),
    )
}

fn require_local_audio(audio_input: &AudioInput) -> Result<&Path, String> {
    match audio_input {
        AudioInput::LocalPath(p) => Ok(p.as_path()),
        AudioInput::Url(_) => Err("compatible adapter requires local audio file path".to_string()),
    }
}

fn resolve_adapter_mode(cfg: &AudioTranscribeConfig, vendor: VendorKind) -> AdapterMode {
    if matches!(vendor, VendorKind::OpenAI | VendorKind::Google) {
        return AdapterMode::Compat;
    }
    parse_adapter_mode(cfg.adapter_mode.as_deref())
}

fn parse_adapter_mode(raw: Option<&str>) -> AdapterMode {
    match raw
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

fn qwen_uses_native_asr_model(cfg: &AudioTranscribeConfig, model: &str) -> bool {
    let requested = model.trim();
    cfg.native_models
        .as_ref()
        .and_then(|list| {
            list.iter().map(|s| s.trim()).find(|candidate| {
                !candidate.is_empty() && candidate.eq_ignore_ascii_case(requested)
            })
        })
        .is_some()
}

fn should_use_qwen_native_asr(
    cfg: &AudioTranscribeConfig,
    model: &str,
    mode: AdapterMode,
    allow_compat: bool,
) -> bool {
    match mode {
        AdapterMode::Native => true,
        AdapterMode::Compat => false,
        AdapterMode::Auto => {
            if qwen_uses_native_asr_model(cfg, model) {
                true
            } else {
                !allow_compat
            }
        }
    }
}

fn qwen_native_transcribe(
    client: &Client,
    audio_cfg: &AudioTranscribeConfig,
    native_base_url: Option<&str>,
    api_key: &str,
    model: &str,
    audio_input: &AudioInput,
    _prompt: &str,
) -> Result<String, String> {
    let file_url = resolve_qwen_asr_file_url(client, audio_cfg, audio_input)?;
    if file_url.is_empty() {
        return Err("qwen native ASR requires non-empty args.audio.url".to_string());
    }
    let base = native_base_url
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("https://dashscope.aliyuncs.com/api/v1");
    let submit_url = format!(
        "{}/services/audio/asr/transcription",
        trim_trailing_slash(base)
    );
    let body = json!({
        "model": model,
        "input": {
            "file_urls": [file_url.as_str()]
        }
    });
    let submit_resp = client
        .post(submit_url)
        .bearer_auth(api_key)
        .header("X-DashScope-Async", "enable")
        .json(&body)
        .send()
        .map_err(|err| format!("qwen native asr submit request failed: {err}"))?;
    let submit_status = submit_resp.status().as_u16();
    let submit_json: Value = submit_resp
        .json()
        .map_err(|err| format!("parse qwen native asr submit response failed: {err}"))?;
    if submit_status >= 300 {
        return Err(format!(
            "qwen native asr submit failed status={submit_status}: {}",
            truncate(&submit_json.to_string(), 400)
        ));
    }
    if let Some(text) = extract_native_asr_text(client, &submit_json) {
        return Ok(text);
    }
    let task_id = submit_json
        .get("output")
        .and_then(|o| o.get("task_id"))
        .and_then(|v| v.as_str())
        .or_else(|| submit_json.get("task_id").and_then(|v| v.as_str()))
        .ok_or_else(|| {
            format!(
                "qwen native asr submit response missing task_id/text: {}",
                truncate(&submit_json.to_string(), 400)
            )
        })?;
    let task_url = format!("{}/tasks/{}", trim_trailing_slash(base), task_id);
    for _ in 0..90 {
        std::thread::sleep(Duration::from_secs(2));
        let poll_resp = client
            .post(&task_url)
            .bearer_auth(api_key)
            .send()
            .map_err(|err| format!("qwen native asr poll request failed: {err}"))?;
        let poll_status = poll_resp.status().as_u16();
        let poll_json: Value = poll_resp
            .json()
            .map_err(|err| format!("parse qwen native asr poll response failed: {err}"))?;
        if poll_status >= 300 {
            return Err(format!(
                "qwen native asr poll failed status={poll_status}: {}",
                truncate(&poll_json.to_string(), 400)
            ));
        }
        if let Some(task_status) = poll_json
            .get("output")
            .and_then(|o| o.get("task_status"))
            .and_then(|v| v.as_str())
            .map(|v| v.to_ascii_uppercase())
        {
            if task_status == "SUCCEEDED" {
                if let Some(text) = extract_native_asr_text(client, &poll_json) {
                    return Ok(text);
                }
                return Err(format!(
                    "qwen native asr task succeeded but no text found: {}",
                    truncate(&poll_json.to_string(), 400)
                ));
            }
            if task_status == "FAILED" || task_status == "CANCELED" {
                return Err(format!(
                    "qwen native asr task failed: {}",
                    truncate(&poll_json.to_string(), 400)
                ));
            }
        }
        if let Some(text) = extract_native_asr_text(client, &poll_json) {
            return Ok(text);
        }
    }
    Err(format!("qwen native asr timeout waiting task_id={task_id}"))
}

fn resolve_qwen_asr_file_url(
    client: &Client,
    cfg: &AudioTranscribeConfig,
    audio_input: &AudioInput,
) -> Result<String, String> {
    match audio_input {
        AudioInput::Url(url) => Ok(url.trim().to_string()),
        AudioInput::LocalPath(path) => {
            if !cfg.local_auto_upload_enabled {
                return Err(
                    "qwen native ASR requires args.audio.url (public URL), or enable audio_transcribe.local_auto_upload_enabled with OSS settings"
                        .to_string(),
                );
            }
            upload_local_audio_to_oss_and_sign_url(client, cfg, path)
        }
    }
}

fn upload_local_audio_to_oss_and_sign_url(
    client: &Client,
    cfg: &AudioTranscribeConfig,
    local_path: &Path,
) -> Result<String, String> {
    if !local_path.exists() || !local_path.is_file() {
        return Err("audio file does not exist".to_string());
    }
    let access_key_id = cfg
        .oss_access_key_id
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "audio_transcribe.oss_access_key_id is required".to_string())?;
    let access_key_secret = cfg
        .oss_access_key_secret
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "audio_transcribe.oss_access_key_secret is required".to_string())?;
    let bucket = cfg
        .oss_bucket
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| "audio_transcribe.oss_bucket is required".to_string())?;
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
        .unwrap_or("rustclaw/audio");
    let ttl_seconds = cfg.oss_url_ttl_seconds.unwrap_or(3600).clamp(60, 24 * 3600);

    let bytes = std::fs::read(local_path).map_err(|err| format!("read audio failed: {err}"))?;
    let content_type = guess_audio_mime(local_path);
    let file_name = local_path
        .file_name()
        .and_then(|s| s.to_str())
        .map(sanitize_oss_filename)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "audio.wav".to_string());
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
        .header("Content-Type", content_type)
        .header("Authorization", authorization)
        .body(bytes)
        .send()
        .map_err(|err| format!("upload audio to OSS failed: {err}"))?;
    let status = put_resp.status().as_u16();
    let body = put_resp.text().unwrap_or_default();
    if status >= 300 {
        return Err(format!(
            "upload audio to OSS failed status={status}: {}",
            truncate(&body, 400)
        ));
    }

    let expires = unix_ts() + ttl_seconds;
    let get_string_to_sign = format!("GET\n\n\n{}\n{}", expires, canonical_resource);
    let get_signature = hmac_sha1_base64(access_key_secret, &get_string_to_sign)?;
    let signed_url = format!(
        "{}?OSSAccessKeyId={}&Expires={}&Signature={}",
        put_url,
        urlencoding::encode(access_key_id),
        expires,
        urlencoding::encode(&get_signature)
    );
    Ok(signed_url)
}

fn hmac_sha1_base64(secret: &str, message: &str) -> Result<String, String> {
    type HmacSha1 = Hmac<Sha1>;
    let mut mac = HmacSha1::new_from_slice(secret.as_bytes())
        .map_err(|err| format!("invalid HMAC key: {err}"))?;
    mac.update(message.as_bytes());
    let result = mac.finalize().into_bytes();
    Ok(STANDARD.encode(result))
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
        "audio.wav".to_string()
    } else {
        out
    }
}

fn extract_native_asr_text(client: &Client, v: &Value) -> Option<String> {
    let direct = v
        .get("output")
        .and_then(|o| o.get("text"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ToString::to_string);
    if direct.is_some() {
        return direct;
    }
    let result_text = v
        .get("output")
        .and_then(|o| o.get("result"))
        .and_then(|r| r.get("text"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ToString::to_string);
    if result_text.is_some() {
        return result_text;
    }
    if let Some(text) = v
        .get("output")
        .and_then(|o| o.get("results"))
        .and_then(|r| r.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("transcription").or_else(|| item.get("text")))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ToString::to_string)
    {
        return Some(text);
    }
    let transcription_url = v
        .get("output")
        .and_then(|o| o.get("results"))
        .and_then(|r| r.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("transcription_url"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())?;
    let result_json: Value = client.get(transcription_url).send().ok()?.json().ok()?;
    let transcript_text = result_json
        .get("transcripts")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("text"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ToString::to_string);
    if transcript_text.is_some() {
        return transcript_text;
    }
    result_json
        .get("transcripts")
        .and_then(|t| t.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("sentences"))
        .and_then(|s| s.as_array())
        .and_then(|arr| arr.first())
        .and_then(|first| first.get("text"))
        .and_then(|t| t.as_str())
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(ToString::to_string)
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
    let url = format!(
        "{}/audio/transcriptions",
        trim_trailing_slash(&cfg.base_url)
    );
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

fn google_native_transcribe(
    client: &Client,
    cfg: &VendorConfig,
    model: &str,
    audio_path: &Path,
    prompt: &str,
) -> Result<String, String> {
    if !audio_path.exists() || !audio_path.is_file() {
        return Err("audio file does not exist".to_string());
    }
    let bytes = std::fs::read(audio_path).map_err(|err| format!("read audio failed: {err}"))?;
    let mime = guess_audio_mime(audio_path);
    let body = json!({
        "contents": [{
            "parts": [
                {"text": format!("Transcribe this audio verbatim. {}", prompt)},
                {"inline_data": {"mime_type": mime, "data": STANDARD.encode(bytes)}}
            ]
        }]
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
        .map_err(|err| format!("google transcription request failed: {err}"))?;
    let status = resp.status().as_u16();
    let v: Value = resp
        .json()
        .map_err(|err| format!("parse google transcription response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "google transcription failed status={status}: {}",
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
    let out = out.trim();
    if out.is_empty() {
        return Err(format!(
            "google transcription response missing text: {}",
            truncate(&v.to_string(), 400)
        ));
    }
    Ok(out.to_string())
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
        "custom" => Some(VendorKind::Custom),
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
    let section = &cfg.audio_transcribe.providers;
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
        VendorKind::Custom => section
            .custom
            .as_ref()
            .or(cfg.llm.custom.as_ref())
            .map(|v| ("custom", v))
            .ok_or_else(|| "custom config missing".to_string()),
    }
}

fn load_root_config() -> RootConfig {
    let root = workspace_root();
    let core_cfg = match std::fs::read_to_string(root.join("configs/config.toml"))
        .ok()
        .and_then(|s| toml::from_str::<TomlValue>(&s).ok())
    {
        Some(v) => v,
        None => TomlValue::Table(toml::map::Map::new()),
    };
    let audio_cfg = match std::fs::read_to_string(root.join("configs/audio.toml"))
        .ok()
        .and_then(|s| toml::from_str::<TomlValue>(&s).ok())
    {
        Some(v) => v,
        None => TomlValue::Table(toml::map::Map::new()),
    };
    let mut cfg = RootConfig::default();
    if let Some(v) = core_cfg.get("llm").cloned() {
        if let Ok(parsed) = v.try_into::<LlmConfig>() {
            cfg.llm = parsed;
        }
    }
    if let Some(v) = audio_cfg.get("audio_transcribe").cloned() {
        if let Ok(parsed) = v.try_into::<AudioTranscribeConfig>() {
            cfg.audio_transcribe = parsed;
        }
    }
    cfg
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

fn vendor_models<'a>(
    cfg: &'a AudioTranscribeConfig,
    vendor: VendorKind,
) -> Option<&'a Vec<String>> {
    match vendor {
        VendorKind::OpenAI => cfg.openai_models.as_ref(),
        VendorKind::Google => cfg.google_models.as_ref(),
        VendorKind::Anthropic => cfg.anthropic_models.as_ref(),
        VendorKind::Grok => cfg.grok_models.as_ref(),
        VendorKind::DeepSeek => cfg.deepseek_models.as_ref(),
        VendorKind::Qwen => cfg.qwen_models.as_ref(),
        VendorKind::MiniMax => cfg.minimax_models.as_ref(),
        VendorKind::Custom => cfg.custom_models.as_ref(),
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
        "custom" => "openai".to_string(),
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
        VendorKind::Custom => "openai",
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

fn load_prompt_template_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
    default_template: &str,
) -> String {
    let resolved_path = resolve_prompt_rel_path_for_vendor(workspace_root, vendor, rel_path);
    match std::fs::read_to_string(workspace_root.join(resolved_path)) {
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

fn guess_audio_mime(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" => "audio/ogg",
        "opus" => "audio/ogg",
        "flac" => "audio/flac",
        _ => "application/octet-stream",
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
        assert!(matches!(parse_vendor("openai"), Some(VendorKind::OpenAI)));
        assert!(matches!(parse_vendor("gemini"), Some(VendorKind::Google)));
        assert!(matches!(
            parse_vendor("claude"),
            Some(VendorKind::Anthropic)
        ));
        assert!(matches!(parse_vendor("xai"), Some(VendorKind::Grok)));
    }

    #[test]
    fn mime_guess_from_ext() {
        assert_eq!(guess_audio_mime(Path::new("a.wav")), "audio/wav");
        assert_eq!(guess_audio_mime(Path::new("a.mp3")), "audio/mpeg");
        assert_eq!(guess_audio_mime(Path::new("a.ogg")), "audio/ogg");
    }

    #[test]
    fn render_prompt_with_hint() {
        let got = render_transcribe_prompt("A __TRANSCRIBE_HINT__ B", "hint");
        assert_eq!(got, "A hint B");
    }

    #[test]
    fn select_vendor_keeps_default_minimax() {
        let got = select_vendor(None, Some("minimax"), Some("qwen"));
        assert_eq!(got, VendorKind::MiniMax);
    }

    #[test]
    fn select_vendor_keeps_explicit_minimax_request() {
        let got = select_vendor(Some("minimax"), Some("qwen"), Some("openai"));
        assert_eq!(got, VendorKind::MiniMax);
    }

    #[test]
    fn sanitize_oss_name_keeps_safe_chars() {
        assert_eq!(sanitize_oss_filename("a b/c?.wav"), "a_b_c_.wav");
    }
}
