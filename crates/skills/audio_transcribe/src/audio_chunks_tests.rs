use super::*;
use std::process::Command;

#[test]
fn small_remote_input_stays_unchanged_when_duration_is_within_boundary() {
    let root = create_test_directory("small");
    let source = root.join("small.wav");
    fs::write(&source, b"not-a-real-wave").unwrap();

    let prepared = prepare_remote_audio(
        &source,
        RemoteChunkConfig {
            enabled: true,
            target_bytes: 1_024,
            max_input_bytes: 2_048,
            max_duration_seconds: 480,
        },
    )
    .unwrap();

    assert!(!prepared.applied);
    assert_eq!(prepared.chunks.len(), 1);
    assert_eq!(prepared.chunks[0].path, source);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn disabled_chunking_rejects_an_input_over_the_machine_boundary() {
    let root = create_test_directory("disabled");
    let source = root.join("large.wav");
    fs::write(&source, vec![0_u8; 32]).unwrap();

    let error = prepare_remote_audio(
        &source,
        RemoteChunkConfig {
            enabled: false,
            target_bytes: 16,
            max_input_bytes: 64,
            max_duration_seconds: 480,
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "input_too_large");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn ffmpeg_segments_remote_audio_and_drop_removes_private_chunks() {
    if Command::new("ffmpeg").arg("-version").output().is_err()
        || Command::new("ffprobe").arg("-version").output().is_err()
    {
        return;
    }
    let root = create_test_directory("ffmpeg");
    let source = root.join("long.wav");
    let generated = Command::new("ffmpeg")
        .args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3.2",
            "-ac",
            "1",
            "-ar",
            "16000",
        ])
        .arg(&source)
        .status()
        .unwrap();
    assert!(generated.success());

    let prepared = prepare_remote_audio(
        &source,
        RemoteChunkConfig {
            enabled: true,
            target_bytes: 1,
            max_input_bytes: 128 * 1_024,
            max_duration_seconds: 1,
        },
    )
    .unwrap();
    assert!(prepared.applied);
    assert_eq!(prepared.chunks.len(), 4);
    assert!(prepared
        .chunks
        .iter()
        .all(|chunk| chunk.size_bytes <= 128 * 1_024));
    let temporary_directory = prepared.temporary_directory().unwrap().to_path_buf();
    assert!(temporary_directory.is_dir());
    drop(prepared);
    assert!(!temporary_directory.exists());
    fs::remove_dir_all(root).unwrap();
}

fn create_test_directory(label: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "agent-audio-chunks-test-{label}-{}-{}",
        std::process::id(),
        TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&path).unwrap();
    path
}
