use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use toml::Value as TomlValue;

const DEFAULT_MODEL: &str = "MiniMax-Hailuo-2.3";
const DEFAULT_RESOLUTION: &str = "768P";
const SKILL_NAME: &str = "video_generate";

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
    video_generation: VideoGenerationConfig,
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
struct VideoGenerationConfig {
    #[serde(default)]
    default_vendor: Option<String>,
    #[serde(default)]
    default_output_dir: Option<String>,
    #[serde(default)]
    default_model: Option<String>,
    #[serde(default)]
    default_resolution: Option<String>,
    #[serde(default)]
    default_duration: Option<u64>,
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
    max_poll_seconds: Option<u64>,
    #[serde(default)]
    poll_interval_ms: Option<u64>,
    #[serde(default)]
    max_input_bytes: Option<u64>,
    #[serde(default)]
    download_on_success: Option<bool>,
    #[serde(default)]
    providers: VideoProviderOverrides,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct VideoProviderOverrides {
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
enum VideoAdapterKind {
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
        "generate" => execute_generate(cfg, workspace_root, args),
        "preview_generate" => {
            let mut preview_args = obj.clone();
            preview_args.insert("dry_run".to_string(), Value::Bool(true));
            execute_generate(cfg, workspace_root, Value::Object(preview_args))
        }
        "poll" => execute_poll(cfg, workspace_root, obj),
        "cancel" => execute_cancel(cfg, obj),
        _ => Err(format!("unsupported action: {action}")),
    }
}

fn execute_generate(
    cfg: &RootConfig,
    workspace_root: &Path,
    args: Value,
) -> Result<(String, Value), String> {
    let obj = args
        .as_object()
        .ok_or_else(|| "args must be object".to_string())?;
    let requested_vendor = obj.get("vendor").and_then(Value::as_str);
    let vendor = select_vendor(
        requested_vendor,
        cfg.video_generation.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let provider_name = vendor_name(vendor);

    let prompt = obj
        .get("prompt")
        .or_else(|| obj.get("description"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "prompt is required".to_string())?;
    if prompt.chars().count() > 2000 {
        return Err("prompt too long: max=2000 chars".to_string());
    }

    let provider_cfg = resolved_vendor_config(cfg, vendor);
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| cfg.video_generation.default_model.as_deref())
        .or_else(|| first_model(vendor_models(&cfg.video_generation, vendor)))
        .or_else(|| first_model(cfg.video_generation.models.as_ref()))
        .or_else(|| provider_cfg.as_ref().map(|config| config.model.as_str()))
        .unwrap_or(DEFAULT_MODEL)
        .to_string();
    let duration = obj
        .get("duration")
        .and_then(Value::as_u64)
        .or(cfg.video_generation.default_duration)
        .unwrap_or(6);
    if !matches!(duration, 6 | 10) {
        return Err("duration must be 6 or 10 seconds".to_string());
    }
    let resolution_source = obj
        .get("resolution")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            cfg.video_generation
                .default_resolution
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .unwrap_or(DEFAULT_RESOLUTION);
    let resolution = normalize_resolution(resolution_source)
        .ok_or_else(|| "resolution must be 512P, 720P, 768P, or 1080P".to_string())?;
    if !matches!(resolution.as_str(), "512P" | "720P" | "768P" | "1080P") {
        return Err("resolution must be 512P, 720P, 768P, or 1080P".to_string());
    }

    let output_path = resolve_output_path(
        workspace_root,
        cfg.video_generation
            .default_output_dir
            .as_deref()
            .unwrap_or("video/download"),
        obj.get("output_path").and_then(Value::as_str),
    )?;
    let max_input_bytes = cfg
        .video_generation
        .max_input_bytes
        .unwrap_or(20 * 1024 * 1024);
    let first_frame = image_arg_to_api_value(
        workspace_root,
        obj.get("first_frame_image")
            .or_else(|| obj.get("first_frame"))
            .or_else(|| obj.get("image")),
        max_input_bytes,
    )?;
    let last_frame = image_arg_to_api_value(
        workspace_root,
        obj.get("last_frame_image")
            .or_else(|| obj.get("last_frame")),
        max_input_bytes,
    )?;
    let mut payload = Map::new();
    payload.insert("model".to_string(), Value::String(model.clone()));
    payload.insert("prompt".to_string(), Value::String(prompt.to_string()));
    payload.insert("duration".to_string(), Value::from(duration));
    payload.insert("resolution".to_string(), Value::String(resolution.clone()));
    if let Some(value) = optional_bool(obj, "prompt_optimizer") {
        payload.insert("prompt_optimizer".to_string(), Value::Bool(value));
    }
    if let Some(value) = optional_bool(obj, "fast_pretreatment") {
        payload.insert("fast_pretreatment".to_string(), Value::Bool(value));
    }
    if let Some(value) = string_arg(obj, "callback_url") {
        payload.insert("callback_url".to_string(), Value::String(value));
    }
    if let Some(value) = first_frame {
        payload.insert("first_frame_image".to_string(), Value::String(value));
    }
    if let Some(value) = last_frame {
        payload.insert("last_frame_image".to_string(), Value::String(value));
    }
    let payload = Value::Object(payload);

    if optional_bool(obj, "dry_run").unwrap_or(false) {
        let output = output_path.to_string_lossy().to_string();
        let poll_interval_ms = cfg_poll_interval_ms(&cfg.video_generation);
        let poll_after_seconds = poll_after_seconds_from_interval_ms(poll_interval_ms);
        let max_poll_seconds = obj
            .get("max_poll_seconds")
            .and_then(Value::as_u64)
            .or(cfg.video_generation.max_poll_seconds)
            .unwrap_or(600)
            .clamp(5, 900);
        let expires_at = (unix_ts() as i64).saturating_add(max_poll_seconds as i64);
        let dry_run_job_id = provider_video_job_id(provider_name, "dry_run");
        return Ok((
            "VIDEO_GENERATE_DRY_RUN".to_string(),
            json!({
                "provider": provider_name,
                "model": model,
                "model_kind": adapter_kind_name(adapter_kind_for(vendor, provider_cfg.as_ref())),
                "adapter_kind": "media_job_poll",
                "dry_run": true,
                "duration": duration,
                "resolution": resolution,
                "request": payload,
                "output_path": output,
                "outputs": [],
                "planned_outputs": [{"type":"video_file","path": output}],
                "pending_async_job_contract": {
                    "job_id": dry_run_job_id,
                    "provider": provider_name,
                    "status": "accepted",
                    "poll_after_seconds": poll_after_seconds,
                    "poll_after_ms": poll_after_seconds.saturating_mul(1_000),
                    "expires_at": expires_at,
                    "cancel_ref": dry_run_job_id,
                    "cancel_token": dry_run_job_id,
                    "result_ref": dry_run_job_id,
                    "message_key": "clawd.task.async_job_pending",
                    "retryable": true,
                    "poll_adapter": {
                        "kind": "media_job_poll",
                        "skill_name": "video_generate",
                        "args": {
                            "action": "poll",
                            "task_id": "dry_run",
                            "job_id": dry_run_job_id,
                            "vendor": provider_name,
                            "model": model,
                            "download": true,
                            "output_path": output,
                            "poll_after_seconds": poll_after_seconds,
                            "expires_at": expires_at,
                            "dry_run": true
                        }
                    }
                },
            }),
        ));
    }

    let provider = provider_cfg
        .as_ref()
        .ok_or_else(|| format!("{provider_name} config missing"))?;
    let adapter_kind = adapter_kind_for(vendor, Some(provider));
    if !matches!(adapter_kind, VideoAdapterKind::MiniMaxNative) {
        return Err(format!(
            "{provider_name} video adapter is not available; configure adapter_kind=minimax_compatible only for MiniMax-compatible endpoints"
        ));
    }
    check_api_key(provider_name, &provider.api_key)?;
    let timeout_seconds = provider
        .timeout_seconds
        .or(cfg.video_generation.timeout_seconds)
        .unwrap_or(120)
        .clamp(5, 900);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| format!("build {provider_name} client failed: {err}"))?;
    let poll_interval_ms = cfg_poll_interval_ms(&cfg.video_generation);
    let poll_after_seconds = poll_after_seconds_from_interval_ms(poll_interval_ms);
    let max_poll_seconds = obj
        .get("max_poll_seconds")
        .and_then(Value::as_u64)
        .or(cfg.video_generation.max_poll_seconds)
        .unwrap_or(600)
        .clamp(5, 900);
    let should_download = optional_bool(obj, "download")
        .or(cfg.video_generation.download_on_success)
        .unwrap_or(true);
    let output_path_string = output_path.to_string_lossy().to_string();
    let task_id = create_video_task(&client, provider, &payload)?;
    let wait_for_completion = wait_for_completion_arg(obj);
    if !wait_for_completion {
        return Ok(video_pending_task_response(
            &task_id,
            provider_name,
            &model,
            adapter_kind,
            poll_after_seconds,
            max_poll_seconds,
            should_download,
            &output_path_string,
        ));
    }
    let query = poll_video_task(
        &client,
        provider,
        &task_id,
        poll_interval_ms,
        max_poll_seconds,
    )?;
    let status = query
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    if status == "Fail" {
        return Err(format!(
            "minimax video task failed: {}",
            truncate(&query.to_string(), 400)
        ));
    }
    if status != "Success" {
        return Ok(video_task_response(
            &status,
            &task_id,
            provider_name,
            &model,
            adapter_kind,
            None,
            query.get("file_id").cloned(),
            None,
        ));
    }

    let file_id = query
        .get("file_id")
        .and_then(value_to_string)
        .ok_or_else(|| "minimax video success response missing file_id".to_string())?;
    if !should_download {
        return Ok(video_task_response(
            &status,
            &task_id,
            provider_name,
            &model,
            adapter_kind,
            None,
            Some(Value::String(file_id)),
            Some(query),
        ));
    }
    let download_url = retrieve_file_url(&client, provider, &file_id)?;
    download_to_path(&client, &download_url, &output_path)?;
    let output = output_path.to_string_lossy().to_string();
    Ok((
        format!("VIDEO_FILE:{output}"),
        json!({
            "provider": provider_name,
            "model": model,
            "model_kind": adapter_kind_name(adapter_kind),
            "task_id": task_id,
            "status": status,
            "file_id": file_id,
            "download_url": download_url,
            "output_path": output,
            "outputs": [{"type":"video_file","path": output}],
            "video_width": query.get("video_width").cloned().unwrap_or(Value::Null),
            "video_height": query.get("video_height").cloned().unwrap_or(Value::Null),
            "latency_ms": 0,
        }),
    ))
}

fn cfg_poll_interval_ms(cfg: &VideoGenerationConfig) -> u64 {
    cfg.poll_interval_ms.unwrap_or(5_000).clamp(500, 60_000)
}

fn wait_for_completion_arg(obj: &Map<String, Value>) -> bool {
    optional_bool(obj, "wait_for_completion").unwrap_or(false)
}

fn poll_after_seconds_from_interval_ms(poll_interval_ms: u64) -> u64 {
    poll_interval_ms.div_ceil(1000).max(1)
}

fn normalize_resolution(value: &str) -> Option<String> {
    let upper = value.trim().to_ascii_uppercase();
    if matches!(upper.as_str(), "512P" | "720P" | "768P" | "1080P") {
        return Some(upper);
    }

    let compact: String = upper
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && *ch != '_')
        .collect();
    if let Ok(number) = compact.parse::<u64>() {
        return canonical_resolution_token(number);
    }

