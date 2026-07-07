use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::Value as TomlValue;

mod async_contract;
mod async_projection;

use async_contract::{
    execute_cancel, execute_poll, music_expires_at, music_poll_after_seconds, provider_music_job_id,
};
use async_projection::music_pending_async_job_contract;

const DEFAULT_MODEL: &str = "music-2.6";
const DEFAULT_FORMAT: &str = "mp3";
const SKILL_NAME: &str = "music_generate";

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
    music_generation: MusicGenerationConfig,
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
    #[serde(default)]
    custom: Option<VendorConfig>,
}

#[derive(Debug, Clone, Deserialize)]
struct VendorConfig {
    base_url: String,
    #[serde(default)]
    api_key: String,
    model: String,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    adapter_kind: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MusicGenerationConfig {
    #[serde(default)]
    default_vendor: Option<String>,
    #[serde(default)]
    default_output_dir: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    default_format: Option<String>,
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
    custom_models: Option<Vec<String>>,
    #[serde(default)]
    timeout_seconds: Option<u64>,
    #[serde(default)]
    max_prompt_chars: Option<usize>,
    #[serde(default)]
    max_lyrics_chars: Option<usize>,
    #[serde(default)]
    sample_rate: Option<u64>,
    #[serde(default)]
    bitrate: Option<u64>,
    #[serde(default)]
    providers: MusicProviderOverrides,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MusicProviderOverrides {
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
    Mimo,
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MusicAdapterKind {
    MiniMaxNative,
    Unsupported,
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

fn execute(
    cfg: &RootConfig,
    workspace_root: &Path,
    args: Value,
) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let action = obj
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("generate");
    match action {
        "generate" => execute_generate(cfg, workspace_root, obj),
        "poll" => execute_poll(cfg, workspace_root, obj),
        "cancel" => execute_cancel(cfg, obj),
        _ => Err(format!("unsupported action: {action}")),
    }
}

fn execute_generate(
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let requested_vendor = obj.get("vendor").and_then(Value::as_str);
    let vendor = select_vendor(
        requested_vendor,
        cfg.music_generation.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let provider_name = vendor_name(vendor);

    let prompt = obj
        .get("prompt")
        .or_else(|| obj.get("description"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    let lyrics = obj
        .get("lyrics")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    let is_instrumental = optional_bool(obj, "is_instrumental").unwrap_or(false);
    let lyrics_optimizer =
        optional_bool(obj, "lyrics_optimizer").unwrap_or(!is_instrumental && lyrics.is_empty());
    if prompt.is_empty() && (lyrics.is_empty() || is_instrumental || lyrics_optimizer) {
        return Err("prompt is required for this music request".to_string());
    }
    let max_prompt_chars = cfg.music_generation.max_prompt_chars.unwrap_or(2000);
    if prompt.chars().count() > max_prompt_chars {
        return Err(format!("prompt too long: max={max_prompt_chars} chars"));
    }
    let max_lyrics_chars = cfg.music_generation.max_lyrics_chars.unwrap_or(3500);
    if !lyrics.is_empty() && lyrics.chars().count() > max_lyrics_chars {
        return Err(format!("lyrics too long: max={max_lyrics_chars} chars"));
    }
    if !is_instrumental && lyrics.is_empty() && !lyrics_optimizer {
        return Err(
            "lyrics is required unless lyrics_optimizer or is_instrumental is true".to_string(),
        );
    }

    let provider_cfg = resolved_vendor_config(cfg, vendor);
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| cfg.music_generation.default_model.as_deref())
        .or_else(|| first_model(vendor_models(&cfg.music_generation, vendor)))
        .or_else(|| first_model(cfg.music_generation.models.as_ref()))
        .or_else(|| provider_cfg.as_ref().map(|config| config.model.as_str()))
        .unwrap_or(DEFAULT_MODEL)
        .to_string();
    let format = obj
        .get("format")
        .or_else(|| obj.get("response_format"))
        .and_then(Value::as_str)
        .or(cfg.music_generation.default_format.as_deref())
        .map(normalize_format)
        .unwrap_or_else(|| DEFAULT_FORMAT.to_string());
    let output_path = resolve_output_path(
        workspace_root,
        cfg.music_generation
            .default_output_dir
            .as_deref()
            .unwrap_or("music/download"),
        obj.get("output_path").and_then(Value::as_str),
        &format,
    )?;

    let mut payload = Map::new();
    payload.insert("model".to_string(), Value::String(model.clone()));
    if !prompt.is_empty() {
        payload.insert("prompt".to_string(), Value::String(prompt.to_string()));
    }
    if !lyrics.is_empty() {
        payload.insert("lyrics".to_string(), Value::String(lyrics.to_string()));
    }
    payload.insert("stream".to_string(), Value::Bool(false));
    payload.insert(
        "output_format".to_string(),
        Value::String("hex".to_string()),
    );
    payload.insert(
        "lyrics_optimizer".to_string(),
        Value::Bool(lyrics_optimizer),
    );
    payload.insert("is_instrumental".to_string(), Value::Bool(is_instrumental));
    if let Some(value) = string_arg(obj, "audio_url") {
        payload.insert("audio_url".to_string(), Value::String(value));
    }
    if let Some(value) = string_arg(obj, "audio_base64") {
        payload.insert("audio_base64".to_string(), Value::String(value));
    }
    if let Some(value) = string_arg(obj, "cover_feature_id") {
        payload.insert("cover_feature_id".to_string(), Value::String(value));
    }
    payload.insert(
        "audio_setting".to_string(),
        json!({
            "sample_rate": cfg.music_generation.sample_rate.unwrap_or(44100),
            "bitrate": cfg.music_generation.bitrate.unwrap_or(256000),
            "format": format,
        }),
    );
    let payload = Value::Object(payload);

    if optional_bool(obj, "dry_run").unwrap_or(false) {
        let output = output_path.to_string_lossy().to_string();
        let poll_after_seconds = music_poll_after_seconds(obj);
        let expires_at = music_expires_at(obj);
        let dry_run_job_id = provider_music_job_id(provider_name, "dry_run");
        return Ok((
            "MUSIC_GENERATE_DRY_RUN".to_string(),
            json!({
                "provider": provider_name,
                "model": model,
                "model_kind": adapter_kind_name(adapter_kind_for(vendor, provider_cfg.as_ref())),
                "adapter_kind": "media_job_poll",
                "dry_run": true,
                "request": payload,
                "output_path": output,
                "outputs": [],
                "planned_outputs": [{"type":"audio_file","path": output}],
                "pending_async_job_contract": music_pending_async_job_contract(
                    provider_name,
                    &model,
                    &dry_run_job_id,
                    "dry_run",
                    &output,
                    poll_after_seconds,
                    expires_at,
                ),
            }),
        ));
    }

    let provider = provider_cfg
        .as_ref()
        .ok_or_else(|| format!("{provider_name} config missing"))?;
    let adapter_kind = adapter_kind_for(vendor, Some(provider));
    if !matches!(adapter_kind, MusicAdapterKind::MiniMaxNative) {
        return Err(format!(
            "{provider_name} music adapter is not available; configure adapter_kind=minimax_compatible only for MiniMax-compatible endpoints"
        ));
    }
    check_api_key(provider_name, &provider.api_key)?;
    let timeout_seconds = provider
        .timeout_seconds
        .or(cfg.music_generation.timeout_seconds)
        .unwrap_or(300)
        .clamp(5, 900);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| format!("build {provider_name} client failed: {err}"))?;
    let response = call_music_generation(&client, provider, &payload)?;
    write_music_output(&client, &response, &output_path)?;
    let output = output_path.to_string_lossy().to_string();
    Ok((
        format!("MUSIC_FILE:{output}"),
        json!({
            "provider": provider_name,
            "model": model,
            "model_kind": adapter_kind_name(adapter_kind),
            "output_path": output,
            "outputs": [{"type":"audio_file","path": output}],
            "audio_format": format,
            "trace_id": response.get("trace_id").cloned().unwrap_or(Value::Null),
            "extra_info": response.get("extra_info").cloned().unwrap_or(Value::Null),
            "latency_ms": 0,
        }),
    ))
}

fn call_music_generation(
    client: &Client,
    cfg: &VendorConfig,
    payload: &Value,
) -> Result<Value, String> {
    let url = format!("{}/music_generation", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(payload)
        .send()
        .map_err(|err| format!("minimax music request failed: {err}"))?;
    let status = resp.status().as_u16();
    let value: Value = resp
        .json()
        .map_err(|err| format!("parse minimax music response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "minimax music failed status={status}: {}",
            truncate(&value.to_string(), 400)
        ));
    }
    check_base_resp(&value, "minimax music")?;
    Ok(value)
}

fn write_music_output(client: &Client, response: &Value, output_path: &Path) -> Result<(), String> {
    let audio = response
        .get("data")
        .and_then(|data| data.get("audio"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            format!(
                "minimax music response missing data.audio: {}",
                truncate(&response.to_string(), 400)
            )
        })?;
    let bytes = if audio.starts_with("http://") || audio.starts_with("https://") {
        let resp = client
            .get(audio)
            .send()
            .map_err(|err| format!("download music failed: {err}"))?;
        let status = resp.status().as_u16();
        let bytes = resp
            .bytes()
            .map_err(|err| format!("read music download failed: {err}"))?;
        if status >= 300 {
            return Err(format!(
                "download music failed status={status}: {}",
                truncate(&String::from_utf8_lossy(&bytes), 400)
            ));
        }
        bytes.to_vec()
    } else {
        hex::decode(audio).map_err(|err| format!("decode minimax music audio failed: {err}"))?
    };
    ensure_parent_dir(output_path)?;
    std::fs::write(output_path, bytes).map_err(|err| format!("write music output failed: {err}"))
}

fn check_base_resp(value: &Value, label: &str) -> Result<(), String> {
    if let Some(code) = value
        .get("base_resp")
        .and_then(|base| base.get("status_code"))
        .and_then(Value::as_i64)
    {
        if code != 0 {
            let msg = value
                .get("base_resp")
                .and_then(|base| base.get("status_msg"))
                .and_then(Value::as_str)
                .unwrap_or("unknown provider error");
            return Err(format!("{label} failed code={code}: {msg}"));
        }
    }
    Ok(())
}

fn normalize_format(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "wav" => "wav".to_string(),
        "flac" => "flac".to_string(),
        "mp3" => "mp3".to_string(),
        _ => DEFAULT_FORMAT.to_string(),
    }
}

fn resolve_output_path(
    workspace_root: &Path,
    default_dir: &str,
    requested: Option<&str>,
    format: &str,
) -> Result<PathBuf, String> {
    if let Some(path) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        let out = normalize_workspace_path(workspace_root, path)?;
        return Ok(out);
    }
    Ok(workspace_root.join(default_dir).join(format!(
        "music-{}.{}",
        unix_ts(),
        normalize_format(format)
    )))
}

fn normalize_workspace_path(workspace_root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(raw_path);
    let out = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    if !out.starts_with(workspace_root) {
        return Err("output_path is outside workspace".to_string());
    }
    Ok(out)
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "output path has no parent directory".to_string())?;
    std::fs::create_dir_all(parent).map_err(|err| format!("create output dir failed: {err}"))
}

fn load_root_config() -> RootConfig {
    let root = workspace_root();
    let core_cfg = read_toml(root.join("configs/config.toml"));
    let music_cfg = read_toml(root.join("configs/music.toml"));
    let mut cfg = RootConfig::default();
    if let Some(value) = core_cfg.get("llm").cloned() {
        if let Ok(parsed) = value.try_into::<LlmConfig>() {
            cfg.llm = parsed;
        }
    }
    if let Some(value) = music_cfg.get("music_generation").cloned() {
        if let Ok(parsed) = value.try_into::<MusicGenerationConfig>() {
            cfg.music_generation = parsed;
        }
    }
    apply_env_overrides(&mut cfg);
    cfg
}

fn read_toml(path: PathBuf) -> TomlValue {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| toml::from_str::<TomlValue>(&raw).ok())
        .unwrap_or_else(|| TomlValue::Table(toml::map::Map::new()))
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
    apply_vendor_api_key_env(&mut cfg.llm.custom, "CUSTOM_API_KEY");

    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.openai,
        "MUSIC_GENERATION_OPENAI_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.google,
        "MUSIC_GENERATION_GOOGLE_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.anthropic,
        "MUSIC_GENERATION_ANTHROPIC_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.grok,
        "MUSIC_GENERATION_GROK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.deepseek,
        "MUSIC_GENERATION_DEEPSEEK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.qwen,
        "MUSIC_GENERATION_QWEN_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.minimax,
        "MUSIC_GENERATION_MINIMAX_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.mimo,
        "MUSIC_GENERATION_MIMO_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.music_generation.providers.custom,
        "MUSIC_GENERATION_CUSTOM_API_KEY",
    );
}

