use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const NORMALIZED_SAMPLE_RATE: u32 = 16_000;
const NORMALIZED_CHANNELS: u8 = 1;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy)]
pub(super) struct RemoteChunkConfig {
    pub enabled: bool,
    pub target_bytes: u64,
    pub max_input_bytes: u64,
    pub max_duration_seconds: u64,
}

#[derive(Debug, Clone)]
pub(super) struct AudioChunk {
    pub path: PathBuf,
    pub index: usize,
    pub start_ms: u64,
    pub end_ms: Option<u64>,
    pub size_bytes: u64,
}

#[derive(Debug)]
pub(super) struct PreparedAudio {
    pub applied: bool,
    pub original_size_bytes: u64,
    pub original_duration_ms: Option<u64>,
    pub chunks: Vec<AudioChunk>,
    _temp_dir: Option<TemporaryAudioDirectory>,
}

impl PreparedAudio {
    #[cfg(test)]
    pub(super) fn temporary_directory(&self) -> Option<&Path> {
        self._temp_dir
            .as_ref()
            .map(|directory| directory.path.as_path())
    }
}

#[derive(Debug)]
struct TemporaryAudioDirectory {
    path: PathBuf,
}

impl Drop for TemporaryAudioDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
pub(super) struct ChunkPreparationError {
    pub code: &'static str,
    pub detail: String,
    pub retryable: bool,
}

impl ChunkPreparationError {
    fn new(code: &'static str, detail: impl Into<String>, retryable: bool) -> Self {
        Self {
            code,
            detail: detail.into(),
            retryable,
        }
    }
}

