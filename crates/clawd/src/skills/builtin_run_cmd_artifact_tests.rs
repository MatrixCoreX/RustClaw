use std::fs;

use super::*;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("rustclaw-output-artifact-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn large_output_is_streamed_to_artifacts_with_range_handles() {
    let temp = TempDir::new();
    let mut writer = CommandOutputArtifactWriter::new(&temp.path, "task:1", 8);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    assert!(!writer
        .append(
            OutputStream::Stdout,
            b"0123456789",
            &mut stdout,
            &mut stderr
        )
        .expect("append stdout"));
    assert!(!writer
        .append(
            OutputStream::Stderr,
            b"failure-detail",
            &mut stdout,
            &mut stderr
        )
        .expect("append stderr"));
    let summary = writer.finish().expect("finish").expect("artifact summary");
    let projection = summary.machine_projection("01234567\n...", 0);

    assert_eq!(stdout, b"01234567");
    assert_eq!(summary.total_bytes, 24);
    assert_eq!(summary.artifact_refs.len(), 2);
    assert_eq!(
        projection["range_handles"][0]["read_capability"],
        "artifact.read_range"
    );
    for artifact in &summary.artifact_refs {
        let path = temp
            .path
            .join(artifact["path"].as_str().expect("artifact path"));
        assert!(path.is_file());
        assert_eq!(
            artifact["sha256"].as_str().map(str::len),
            Some(64),
            "artifact digest must be complete"
        );
    }
}

#[test]
fn small_output_stays_inline_without_artifact_files() {
    let temp = TempDir::new();
    let mut writer = CommandOutputArtifactWriter::new(&temp.path, "task", 32);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();

    assert!(!writer
        .append(OutputStream::Stdout, b"small", &mut stdout, &mut stderr)
        .expect("append"));

    assert!(writer.finish().expect("finish").is_none());
    assert_eq!(stdout, b"small");
    assert!(!temp.path.join(ARTIFACT_ROOT).exists());
}