fn resolved_vendor_config(cfg: &RootConfig, vendor: VendorKind) -> Option<VendorConfig> {
    let dedicated = match vendor {
        VendorKind::OpenAI => cfg.music_generation.providers.openai.clone(),
        VendorKind::Google => cfg.music_generation.providers.google.clone(),
        VendorKind::Anthropic => cfg.music_generation.providers.anthropic.clone(),
        VendorKind::Grok => cfg.music_generation.providers.grok.clone(),
        VendorKind::DeepSeek => cfg.music_generation.providers.deepseek.clone(),
        VendorKind::Qwen => cfg.music_generation.providers.qwen.clone(),
        VendorKind::MiniMax => cfg.music_generation.providers.minimax.clone(),
        VendorKind::Mimo => cfg.music_generation.providers.mimo.clone(),
        VendorKind::Custom => cfg.music_generation.providers.custom.clone(),
    };
    let shared = match vendor {
        VendorKind::OpenAI => cfg.llm.openai.clone(),
        VendorKind::Google => cfg.llm.google.clone(),
        VendorKind::Anthropic => cfg.llm.anthropic.clone(),
        VendorKind::Grok => cfg.llm.grok.clone(),
        VendorKind::DeepSeek => cfg.llm.deepseek.clone(),
        VendorKind::Qwen => cfg.llm.qwen.clone(),
        VendorKind::MiniMax => cfg.llm.minimax.clone(),
        VendorKind::Mimo => cfg.llm.mimo.clone(),
        VendorKind::Custom => cfg.llm.custom.clone(),
    };
    match (dedicated, shared) {
        (Some(mut dedicated), Some(shared)) => {
            fill_empty_provider_fields(&mut dedicated, &shared);
            Some(dedicated)
        }
        (Some(dedicated), None) => Some(dedicated),
        (None, Some(shared)) => Some(shared),
        (None, None) => None,
    }
}

