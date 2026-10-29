use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn temporary_directory(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "video-transcribe-{label}-{}-{nonce}",
        std::process::id()
    ))
}

#[test]
fn video_input_accepts_structured_path() {
    assert_eq!(
        video_input(&json!({"video": {"path": "data/channel/sample.mp4"}})).as_deref(),
        Some("data/channel/sample.mp4")
    );
}

#[test]
fn missing_input_proves_no_effect_without_creating_output() {
    let root = temporary_directory("missing-input");
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&root).unwrap();
    let response = response_for(Request {
        request_id: "missing-input".into(),
        args: json!({"action":"extract_audio","path":"absent.mp4"}),
        context: Some(json!({"workspace_root":root,"artifact_output_directory":artifacts})),
    });
    assert_eq!(response.status, "error");
    assert_eq!(response.extra["error_code"], "video_input_missing");
    assert_eq!(response.extra["side_effect_applied"], false);
    assert_eq!(response.extra["failure_phase"], "execution_no_effect");
    assert_eq!(response.extra["retryable"], false);
    assert!(!artifacts.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn invalid_action_or_missing_path_are_effect_free() {
    for args in [
        json!({"action":"unsupported"}),
        json!({"action":"extract_audio"}),
    ] {
        let response = response_for(Request {
            request_id: "invalid".into(),
            args,
            context: None,
        });
        assert_eq!(response.extra["error_code"], "invalid_input");
        assert_eq!(response.extra["side_effect_applied"], false);
        assert_eq!(response.extra["failure_phase"], "execution_no_effect");
    }
}

#[test]
fn missing_output_context_is_rejected_before_probes_or_writes() {
    let root = temporary_directory("missing-context");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("input.mp4"), b"not probed").unwrap();
    let response = response_for(Request {
        request_id: "missing-context".into(),
        args: json!({"path":"input.mp4"}),
        context: Some(json!({"workspace_root":root})),
    });
    assert_eq!(response.extra["error_code"], "artifact_directory_missing");
    assert_eq!(response.extra["side_effect_applied"], false);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failures_after_mutation_do_not_claim_no_effect() {
    let response = error_response(
        "post-write".into(),
        SkillFailure::new("audio_artifact_finalize_failed", "fixture failure", true)
            .with_details(json!({"failure_phase":"audio_extraction"})),
    );
    assert_eq!(
        response.extra["error_code"],
        "audio_artifact_finalize_failed"
    );
    assert!(response.extra.get("side_effect_applied").is_none());
    assert!(response.extra.get("failure_phase").is_none());
    assert_eq!(
        response.extra["details"]["failure_phase"],
        "audio_extraction"
    );
}

#[test]
fn ffmpeg_command_selects_first_audio_stream_and_standard_pcm() {
    let args = ffmpeg_extract_args(Path::new("input.mp4"), Path::new("output.wav"));
    assert!(args.windows(2).any(|pair| pair == ["-map", "0:a:0"]));
    assert!(args.windows(2).any(|pair| pair == ["-ar", "16000"]));
    assert!(args.windows(2).any(|pair| pair == ["-ac", "1"]));
    assert!(args.windows(2).any(|pair| pair == ["-c:a", "pcm_s16le"]));
}

#[test]
fn extract_audio_happy_path_when_ffmpeg_is_available() {
    if Command::new("ffmpeg").arg("-version").output().is_err()
        || Command::new("ffprobe").arg("-version").output().is_err()
    {
        return;
    }
    let root = temporary_directory("happy");
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&artifacts).expect("create fixture directories");
    let video = root.join("sample.mp4");
    let generated = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:d=0.2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=0.2",
            "-shortest",
            "-c:v",
            "mpeg4",
            "-c:a",
            "aac",
            "-y",
        ])
        .arg(&video)
        .status()
        .expect("start fixture ffmpeg");
    assert!(generated.success(), "generate fixture video");

    let extra = execute(
        &json!({"action": "extract_audio", "video": {"path": "sample.mp4"}}),
        Some(&json!({
            "workspace_root": root,
            "artifact_output_directory": artifacts,
            "allow_path_outside_workspace": false,
        })),
    )
    .expect("extract video audio");
    let audio_path = PathBuf::from(
        extra["extracted_audio"]["path"]
            .as_str()
            .expect("audio path"),
    );
    assert!(audio_path.is_file());
    assert_eq!(extra["extracted_audio"]["sample_rate"], 16000);
    assert_eq!(
        extra["followup_policy"]["next_capability"],
        "audio.preview_transcribe"
    );
    assert_eq!(
        extra["content_bundle"]["followup_policy"]["steps"][0]["input_value"],
        audio_path.to_string_lossy().as_ref()
    );
    assert_eq!(
        extra["content_bundle"]["followup_policy"]["steps"][0]["recommended_capability_pointer"],
        "/extra/recommended_capability"
    );
    fs::remove_dir_all(root).expect("remove fixture");
}

#[test]
fn video_without_audio_returns_structured_failure_when_ffmpeg_is_available() {
    if Command::new("ffmpeg").arg("-version").output().is_err()
        || Command::new("ffprobe").arg("-version").output().is_err()
    {
        return;
    }
    let root = temporary_directory("no-audio");
    let artifacts = root.join("artifacts");
    fs::create_dir_all(&root).expect("create fixture directories");
    let video = root.join("silent.mp4");
    let generated = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostdin",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=c=black:s=64x64:d=0.2",
            "-c:v",
            "mpeg4",
            "-an",
            "-y",
        ])
        .arg(&video)
        .status()
        .expect("start fixture ffmpeg");
    assert!(generated.success(), "generate silent fixture video");

    let failure = execute(
        &json!({"action": "extract_audio", "video_path": "silent.mp4"}),
        Some(&json!({
            "workspace_root": root,
            "artifact_output_directory": artifacts,
            "allow_path_outside_workspace": false,
        })),
    )
    .expect_err("silent video must fail");
    assert_eq!(failure.code, "audio_stream_missing");
    assert!(!failure.retryable);
    assert!(failure.no_effect);
    assert!(!artifacts.exists());
    fs::remove_dir_all(root).expect("remove fixture");
}