    let dimensions = compact
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .take(2)
        .filter_map(|part| part.parse::<u64>().ok())
        .collect::<Vec<_>>();
    if dimensions.len() == 2 {
        return canonical_resolution_token(dimensions[0].min(dimensions[1]));
    }

    None
}

fn canonical_resolution_token(short_edge: u64) -> Option<String> {
    match short_edge {
        512 | 720 | 768 | 1080 => Some(format!("{short_edge}P")),
        _ => None,
    }
}

fn provider_video_job_id(provider: &str, task_id: &str) -> String {
    format!("provider:video_generate:{provider}:{task_id}")
}

fn video_task_response(
    status: &str,
    task_id: &str,
    provider: &str,
    model: &str,
    model_kind: VideoAdapterKind,
    output_path: Option<String>,
    file_id: Option<Value>,
    query: Option<Value>,
) -> (String, Value) {
    (
        format!("VIDEO_TASK:{task_id}"),
        json!({
            "provider": provider,
            "model": model,
            "model_kind": adapter_kind_name(model_kind),
            "task_id": task_id,
            "status": status,
            "file_id": file_id.unwrap_or(Value::Null),
            "output_path": output_path,
            "outputs": [],
            "query": query.unwrap_or(Value::Null),
        }),
    )
}

fn video_pending_task_response(
    task_id: &str,
    provider: &str,
    model: &str,
    model_kind: VideoAdapterKind,
    poll_after_seconds: u64,
    max_poll_seconds: u64,
    download: bool,
    output_path: &str,
) -> (String, Value) {
    let job_id = provider_video_job_id(provider, task_id);
    let expires_at = (unix_ts() as i64).saturating_add(max_poll_seconds as i64);
    (
        format!("VIDEO_TASK:{task_id}"),
        json!({
            "provider": provider,
            "model": model,
            "model_kind": adapter_kind_name(model_kind),
            "task_id": task_id,
            "status": "submitted",
            "file_id": Value::Null,
            "output_path": output_path,
            "outputs": [],
            "pending_async_job": {
                "job_id": job_id,
                "provider": provider,
                "status": "accepted",
                "poll_after_seconds": poll_after_seconds,
                "poll_after_ms": poll_after_seconds.saturating_mul(1_000),
                "expires_at": expires_at,
                "cancel_ref": job_id,
                "cancel_token": job_id,
                "result_ref": job_id,
                "message_key": "clawd.task.async_job_pending",
                "retryable": true,
                "poll_adapter": {
                    "kind": "media_job_poll",
                    "skill_name": "video_generate",
                    "args": {
                        "action": "poll",
                        "task_id": task_id,
                        "job_id": job_id,
                        "vendor": provider,
                        "model": model,
                        "download": download,
                        "output_path": output_path,
                        "poll_after_seconds": poll_after_seconds,
                        "expires_at": expires_at
                    }
                }
            },
        }),
    )
}