fn fill_empty_provider_fields(target: &mut VendorConfig, fallback: &VendorConfig) {
    if target.base_url.trim().is_empty() {
        target.base_url = fallback.base_url.clone();
    }
    if target.api_key.trim().is_empty() {
        target.api_key = fallback.api_key.clone();
    }
    if target.model.trim().is_empty() {
        target.model = fallback.model.clone();
    }
    if target.timeout_seconds.is_none() {
        target.timeout_seconds = fallback.timeout_seconds;
    }
    if target.adapter_kind.is_none() {
        target.adapter_kind = fallback.adapter_kind.clone();
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
        .unwrap_or(VendorKind::MiniMax)
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
        "custom" => Some(VendorKind::Custom),
        _ => None,
    }
}

fn vendor_name(vendor: VendorKind) -> &'static str {
    match vendor {
        VendorKind::OpenAI => "openai",
        VendorKind::Google => "google",
        VendorKind::Anthropic => "anthropic",
        VendorKind::Grok => "grok",
        VendorKind::DeepSeek => "deepseek",
        VendorKind::Qwen => "qwen",
        VendorKind::MiniMax => "minimax",
        VendorKind::Mimo => "mimo",
        VendorKind::Custom => "custom",
    }
}

fn vendor_models(cfg: &MusicGenerationConfig, vendor: VendorKind) -> Option<&Vec<String>> {
    match vendor {
        VendorKind::OpenAI => cfg.openai_models.as_ref(),
        VendorKind::Google => cfg.google_models.as_ref(),
        VendorKind::Anthropic => cfg.anthropic_models.as_ref(),
        VendorKind::Grok => cfg.grok_models.as_ref(),
        VendorKind::DeepSeek => cfg.deepseek_models.as_ref(),
        VendorKind::Qwen => cfg.qwen_models.as_ref(),
        VendorKind::MiniMax => cfg.minimax_models.as_ref(),
        VendorKind::Mimo => cfg.mimo_models.as_ref(),
        VendorKind::Custom => cfg.custom_models.as_ref(),
    }
}

