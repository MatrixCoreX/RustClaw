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
    fs::create_dir_all(&artifacts).expect("create fixture directories");
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
    fs::remove_dir_all(root).expect("remove fixture");
}