fn execute_poll(
    cfg: &RootConfig,
    workspace_root: &Path,
    obj: &Map<String, Value>,
) -> Result<(String, Value), String> {
    let requested_vendor = obj.get("vendor").and_then(Value::as_str);
    let vendor = select_vendor(
        requested_vendor,
        cfg.video_generation.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let provider_name = vendor_name(vendor);
    let provider_cfg = resolved_vendor_config(cfg, vendor);
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| cfg.video_generation.default_model.as_deref())
        .or_else(|| first_model(vendor_models(&cfg.video_generation, vendor)))
        .or_else(|| first_model(cfg.video_generation.models.as_ref()))
        .or_else(|| provider_cfg.as_ref().map(|config| config.model.as_str()))
        .unwrap_or(DEFAULT_MODEL)
        .to_string();
    let adapter_kind = adapter_kind_for(vendor, provider_cfg.as_ref());
    let task_id = obj
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "task_id is required".to_string())?;
    let job_id = obj
        .get("job_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| provider_video_job_id(provider_name, task_id));
    let poll_after_seconds = obj
        .get("poll_after_seconds")
        .and_then(Value::as_u64)
        .or_else(|| {
            obj.get("poll_after_ms")
                .and_then(Value::as_u64)
                .filter(|millis| *millis > 0)
                .map(|millis| millis.saturating_add(999) / 1_000)
        })
        .unwrap_or_else(|| {
            poll_after_seconds_from_interval_ms(cfg_poll_interval_ms(&cfg.video_generation))
        })
        .clamp(1, 3600);
    let expires_at = obj
        .get("expires_at")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| {
            (unix_ts() as i64).saturating_add(
                cfg.video_generation
                    .max_poll_seconds
                    .unwrap_or(600)
                    .clamp(5, 900) as i64,
            )
        });
    if expires_at <= unix_ts() as i64 {
        return Ok(video_poll_response(
            task_id,
            &job_id,
            provider_name,
            &model,
            adapter_kind,
            poll_after_seconds,
            expires_at,
            video_poll_adapter_result(
                &job_id,
                "expired",
                poll_after_seconds,
                expires_at,
                None,
                Some("async_poll_expired"),
                Some("clawd.task.async_poll_expired"),
            ),
            json!({"status": "expired"}),
        ));
    }
    if optional_bool(obj, "dry_run").unwrap_or(false) {
        let status = obj
            .get("mock_status")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("running");
        let query = json!({
            "status": status,
            "file_id": obj.get("mock_file_id").cloned().unwrap_or(Value::Null),
        });
        let adapter_result = adapter_result_from_video_query(
            workspace_root,
            cfg.video_generation
                .default_output_dir
                .as_deref()
                .unwrap_or("video/download"),
            obj,
            task_id,
            &job_id,
            provider_name,
            &model,
            adapter_kind,
            poll_after_seconds,
            expires_at,
            query.clone(),
            true,
            optional_bool(obj, "download").unwrap_or(true),
            None,
        )?;
        return Ok(video_poll_response(
            task_id,
            &job_id,
            provider_name,
            &model,
            adapter_kind,
            poll_after_seconds,
            expires_at,
            adapter_result,
            query,
        ));
    }

    let provider = provider_cfg
        .as_ref()
        .ok_or_else(|| format!("{provider_name} config missing"))?;
    if !matches!(adapter_kind, VideoAdapterKind::MiniMaxNative) {
        return Err(format!(
            "{provider_name} video adapter is not available; configure adapter_kind=minimax_compatible only for MiniMax-compatible endpoints"
        ));
    }
    check_api_key(provider_name, &provider.api_key)?;
    let timeout_seconds = provider
        .timeout_seconds
        .or(cfg.video_generation.timeout_seconds)
        .unwrap_or(120)
        .clamp(5, 900);
    let client = Client::builder()
        .timeout(Duration::from_secs(timeout_seconds))
        .build()
        .map_err(|err| format!("build {provider_name} client failed: {err}"))?;
    let query = query_video_task(&client, provider, task_id)?;
    let should_download = optional_bool(obj, "download")
        .or(cfg.video_generation.download_on_success)
        .unwrap_or(true);
    let download_url = if query
        .get("status")
        .and_then(Value::as_str)
        .is_some_and(|status| status == "Success")
        && should_download
    {
        let file_id = query
            .get("file_id")
            .and_then(value_to_string)
            .ok_or_else(|| "minimax video success response missing file_id".to_string())?;
        let output_path = resolve_output_path(
            workspace_root,
            cfg.video_generation
                .default_output_dir
                .as_deref()
                .unwrap_or("video/download"),
            obj.get("output_path").and_then(Value::as_str),
        )?;
        let url = retrieve_file_url(&client, provider, &file_id)?;
        download_to_path(&client, &url, &output_path)?;
        Some(url)
    } else {
        None
    };
    let adapter_result = adapter_result_from_video_query(
        workspace_root,
        cfg.video_generation
            .default_output_dir
            .as_deref()
            .unwrap_or("video/download"),
        obj,
        task_id,
        &job_id,
        provider_name,
        &model,
        adapter_kind,
        poll_after_seconds,
        expires_at,
        query.clone(),
        false,
        should_download,
        download_url,
    )?;
    Ok(video_poll_response(
        task_id,
        &job_id,
        provider_name,
        &model,
        adapter_kind,
        poll_after_seconds,
        expires_at,
        adapter_result,
        query,
    ))
}