fn adapter_kind_for(vendor: VendorKind, cfg: Option<&VendorConfig>) -> MusicAdapterKind {
    if matches!(vendor, VendorKind::MiniMax) {
        return MusicAdapterKind::MiniMaxNative;
    }
    match cfg
        .and_then(|cfg| cfg.adapter_kind.as_deref())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("minimax") | Some("minimax_native") | Some("minimax_compatible") => {
            MusicAdapterKind::MiniMaxNative
        }
        _ => MusicAdapterKind::Unsupported,
    }
}

fn adapter_kind_name(kind: MusicAdapterKind) -> &'static str {
    match kind {
        MusicAdapterKind::MiniMaxNative => "minimax_native",
        MusicAdapterKind::Unsupported => "unsupported",
    }
}

fn first_model(models: Option<&Vec<String>>) -> Option<&str> {
    models?
        .iter()
        .map(|value| value.trim())
        .find(|value| !value.is_empty())
}

fn string_arg(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn optional_bool(obj: &Map<String, Value>, key: &str) -> Option<bool> {
    obj.get(key).and_then(Value::as_bool)
}

fn check_api_key(vendor: &str, key: &str) -> Result<(), String> {
    let t = key.trim();
    if t.is_empty() || t.starts_with("REPLACE_ME_") {
        return Err(format!("{vendor} api key is not configured"));
    }
    Ok(())
}

fn trim_trailing_slash(value: &str) -> String {
    value.trim_end_matches('/').to_string()
}

fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect::<String>() + "..."
}

fn unix_ts() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn workspace_root() -> PathBuf {
    std::env::var("WORKSPACE_ROOT")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
