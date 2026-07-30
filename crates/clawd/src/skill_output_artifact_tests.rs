use std::fs;

use super::*;

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("skillctl-output-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).expect("create workspace");
        Self { root }
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn small_output_remains_inline() {
    let workspace = TestWorkspace::new();
    let mut text = "small output".to_string();
    let mut extra = None;

    assert!(
        !spill_skill_text_if_needed(&workspace.root, "task", "skill", &mut text, &mut extra,)
            .expect("spill decision")
    );
    assert_eq!(text, "small output");
    assert!(extra.is_none());
    assert!(!workspace.root.join(".agent-runtime").exists());
}

#[test]
fn large_text_output_is_preserved_with_exact_resume_metadata() {
    let workspace = TestWorkspace::new();
    let original = "prefix-中文-content\n".repeat(3000);
    let mut text = original.clone();
    let mut extra = Some(json!({"existing": true}));

    assert!(spill_skill_text_if_needed(
        &workspace.root,
        "task:unsafe/path",
        "custom skill",
        &mut text,
        &mut extra,
    )
    .expect("spill output"));

    let extra = extra.expect("extra");
    let artifact = &extra["artifact_refs"][0];
    let artifact_path = workspace
        .root
        .join(artifact["path"].as_str().expect("artifact path"));
    assert_eq!(fs::read_to_string(&artifact_path).unwrap(), original);
    assert!(artifact_path.starts_with(
        workspace
            .root
            .join(".agent-runtime/artifacts/skill-output/task-unsafe-path")
    ));
    assert_eq!(extra["existing"], true);
    assert_eq!(extra["truncated"], true);
    assert_eq!(
        extra["range_handles"][0]["read_capability"],
        "artifact.read_range"
    );
    assert_eq!(
        extra["page"]["next_cursor"].as_u64().unwrap() as usize,
        text.len()
    );
    assert!(original.starts_with(&text));
}

#[test]
fn large_json_output_keeps_inline_projection_valid_json() {
    let workspace = TestWorkspace::new();
    let original = json!({"rows": vec!["value"; 10000]}).to_string();
    let mut text = original.clone();
    let mut extra = None;

    spill_skill_text_if_needed(&workspace.root, "task", "json_skill", &mut text, &mut extra)
        .expect("spill output");

    let projection: Value = serde_json::from_str(&text).expect("valid JSON projection");
    let extra = extra.expect("extra");
    assert_eq!(projection["status_code"], "output_truncated");
    assert_eq!(projection["next_cursor"], extra["page"]["next_cursor"]);
    assert_eq!(projection["total_bytes"], original.len());
    assert_eq!(
        fs::read_to_string(
            workspace
                .root
                .join(extra["artifacts"][0]["path"].as_str().unwrap())
        )
        .unwrap(),
        original
    );
}

#[test]
fn existing_async_output_is_published_without_losing_source() {
    let workspace = TestWorkspace::new();
    let source_dir = workspace.root.join(".agent-runtime/async_jobs/job");
    fs::create_dir_all(&source_dir).unwrap();
    let source = source_dir.join("stdout");
    let content = "async output\n".repeat(5000);
    fs::write(&source, &content).unwrap();

    let published = publish_existing_task_artifact(
        &workspace.root,
        "task:async",
        "async-process",
        &source,
        "stdout.log",
        "text/plain; charset=utf-8",
        json!({"stream": "stdout", "job_id": "local_process:test"}),
    )
    .expect("publish")
    .expect("non-empty artifact");

    let published_path = workspace
        .root
        .join(published.artifact_ref["path"].as_str().unwrap());
    assert_eq!(fs::read_to_string(published_path).unwrap(), content);
    assert_eq!(fs::read_to_string(source).unwrap(), content);
    assert_eq!(
        published.range_handle["read_capability"],
        "artifact.read_range"
    );
    assert_eq!(
        published.artifact_ref["metadata"]["stream"],
        Value::String("stdout".to_string())
    );
}

#[test]
fn sensitive_json_keeps_hashes_but_never_exposes_secret_values_to_the_model() {
    let source = json!({
        "endpoint": "https://example.com",
        "api_key": "sk-secret-value",
        "nested": {"authorization": "Bearer private-token", "count": 3},
    });
    let (model_view, redacted) = sensitivity_aware_json_model_view(&source);

    assert!(redacted);
    assert_eq!(model_view["endpoint"], "https://example.com");
    assert_eq!(model_view["nested"]["count"], 3);
    assert_eq!(model_view["api_key"]["redacted"], true);
    assert!(!model_view.to_string().contains("sk-secret-value"));
    assert!(!model_view.to_string().contains("private-token"));
}

#[test]
fn sensitive_text_redacts_assignments_bearer_tokens_and_private_keys() {
    let source = "api_key=secret\nAuthorization: Bearer token\n-----BEGIN PRIVATE KEY-----\nbody\n-----END PRIVATE KEY-----\nsafe=value\n";
    let (model_view, redacted) = sensitivity_aware_text_model_view(source);

    assert!(redacted);
    assert!(model_view.contains("api_key=[REDACTED sha256:"));
    assert!(!model_view.contains("secret"));
    assert!(!model_view.contains("Bearer token"));
    assert!(!model_view.contains("body"));
    assert!(model_view.contains("safe=value"));
}