pub(super) fn prepare_remote_audio(
    audio_path: &Path,
    config: RemoteChunkConfig,
) -> Result<PreparedAudio, ChunkPreparationError> {
    let metadata = fs::metadata(audio_path).map_err(|error| {
        ChunkPreparationError::new(
            "invalid_input",
            format!("read audio metadata failed: {error}"),
            false,
        )
    })?;
    if !metadata.is_file() {
        return Err(ChunkPreparationError::new(
            "invalid_input",
            "audio input is not a regular file",
            false,
        ));
    }

    let original_size_bytes = metadata.len();
    let original_duration_ms = probe_duration_ms(audio_path).ok();
    let duration_exceeds_limit = original_duration_ms
        .is_some_and(|duration_ms| duration_ms > config.max_duration_seconds.saturating_mul(1_000));
    let size_exceeds_target = original_size_bytes > config.target_bytes;
    if !duration_exceeds_limit && !size_exceeds_target {
        return Ok(PreparedAudio {
            applied: false,
            original_size_bytes,
            original_duration_ms,
            chunks: vec![AudioChunk {
                path: audio_path.to_path_buf(),
                index: 1,
                start_ms: 0,
                end_ms: original_duration_ms,
                size_bytes: original_size_bytes,
            }],
            _temp_dir: None,
        });
    }
    if !config.enabled {
        return Err(ChunkPreparationError::new(
            "input_too_large",
            format!(
                "remote audio exceeds the configured single-request boundary: bytes={original_size_bytes}, duration_ms={}",
                original_duration_ms
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ),
            false,
        ));
    }

    let temp_dir = create_private_temp_directory()?;
    let output_pattern = temp_dir.path.join("chunk_%06d.wav");
    let output = Command::new("ffmpeg")
        .arg("-v")
        .arg("error")
        .arg("-nostdin")
        .arg("-y")
        .arg("-i")
        .arg(audio_path)
        .arg("-vn")
        .arg("-ac")
        .arg(NORMALIZED_CHANNELS.to_string())
        .arg("-ar")
        .arg(NORMALIZED_SAMPLE_RATE.to_string())
        .arg("-c:a")
        .arg("pcm_s16le")
        .arg("-f")
        .arg("segment")
        .arg("-segment_time")
        .arg(config.max_duration_seconds.to_string())
        .arg("-reset_timestamps")
        .arg("1")
        .arg(&output_pattern)
        .output()
        .map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                "dependency_unavailable"
            } else {
                "audio_split_failed"
            };
            ChunkPreparationError::new(code, format!("start ffmpeg failed: {error}"), false)
        })?;
    ensure_command_succeeded("ffmpeg", &output)?;

    let mut paths = fs::read_dir(&temp_dir.path)
        .map_err(|error| {
            ChunkPreparationError::new(
                "audio_split_failed",
                format!("read audio chunk directory failed: {error}"),
                false,
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("wav"))
        .collect::<Vec<_>>();
    paths.sort();
    if paths.is_empty() {
        return Err(ChunkPreparationError::new(
            "audio_split_failed",
            "ffmpeg produced no audio chunks",
            false,
        ));
    }

    let max_duration_ms = config.max_duration_seconds.saturating_mul(1_000);
    let mut chunks = Vec::with_capacity(paths.len());
    for (offset, path) in paths.into_iter().enumerate() {
        let size_bytes = fs::metadata(&path)
            .map_err(|error| {
                ChunkPreparationError::new(
                    "audio_split_failed",
                    format!("read audio chunk metadata failed: {error}"),
                    false,
                )
            })?
            .len();
        if size_bytes == 0 || size_bytes > config.max_input_bytes {
            return Err(ChunkPreparationError::new(
                "audio_chunk_invalid",
                format!(
                    "normalized audio chunk is outside the provider boundary: index={}, bytes={size_bytes}, max={}",
                    offset + 1,
                    config.max_input_bytes
                ),
                false,
            ));
        }
        let start_ms = (offset as u64).saturating_mul(max_duration_ms);
        let inferred_end_ms = start_ms.saturating_add(max_duration_ms);
        let end_ms = original_duration_ms
            .map(|duration_ms| inferred_end_ms.min(duration_ms))
            .or(Some(inferred_end_ms));
        chunks.push(AudioChunk {
            path,
            index: offset + 1,
            start_ms,
            end_ms,
            size_bytes,
        });
    }

    Ok(PreparedAudio {
        applied: true,
        original_size_bytes,
        original_duration_ms,
        chunks,
        _temp_dir: Some(temp_dir),
    })
}

fn probe_duration_ms(audio_path: &Path) -> Result<u64, ChunkPreparationError> {
    let output = Command::new("ffprobe")
        .arg("-v")
        .arg("error")
        .arg("-show_entries")
        .arg("format=duration")
        .arg("-of")
        .arg("default=noprint_wrappers=1:nokey=1")
        .arg(audio_path)
        .output()
        .map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                "dependency_unavailable"
            } else {
                "audio_probe_failed"
            };
            ChunkPreparationError::new(code, format!("start ffprobe failed: {error}"), false)
        })?;
    ensure_command_succeeded("ffprobe", &output)?;
    let raw = String::from_utf8(output.stdout).map_err(|error| {
        ChunkPreparationError::new(
            "audio_probe_failed",
            format!("ffprobe duration was not UTF-8: {error}"),
            false,
        )
    })?;
    let seconds = raw.trim().parse::<f64>().map_err(|error| {
        ChunkPreparationError::new(
            "audio_probe_failed",
            format!("ffprobe duration was not numeric: {error}"),
            false,
        )
    })?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err(ChunkPreparationError::new(
            "audio_probe_failed",
            "ffprobe duration was outside the supported range",
            false,
        ));
    }
    Ok((seconds * 1_000.0).round() as u64)
}

fn ensure_command_succeeded(
    command: &'static str,
    output: &Output,
) -> Result<(), ChunkPreparationError> {
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.chars().take(1_000).collect::<String>();
    Err(ChunkPreparationError::new(
        if command == "ffprobe" {
            "audio_probe_failed"
        } else {
            "audio_split_failed"
        },
        format!("{command} failed with status={}: {detail}", output.status),
        false,
    ))
}

fn create_private_temp_directory() -> Result<TemporaryAudioDirectory, ChunkPreparationError> {
    let base = std::env::temp_dir();
    for _ in 0..16 {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!(
            "agent-audio-chunks-{}-{timestamp}-{sequence}",
            std::process::id()
        ));
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        match builder.create(&path) {
            Ok(()) => return Ok(TemporaryAudioDirectory { path }),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(ChunkPreparationError::new(
                    "audio_split_failed",
                    format!("create private audio chunk directory failed: {error}"),
                    false,
                ))
            }
        }
    }
    Err(ChunkPreparationError::new(
        "audio_split_failed",
        "could not allocate a private audio chunk directory",
        true,
    ))
}

#[cfg(test)]
#[path = "audio_chunks_tests.rs"]
mod tests;
