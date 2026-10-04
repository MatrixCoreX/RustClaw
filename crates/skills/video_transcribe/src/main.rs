use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SKILL_NAME: &str = "video_transcribe";

#[derive(Debug, Deserialize)]
struct Request {
    request_id: String,
    #[serde(default)]
    args: Value,
    #[serde(default)]
    context: Option<Value>,
}

#[derive(Debug, Serialize)]
struct Response {
    request_id: String,
    status: &'static str,
    text: String,
    error_text: Option<String>,
    extra: Value,
}

#[derive(Debug)]
struct SkillFailure {
    code: &'static str,
    message: String,
    retryable: bool,
    details: Option<Value>,
}

impl SkillFailure {
    fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            message: message.into(),
            retryable,
            details: None,
        }
    }

    fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }
}

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => response_for(request),
            Err(error) => error_response(
                "unknown".to_string(),
                SkillFailure::new("invalid_input", format!("invalid input: {error}"), false),
            ),
        };
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn response_for(request: Request) -> Response {
    match execute(&request.args, request.context.as_ref()) {
        Ok(extra) => Response {
            request_id: request.request_id,
            status: "ok",
            text: "VIDEO_AUDIO_READY".to_string(),
            error_text: None,
            extra,
        },
        Err(error) => error_response(request.request_id, error),
    }
}

fn error_response(request_id: String, error: SkillFailure) -> Response {
    let mut extra = json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "error",
        "error_code": error.code,
        "message_key": format!("skill.{SKILL_NAME}.{}", error.code),
        "retryable": error.retryable,
    });
    if let (Some(object), Some(details)) = (extra.as_object_mut(), error.details) {
        object.insert("details".to_string(), details);
    }
    Response {
        request_id,
        status: "error",
        text: String::new(),
        error_text: Some(error.message),
        extra,
    }
}