fn execute_cancel(cfg: &RootConfig, obj: &Map<String, Value>) -> Result<(String, Value), String> {
    let requested_vendor = obj.get("vendor").and_then(Value::as_str);
    let vendor = select_vendor(
        requested_vendor,
        cfg.video_generation.default_vendor.as_deref(),
        cfg.llm.selected_vendor.as_deref(),
    );
    let provider_name = vendor_name(vendor);
    let provider_cfg = resolved_vendor_config(cfg, vendor);
    let model = obj
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| cfg.video_generation.default_model.as_deref())
        .or_else(|| first_model(vendor_models(&cfg.video_generation, vendor)))
        .or_else(|| first_model(cfg.video_generation.models.as_ref()))
        .or_else(|| provider_cfg.as_ref().map(|config| config.model.as_str()))
        .unwrap_or(DEFAULT_MODEL)
        .to_string();
    let adapter_kind = adapter_kind_for(vendor, provider_cfg.as_ref());
    let task_id = obj
        .get("task_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "task_id is required".to_string())?;
    let job_id = obj
        .get("job_id")
        .or_else(|| obj.get("cancel_token"))
        .or_else(|| obj.get("cancel_ref"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| provider_video_job_id(provider_name, task_id));
    let cancelled_at = unix_ts() as i64;
    let provider_cancel_contract = json!({
        "provider": provider_name,
        "skill_name": "video_generate",
        "task_id": task_id,
        "job_id": job_id,
        "cancel_ref": job_id,
    });

    if optional_bool(obj, "dry_run").unwrap_or(false) {
        let adapter_result = video_cancelled_adapter_result(
            task_id,
            &job_id,
            provider_name,
            &model,
            adapter_kind,
            cancelled_at,
        );
        return Ok((
            format!("VIDEO_TASK_CANCELLED:{task_id}"),
            json!({
                "provider": provider_name,
                "model": model,
                "model_kind": adapter_kind_name(adapter_kind),
                "task_id": task_id,
                "job_id": job_id,
                "status": "cancelled",
                "dry_run": true,
                "provider_cancel_contract": provider_cancel_contract,
                "async_cancel_adapter_result": adapter_result,
                "async_poll_adapter_result": adapter_result,
            }),
        ));
    }

    let adapter_result = json!({
        "schema_version": 1,
        "adapter_kind": "media_job_poll",
        "status": "requires_provider_adapter",
        "job_id": job_id,
        "result_ref": job_id,
        "cancel_ref": job_id,
        "cancel_token": job_id,
        "cancelled_at": cancelled_at,
        "message_key": "clawd.task.cancelled",
        "error_code": "provider_cancel_adapter_missing",
        "retryable": false,
        "provider_cancel_contract": provider_cancel_contract,
    });
    Ok((
        format!("VIDEO_TASK_CANCEL_ADAPTER_REQUIRED:{task_id}"),
        json!({
            "provider": provider_name,
            "model": model,
            "model_kind": adapter_kind_name(adapter_kind),
            "task_id": task_id,
            "job_id": job_id,
            "status": "requires_provider_adapter",
            "provider_cancel_contract": provider_cancel_contract,
            "async_cancel_adapter_result": adapter_result,
        }),
    ))
}

fn video_cancelled_adapter_result(
    task_id: &str,
    job_id: &str,
    provider: &str,
    model: &str,
    model_kind: VideoAdapterKind,
    cancelled_at: i64,
) -> Value {
    json!({
        "schema_version": 1,
        "adapter_kind": "media_job_poll",
        "status": "cancelled",
        "job_id": job_id,
        "result_ref": job_id,
        "cancel_ref": job_id,
        "cancel_token": job_id,
        "poll_after_seconds": 0,
        "poll_after_ms": 0,
        "expires_at": cancelled_at,
        "message_key": "clawd.task.cancelled",
        "retryable": false,
        "cancellation_result_json": {
            "schema_version": 1,
            "source": "video_generate_cancel_adapter",
            "provider": provider,
            "model": model,
            "model_kind": adapter_kind_name(model_kind),
            "task_id": task_id,
            "job_id": job_id,
            "cancel_ref": job_id,
            "status": "cancelled",
            "cancelled_at": cancelled_at,
            "dry_run": true,
        },
    })
}

#[allow(clippy::too_many_arguments)]
fn adapter_result_from_video_query(
    workspace_root: &Path,
    default_output_dir: &str,
    obj: &Map<String, Value>,
    task_id: &str,
    job_id: &str,
    provider: &str,
    model: &str,
    model_kind: VideoAdapterKind,
    poll_after_seconds: u64,
    expires_at: i64,
    query: Value,
    dry_run: bool,
    should_download: bool,
    download_url: Option<String>,
) -> Result<Value, String> {
    let status = query
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    if status == "Fail" {
        return Ok(video_poll_adapter_result(
            job_id,
            "failed",
            poll_after_seconds,
            expires_at,
            Some(json!({
                "schema_version": 1,
                "source": "video_generate_poll_adapter",
                "provider": provider,
                "model": model,
                "model_kind": adapter_kind_name(model_kind),
                "task_id": task_id,
                "status": status,
                "query": query,
            })),
            Some("provider_video_job_failed"),
            Some("clawd.task.async_poll_adapter_failed"),
        ));
    }
    if status != "Success" {
        return Ok(video_poll_adapter_result(
            job_id,
            if status == "Queueing" {
                "accepted"
            } else {
                "running"
            },
            poll_after_seconds,
            expires_at,
            None,
            None,
            Some("clawd.task.async_job_pending"),
        ));
    }
    let file_id = query
        .get("file_id")
        .and_then(value_to_string)
        .ok_or_else(|| "minimax video success response missing file_id".to_string())?;
    let output_path = resolve_output_path(
        workspace_root,
        default_output_dir,
        obj.get("output_path").and_then(Value::as_str),
    )?;
    let output = output_path.to_string_lossy().to_string();
    let mut final_result = json!({
        "schema_version": 1,
        "source": "video_generate_poll_adapter",
        "provider": provider,
        "model": model,
        "model_kind": adapter_kind_name(model_kind),
        "task_id": task_id,
        "status": status,
        "file_id": file_id,
        "output_path": output,
        "outputs": [],
        "query": query,
        "dry_run": dry_run,
    });
    if should_download {
        if let Some(obj) = final_result.as_object_mut() {
            obj.insert(
                "outputs".to_string(),
                json!([{"type":"video_file","path": output}]),
            );
            if let Some(download_url) = download_url {
                obj.insert("download_url".to_string(), json!(download_url));
            }
        }
    }
    Ok(video_poll_adapter_result(
        job_id,
        "succeeded",
        poll_after_seconds,
        expires_at,
        Some(final_result),
        None,
        Some("clawd.task.async_job_completed"),
    ))
}

fn video_poll_adapter_result(
    job_id: &str,
    status: &str,
    poll_after_seconds: u64,
    expires_at: i64,
    payload: Option<Value>,
    error_code: Option<&str>,
    message_key: Option<&str>,
) -> Value {
    let mut result = json!({
        "job_id": job_id,
        "result_ref": job_id,
        "status": status,
        "poll_after_seconds": poll_after_seconds,
        "poll_after_ms": poll_after_seconds.saturating_mul(1_000),
        "expires_at": expires_at,
        "message_key": message_key.unwrap_or("clawd.task.async_job_pending"),
        "retryable": matches!(status, "accepted" | "running"),
    });
    if let Some(obj) = result.as_object_mut() {
        match status {
            "succeeded" => {
                if let Some(payload) = payload.filter(Value::is_object) {
                    obj.insert("final_result_json".to_string(), payload);
                }
            }
            "failed" | "expired" => {
                if let Some(error_code) = error_code {
                    obj.insert("error_code".to_string(), json!(error_code));
                }
                if let Some(payload) = payload.filter(Value::is_object) {
                    obj.insert("failure_result_json".to_string(), payload);
                }
            }
            _ => {}
        }
    }
    result
}

#[allow(clippy::too_many_arguments)]
fn video_poll_response(
    task_id: &str,
    job_id: &str,
    provider: &str,
    model: &str,
    model_kind: VideoAdapterKind,
    poll_after_seconds: u64,
    expires_at: i64,
    adapter_result: Value,
    query: Value,
) -> (String, Value) {
    (
        format!("VIDEO_TASK:{task_id}"),
        json!({
            "provider": provider,
            "model": model,
            "model_kind": adapter_kind_name(model_kind),
            "task_id": task_id,
            "job_id": job_id,
            "status": query.get("status").cloned().unwrap_or(Value::Null),
            "poll_after_seconds": poll_after_seconds,
            "expires_at": expires_at,
            "query": query,
            "async_poll_adapter_result": adapter_result,
        }),
    )
}

fn create_video_task(
    client: &Client,
    cfg: &VendorConfig,
    payload: &Value,
) -> Result<String, String> {
    let url = format!("{}/video_generation", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .post(url)
        .bearer_auth(&cfg.api_key)
        .json(payload)
        .send()
        .map_err(|err| format!("minimax video create request failed: {err}"))?;
    let status = resp.status().as_u16();
    let value: Value = resp
        .json()
        .map_err(|err| format!("parse minimax video create response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "minimax video create failed status={status}: {}",
            truncate(&value.to_string(), 400)
        ));
    }
    check_base_resp(&value, "minimax video create")?;
    value
        .get("task_id")
        .and_then(value_to_string)
        .ok_or_else(|| {
            format!(
                "minimax video create response missing task_id: {}",
                truncate(&value.to_string(), 400)
            )
        })
}

fn poll_video_task(
    client: &Client,
    cfg: &VendorConfig,
    task_id: &str,
    poll_interval_ms: u64,
    max_poll_seconds: u64,
) -> Result<Value, String> {
    let started = Instant::now();
    loop {
        let value = query_video_task(client, cfg, task_id)?;
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if matches!(status, "Success" | "Fail") || started.elapsed().as_secs() >= max_poll_seconds {
            return Ok(value);
        }
        thread::sleep(Duration::from_millis(poll_interval_ms));
    }
}

fn query_video_task(client: &Client, cfg: &VendorConfig, task_id: &str) -> Result<Value, String> {
    let url = format!(
        "{}/query/video_generation",
        trim_trailing_slash(&cfg.base_url)
    );
    let resp = client
        .get(url)
        .bearer_auth(&cfg.api_key)
        .query(&[("task_id", task_id)])
        .send()
        .map_err(|err| format!("minimax video query request failed: {err}"))?;
    let status = resp.status().as_u16();
    let value: Value = resp
        .json()
        .map_err(|err| format!("parse minimax video query response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "minimax video query failed status={status}: {}",
            truncate(&value.to_string(), 400)
        ));
    }
    check_base_resp(&value, "minimax video query")?;
    Ok(value)
}

