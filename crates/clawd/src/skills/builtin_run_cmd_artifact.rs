use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

const ARTIFACT_ROOT: &str = ".rustclaw";
const ARTIFACT_DIR: &str = "artifacts/tool-output";
const MIN_HARD_LIMIT_BYTES: usize = 8 * 1024 * 1024;
const MAX_HARD_LIMIT_BYTES: usize = 64 * 1024 * 1024;
const HARD_LIMIT_MULTIPLIER: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputStream {
    Stdout,
    Stderr,
}

impl OutputStream {
    fn as_str(self) -> &'static str {
        match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CommandOutputArtifactSummary {
    pub(super) total_bytes: usize,
    pub(super) excerpt_bytes: usize,
    pub(super) artifact_refs: Vec<Value>,
}

impl CommandOutputArtifactSummary {
    pub(super) fn machine_projection(&self, excerpt: &str, exit_code: i32) -> Value {
        let range_handles = self
            .artifact_refs
            .iter()
            .filter_map(|artifact| {
                Some(json!({
                    "artifact_ref": artifact.get("id")?.as_str()?,
                    "path": artifact.get("path")?.as_str()?,
                    "start_byte": 0,
                    "end_byte": artifact.pointer("/metadata/size_bytes")?.as_u64()?,
                    "read_capability": "artifact.read_range",
                }))
            })
            .collect::<Vec<_>>();
        json!({
            "schema_version": 1,
            "kind": "tool_output_artifact",
            "status_code": "ok",
            "exit_code": exit_code,
            "summary": {
                "excerpt": excerpt,
                "output_truncated": true,
                "excerpt_bytes": self.excerpt_bytes,
                "total_bytes": self.total_bytes,
            },
            "artifact_refs": self.artifact_refs,
            "range_handles": range_handles,
        })
    }
}

pub(super) struct CommandOutputArtifactWriter {
    workspace_root: PathBuf,
    artifact_dir: PathBuf,
    artifact_id: String,
    stdout_path: PathBuf,
    stderr_path: PathBuf,
    stdout_file: Option<File>,
    stderr_file: Option<File>,
    total_bytes: usize,
    excerpt_bytes: usize,
    excerpt_limit_bytes: usize,
    hard_limit_bytes: usize,
    activated: bool,
}

impl CommandOutputArtifactWriter {
    pub(super) fn new(workspace_root: &Path, task_id: &str, excerpt_limit_bytes: usize) -> Self {
        let task_key = machine_path_component(task_id);
        let artifact_id = uuid::Uuid::new_v4().to_string();
        let artifact_dir = workspace_root
            .join(ARTIFACT_ROOT)
            .join(ARTIFACT_DIR)
            .join(task_key);
        let stdout_path = artifact_dir.join(format!("{artifact_id}.stdout.log"));
        let stderr_path = artifact_dir.join(format!("{artifact_id}.stderr.log"));
        let hard_limit_bytes = excerpt_limit_bytes
            .saturating_mul(HARD_LIMIT_MULTIPLIER)
            .clamp(MIN_HARD_LIMIT_BYTES, MAX_HARD_LIMIT_BYTES);
        Self {
            workspace_root: workspace_root.to_path_buf(),
            artifact_dir,
            artifact_id,
            stdout_path,
            stderr_path,
            stdout_file: None,
            stderr_file: None,
            total_bytes: 0,
            excerpt_bytes: 0,
            excerpt_limit_bytes,
            hard_limit_bytes,
            activated: false,
        }
    }

    pub(super) fn append(
        &mut self,
        stream: OutputStream,
        bytes: &[u8],
        stdout_excerpt: &mut Vec<u8>,
        stderr_excerpt: &mut Vec<u8>,
    ) -> io::Result<bool> {
        if bytes.is_empty() {
            return Ok(false);
        }
        self.total_bytes = self.total_bytes.saturating_add(bytes.len());
        if self.total_bytes > self.hard_limit_bytes {
            return Ok(true);
        }
        if self.activated {
            self.write_stream(stream, bytes)?;
            return Ok(false);
        }

        let remaining = self.excerpt_limit_bytes.saturating_sub(self.excerpt_bytes);
        let take = bytes.len().min(remaining);
        if take > 0 {
            match stream {
                OutputStream::Stdout => stdout_excerpt.extend_from_slice(&bytes[..take]),
                OutputStream::Stderr => stderr_excerpt.extend_from_slice(&bytes[..take]),
            }
            self.excerpt_bytes = self.excerpt_bytes.saturating_add(take);
        }
        if take < bytes.len() {
            self.activate(stdout_excerpt, stderr_excerpt)?;
            self.write_stream(stream, &bytes[take..])?;
        }
        Ok(false)
    }

    pub(super) fn finish(mut self) -> io::Result<Option<CommandOutputArtifactSummary>> {
        if !self.activated {
            return Ok(None);
        }
        if let Some(file) = self.stdout_file.as_mut() {
            file.flush()?;
        }
        if let Some(file) = self.stderr_file.as_mut() {
            file.flush()?;
        }
        drop(self.stdout_file.take());
        drop(self.stderr_file.take());

        let mut refs = Vec::new();
        for (stream, path) in [
            (OutputStream::Stdout, &self.stdout_path),
            (OutputStream::Stderr, &self.stderr_path),
        ] {
            let metadata = fs::metadata(path)?;
            if metadata.len() == 0 {
                let _ = fs::remove_file(path);
                continue;
            }
            refs.push(json!({
                "id": format!("tool-output:{}:{}", self.artifact_id, stream.as_str()),
                "path": relative_artifact_path(&self.workspace_root, path),
                "media_type": "text/plain",
                "sha256": sha256_file(path)?,
                "metadata": {
                    "size_bytes": metadata.len(),
                    "stream": stream.as_str(),
                    "provenance": "run_cmd",
                },
            }));
        }
        Ok(Some(CommandOutputArtifactSummary {
            total_bytes: self.total_bytes,
            excerpt_bytes: self.excerpt_bytes,
            artifact_refs: refs,
        }))
    }

    fn activate(&mut self, stdout_excerpt: &[u8], stderr_excerpt: &[u8]) -> io::Result<()> {
        fs::create_dir_all(&self.artifact_dir)?;
        let mut stdout_file = File::create(&self.stdout_path)?;
        let mut stderr_file = File::create(&self.stderr_path)?;
        stdout_file.write_all(stdout_excerpt)?;
        stderr_file.write_all(stderr_excerpt)?;
        self.stdout_file = Some(stdout_file);
        self.stderr_file = Some(stderr_file);
        self.activated = true;
        Ok(())
    }

    fn write_stream(&mut self, stream: OutputStream, bytes: &[u8]) -> io::Result<()> {
        match stream {
            OutputStream::Stdout => self
                .stdout_file
                .as_mut()
                .expect("artifact writer activated")
                .write_all(bytes),
            OutputStream::Stderr => self
                .stderr_file
                .as_mut()
                .expect("artifact writer activated")
                .write_all(bytes),
        }
    }
}

fn relative_artifact_path(workspace_root: &Path, path: &Path) -> String {
    path.strip_prefix(workspace_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn machine_path_component(value: &str) -> String {
    let mut out = value
        .trim()
        .chars()
        .take(96)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();
    if out.is_empty() {
        out.push_str("task");
    }
    out
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut hasher = Sha256::new();
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

#[cfg(test)]
#[path = "builtin_run_cmd_artifact_tests.rs"]
mod tests;
