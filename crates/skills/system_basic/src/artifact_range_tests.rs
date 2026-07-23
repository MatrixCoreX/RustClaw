use super::*;

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "rustclaw-system-artifact-{name}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(root.join(".rustclaw/artifacts/skill-output"))
            .expect("create artifact root");
        Self { root }
    }

    fn artifact(&self, name: &str) -> PathBuf {
        self.root
            .join(".rustclaw/artifacts/skill-output")
            .join(name)
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn text_artifact_pages_resume_at_exact_utf8_boundaries() {
    let workspace = TestWorkspace::new("text-pages");
    let path = workspace.artifact("output.txt");
    let source = "alpha-中文-omega\n".repeat(80);
    std::fs::write(&path, &source).expect("write artifact");

    let first = read_artifact_range(
        &workspace.root,
        &json!({"path": path, "max_bytes": 256})
            .as_object()
            .unwrap()
            .clone(),
    )
    .expect("first page");
    let first: Value = serde_json::from_str(&first).unwrap();
    let cursor = first["page"]["next_cursor"].as_u64().unwrap();
    let second = read_artifact_range(
        &workspace.root,
        &json!({"path": path, "start_byte": cursor, "max_bytes": 256})
            .as_object()
            .unwrap()
            .clone(),
    )
    .expect("second page");
    let second: Value = serde_json::from_str(&second).unwrap();

    assert_eq!(first["encoding"], "utf-8");
    assert_eq!(first["page"]["end_byte"], cursor);
    assert_eq!(second["page"]["start_byte"], cursor);
    assert_eq!(first["sha256"], second["sha256"]);
    let joined = format!(
        "{}{}",
        first["content"].as_str().unwrap(),
        second["content"].as_str().unwrap()
    );
    let second_end = second["page"]["end_byte"].as_u64().unwrap() as usize;
    assert_eq!(joined.as_bytes(), &source.as_bytes()[..second_end]);
}

#[test]
fn binary_artifact_returns_base64_without_loss() {
    let workspace = TestWorkspace::new("binary");
    let path = workspace.artifact("output.bin");
    let source = vec![0xff, 0x00, 0x81, 0x82, 0x83];
    std::fs::write(&path, &source).expect("write artifact");

    let output = read_artifact_range(
        &workspace.root,
        &json!({"path": path, "max_bytes": 256})
            .as_object()
            .unwrap()
            .clone(),
    )
    .expect("read artifact");
    let output: Value = serde_json::from_str(&output).unwrap();

    assert_eq!(output["encoding"], "base64");
    assert_eq!(output["binary"], true);
    assert_eq!(
        BASE64_STANDARD
            .decode(output["content"].as_str().unwrap())
            .unwrap(),
        source
    );
}

#[test]
fn artifact_reader_rejects_regular_workspace_files() {
    let workspace = TestWorkspace::new("fence");
    let path = workspace.root.join("README.md");
    std::fs::write(&path, "outside artifact root").expect("write regular file");

    let error = read_artifact_range(
        &workspace.root,
        &json!({"path": path}).as_object().unwrap().clone(),
    )
    .expect_err("regular workspace file must be rejected");

    assert_eq!(error.kind, "path_denied");
}