fn retrieve_file_url(client: &Client, cfg: &VendorConfig, file_id: &str) -> Result<String, String> {
    let url = format!("{}/files/retrieve", trim_trailing_slash(&cfg.base_url));
    let resp = client
        .get(url)
        .bearer_auth(&cfg.api_key)
        .query(&[("file_id", file_id)])
        .send()
        .map_err(|err| format!("minimax file retrieve request failed: {err}"))?;
    let status = resp.status().as_u16();
    let value: Value = resp
        .json()
        .map_err(|err| format!("parse minimax file retrieve response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "minimax file retrieve failed status={status}: {}",
            truncate(&value.to_string(), 400)
        ));
    }
    check_base_resp(&value, "minimax file retrieve")?;
    value
        .get("file")
        .and_then(|file| file.get("download_url"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            format!(
                "minimax file retrieve response missing file.download_url: {}",
                truncate(&value.to_string(), 400)
            )
        })
}

fn download_to_path(client: &Client, url: &str, output_path: &Path) -> Result<(), String> {
    let resp = client
        .get(url)
        .send()
        .map_err(|err| format!("download video failed: {err}"))?;
    let status = resp.status().as_u16();
    let bytes = resp
        .bytes()
        .map_err(|err| format!("read video response failed: {err}"))?;
    if status >= 300 {
        return Err(format!(
            "download video failed status={status}: {}",
            truncate(&String::from_utf8_lossy(&bytes), 400)
        ));
    }
    ensure_parent_dir(output_path)?;
    std::fs::write(output_path, bytes).map_err(|err| format!("write video output failed: {err}"))
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

fn image_arg_to_api_value(
    workspace_root: &Path,
    value: Option<&Value>,
    max_input_bytes: u64,
) -> Result<Option<String>, String> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Some(s) = value.as_str().map(str::trim).filter(|v| !v.is_empty()) {
        return image_string_to_api_value(workspace_root, s, max_input_bytes).map(Some);
    }
    if let Some(obj) = value.as_object() {
        if let Some(url) = obj
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Ok(Some(url.to_string()));
        }
        if let Some(data_url) = obj
            .get("data_url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return Ok(Some(data_url.to_string()));
        }
        if let Some(b64) = obj
            .get("base64")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let mime = obj
                .get("mime")
                .and_then(Value::as_str)
                .unwrap_or("image/png");
            return Ok(Some(format!("data:{mime};base64,{b64}")));
        }
        if let Some(path) = obj
            .get("path")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            return image_path_to_data_url(workspace_root, path, max_input_bytes).map(Some);
        }
    }
    Err("image input must be url/path/base64/data_url".to_string())
}