fn execute(args: &Value, context: Option<&Value>) -> Result<Value, SkillFailure> {
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("extract_audio");
    if action != "extract_audio" {
        return Err(SkillFailure::new(
            "invalid_input",
            format!("unsupported action: {action}"),
            false,
        ));
    }

    let raw_input = video_input(args).ok_or_else(|| {
        SkillFailure::new("invalid_input", "a local video path is required", false)
    })?;
    let workspace_root = context
        .and_then(|value| value.get("workspace_root"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("WORKSPACE_ROOT").map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let allow_outside = context
        .and_then(|value| value.get("allow_path_outside_workspace"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || std::env::var("APP_ALLOW_PATH_OUTSIDE_WORKSPACE").is_ok_and(|value| value == "1");
    let input_path = resolve_input_path(&workspace_root, &raw_input, allow_outside)?;
    let output_directory = artifact_output_directory(context)?;
    fs::create_dir_all(&output_directory).map_err(|error| {
        SkillFailure::new(
            "artifact_directory_unavailable",
            format!("cannot create artifact directory: {error}"),
            true,
        )
    })?;

    ensure_stream(&input_path, "v:0", "video_stream_missing")?;
    ensure_stream(&input_path, "a:0", "audio_stream_missing")?;

    let output_path = output_directory.join("video_audio.wav");
    let partial_path = output_directory.join("video_audio.partial.wav");
    let _ = fs::remove_file(&partial_path);
    let completed = Command::new("ffmpeg")
        .args(ffmpeg_extract_args(&input_path, &partial_path))
        .output()
        .map_err(|error| {
            SkillFailure::new(
                "ffmpeg_unavailable",
                format!("cannot start ffmpeg: {error}"),
                false,
            )
        })?;
    if !completed.status.success() {
        let _ = fs::remove_file(&partial_path);
        return Err(SkillFailure::new(
            "audio_extraction_failed",
            "ffmpeg could not extract the video audio stream",
            false,
        )
        .with_details(json!({
            "exit_code": completed.status.code(),
            "failure_phase": "audio_extraction",
        })));
    }
    let size_bytes = fs::metadata(&partial_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if size_bytes <= 44 {
        let _ = fs::remove_file(&partial_path);
        return Err(SkillFailure::new(
            "extracted_audio_empty",
            "the extracted audio stream is empty",
            false,
        ));
    }
    fs::rename(&partial_path, &output_path).map_err(|error| {
        let _ = fs::remove_file(&partial_path);
        SkillFailure::new(
            "audio_artifact_finalize_failed",
            format!("cannot finalize extracted audio: {error}"),
            true,
        )
    })?;

    let output_path = output_path.to_string_lossy().into_owned();
    let extracted_audio = json!({
        "status": "available",
        "path": output_path,
        "filename": "video_audio.wav",
        "mime_type": "audio/wav",
        "size_bytes": size_bytes,
        "sample_rate": 16000,
        "channels": 1,
        "encoding": "pcm_s16le",
        "artifact_role": "transcription_input",
        "deliver_to_user": false,
    });
    let continuation = json!({
        "activation_requirement": "required",
        "completion_requirement": "selected_components",
        "steps": [{
            "component_kind": "video_audio",
            "capability": "audio.preview_transcribe",
            "input_field": "input_path",
            "input_value": output_path,
            "completion_capabilities": ["audio.transcribe", "local_asr.transcribe"],
            "recommended_capability_pointer": "/extra/recommended_capability",
            "result_label_kind": "audio_transcript",
        }],
        "synthesis_sources": ["audio_transcript"],
        "result_label_kinds": {"audio_transcript": "video_audio_transcript"},
    });
    Ok(json!({
        "schema_version": 1,
        "source_skill": SKILL_NAME,
        "status": "ok",
        "action": "extract_audio",
        "source_video": {
            "path": input_path,
            "size_bytes": fs::metadata(&input_path).map(|metadata| metadata.len()).unwrap_or(0),
        },
        "extracted_audio": extracted_audio,
        "followup_policy": {
            "activation_requirement": "required",
            "next_capability": "audio.preview_transcribe",
            "completion_capabilities": ["audio.transcribe", "local_asr.transcribe"],
            "fallback_recommended": false,
        },
        "content_bundle": {
            "kind": "video_audio",
            "processing_inputs": {"video_audio": extracted_audio},
            "followup_policy": continuation,
        },
    }))
}

fn video_input(args: &Value) -> Option<String> {
    for key in ["video_path", "input_path", "path", "file"] {
        if let Some(value) = args
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            return Some(value.to_string());
        }
    }
    let video = args.get("video")?;
    if let Some(value) = video
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Some(value.to_string());
    }
    video
        .get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn resolve_input_path(
    workspace_root: &Path,
    raw_input: &str,
    allow_outside: bool,
) -> Result<PathBuf, SkillFailure> {
    let candidate = if Path::new(raw_input).is_absolute() {
        PathBuf::from(raw_input)
    } else {
        workspace_root.join(raw_input)
    };
    let canonical = candidate.canonicalize().map_err(|_| {
        SkillFailure::new(
            "video_input_missing",
            "the video input does not exist",
            false,
        )
    })?;
    if !canonical.is_file() {
        return Err(SkillFailure::new(
            "video_input_invalid",
            "the video input is not a regular file",
            false,
        ));
    }
    if !allow_outside {
        let canonical_root = workspace_root.canonicalize().map_err(|error| {
            SkillFailure::new(
                "workspace_unavailable",
                format!("cannot resolve workspace root: {error}"),
                false,
            )
        })?;
        if !canonical.starts_with(canonical_root) {
            return Err(SkillFailure::new(
                "video_path_outside_workspace",
                "the video input is outside the permitted workspace",
                false,
            ));
        }
    }
    Ok(canonical)
}

fn artifact_output_directory(context: Option<&Value>) -> Result<PathBuf, SkillFailure> {
    context
        .and_then(|value| value.get("artifact_output_directory"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| {
            SkillFailure::new(
                "artifact_directory_missing",
                "runtime artifact output directory is required",
                false,
            )
        })
}

fn ensure_stream(
    input_path: &Path,
    selector: &str,
    missing_code: &'static str,
) -> Result<(), SkillFailure> {
    let completed = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            selector,
            "-show_entries",
            "stream=index",
            "-of",
            "csv=p=0",
        ])
        .arg(input_path)
        .output()
        .map_err(|error| {
            SkillFailure::new(
                "ffprobe_unavailable",
                format!("cannot start ffprobe: {error}"),
                false,
            )
        })?;
    if !completed.status.success() || String::from_utf8_lossy(&completed.stdout).trim().is_empty() {
        return Err(SkillFailure::new(
            missing_code,
            if selector.starts_with('v') {
                "the input does not contain a readable video stream"
            } else {
                "the video does not contain an audio stream"
            },
            false,
        ));
    }
    Ok(())
}

fn ffmpeg_extract_args(input_path: &Path, output_path: &Path) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-nostdin".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-protocol_whitelist".to_string(),
        "file,pipe".to_string(),
        "-i".to_string(),
        input_path.to_string_lossy().into_owned(),
        "-map".to_string(),
        "0:a:0".to_string(),
        "-vn".to_string(),
        "-ac".to_string(),
        "1".to_string(),
        "-ar".to_string(),
        "16000".to_string(),
        "-c:a".to_string(),
        "pcm_s16le".to_string(),
        "-y".to_string(),
        output_path.to_string_lossy().into_owned(),
    ]
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