fn image_string_to_api_value(
    workspace_root: &Path,
    raw: &str,
    max_input_bytes: u64,
) -> Result<String, String> {
    if raw.starts_with("http://") || raw.starts_with("https://") || raw.starts_with("data:image/") {
        return Ok(raw.to_string());
    }
    image_path_to_data_url(workspace_root, raw, max_input_bytes)
}

fn image_path_to_data_url(
    workspace_root: &Path,
    raw_path: &str,
    max_input_bytes: u64,
) -> Result<String, String> {
    let path = normalize_workspace_path(workspace_root, raw_path)?;
    let metadata =
        std::fs::metadata(&path).map_err(|err| format!("read image metadata failed: {err}"))?;
    if metadata.len() > max_input_bytes {
        return Err(format!(
            "image input too large: {} bytes, max={max_input_bytes}",
            metadata.len()
        ));
    }
    let bytes = std::fs::read(&path).map_err(|err| format!("read image input failed: {err}"))?;
    let mime = image_mime_from_path(&path);
    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn image_mime_from_path(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn normalize_workspace_path(workspace_root: &Path, raw_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(raw_path);
    let out = if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace_root.join(p)
    };
    if !out.starts_with(workspace_root) {
        return Err("path is outside workspace".to_string());
    }
    Ok(out)
}

fn resolve_output_path(
    workspace_root: &Path,
    default_dir: &str,
    requested: Option<&str>,
) -> Result<PathBuf, String> {
    if let Some(path) = requested.map(str::trim).filter(|value| !value.is_empty()) {
        let out = normalize_workspace_path(workspace_root, path)?;
        return Ok(out);
    }
    Ok(workspace_root
        .join(default_dir)
        .join(format!("video-{}.mp4", unix_ts())))
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
    let video_cfg = read_toml(root.join("configs/video.toml"));
    let mut cfg = RootConfig::default();
    if let Some(value) = core_cfg.get("llm").cloned() {
        if let Ok(parsed) = value.try_into::<LlmConfig>() {
            cfg.llm = parsed;
        }
    }
    if let Some(value) = video_cfg.get("video_generation").cloned() {
        if let Ok(parsed) = value.try_into::<VideoGenerationConfig>() {
            cfg.video_generation = parsed;
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
        &mut cfg.video_generation.providers.openai,
        "VIDEO_GENERATION_OPENAI_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.google,
        "VIDEO_GENERATION_GOOGLE_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.anthropic,
        "VIDEO_GENERATION_ANTHROPIC_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.grok,
        "VIDEO_GENERATION_GROK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.deepseek,
        "VIDEO_GENERATION_DEEPSEEK_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.qwen,
        "VIDEO_GENERATION_QWEN_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.minimax,
        "VIDEO_GENERATION_MINIMAX_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.mimo,
        "VIDEO_GENERATION_MIMO_API_KEY",
    );
    apply_vendor_api_key_env(
        &mut cfg.video_generation.providers.custom,
        "VIDEO_GENERATION_CUSTOM_API_KEY",
    );
}

fn resolved_vendor_config(cfg: &RootConfig, vendor: VendorKind) -> Option<VendorConfig> {
    let dedicated = match vendor {
        VendorKind::OpenAI => cfg.video_generation.providers.openai.clone(),
        VendorKind::Google => cfg.video_generation.providers.google.clone(),
        VendorKind::Anthropic => cfg.video_generation.providers.anthropic.clone(),
        VendorKind::Grok => cfg.video_generation.providers.grok.clone(),
        VendorKind::DeepSeek => cfg.video_generation.providers.deepseek.clone(),
        VendorKind::Qwen => cfg.video_generation.providers.qwen.clone(),
        VendorKind::MiniMax => cfg.video_generation.providers.minimax.clone(),
        VendorKind::Mimo => cfg.video_generation.providers.mimo.clone(),
        VendorKind::Custom => cfg.video_generation.providers.custom.clone(),
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

fn vendor_models(cfg: &VideoGenerationConfig, vendor: VendorKind) -> Option<&Vec<String>> {
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

fn adapter_kind_for(vendor: VendorKind, cfg: Option<&VendorConfig>) -> VideoAdapterKind {
    if matches!(vendor, VendorKind::MiniMax) {
        return VideoAdapterKind::MiniMaxNative;
    }
    match cfg
        .and_then(|cfg| cfg.adapter_kind.as_deref())
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("minimax") | Some("minimax_native") | Some("minimax_compatible") => {
            VideoAdapterKind::MiniMaxNative
        }
        _ => VideoAdapterKind::Unsupported,
    }
}

fn adapter_kind_name(kind: VideoAdapterKind) -> &'static str {
    match kind {
        VideoAdapterKind::MiniMaxNative => "minimax_native",
        VideoAdapterKind::Unsupported => "unsupported",
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

fn value_to_string(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = value.as_i64() {
        return Some(n.to_string());
    }
    value.as_u64().map(|n| n.to_string())
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
