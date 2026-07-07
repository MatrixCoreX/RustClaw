use super::{
    bounded_preview, error_extra, http_observation, resolve_output_path, HttpArtifact, SKILL_NAME,
};

#[test]
fn error_extra_exposes_machine_contract() {
    let extra = error_extra("execution_failed");

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "execution_failed");
    assert_eq!(extra["message_key"], "skill.http_basic.execution_failed");
    assert_eq!(extra["retryable"], false);
}

#[test]
fn http_non_success_response_is_structured_observation() {
    let (text, extra) = http_observation(
        "get",
        "http://127.0.0.1:8787/missing",
        404,
        false,
        "not found",
        None,
    );

    assert_eq!(text, "status=404\nnot found");
    assert_eq!(extra.get("status_code").and_then(|v| v.as_u64()), Some(404));
    assert_eq!(
        extra.get("success_status").and_then(|v| v.as_bool()),
        Some(false)
    );
    assert_eq!(
        extra.get("body_preview").and_then(|v| v.as_str()),
        Some("not found")
    );
}

#[test]
fn http_download_observation_exposes_artifact_fields() {
    let artifact = HttpArtifact {
        output_path: "document/http/download/example.json".to_string(),
        size_bytes: 12,
        content_type: Some("application/json".to_string()),
    };
    let (text, extra) = http_observation(
        "get",
        "https://example.com/data.json",
        200,
        true,
        "{\"ok\":true}",
        Some(&artifact),
    );

    assert!(text.contains("status=200"));
    assert!(text.contains("output_path=document/http/download/example.json"));
    assert_eq!(
        extra.get("downloaded").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        extra.get("output_path").and_then(|v| v.as_str()),
        Some("document/http/download/example.json")
    );
    assert_eq!(
        extra.get("artifact_path").and_then(|v| v.as_str()),
        Some("document/http/download/example.json")
    );
    assert_eq!(extra.get("size_bytes").and_then(|v| v.as_u64()), Some(12));
    assert_eq!(
        extra.get("content_type").and_then(|v| v.as_str()),
        Some("application/json")
    );
}

#[test]
fn http_download_output_path_must_stay_inside_workspace() {
    let workspace =
        std::env::temp_dir().join(format!("rustclaw-http-basic-test-{}", std::process::id()));
    std::fs::create_dir_all(&workspace).expect("create temp workspace");

    let inside = resolve_output_path(
        &workspace,
        "document/http/download",
        Some("document/http/download/out.body"),
    )
    .expect("inside workspace path");
    assert!(inside.starts_with(&workspace));

    let outside = workspace
        .parent()
        .expect("temp workspace parent")
        .join("outside.body");
    let err = resolve_output_path(&workspace, "document/http/download", outside.to_str())
        .expect_err("outside path should be rejected");
    assert_eq!(err, "output_path is outside workspace");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn bounded_preview_truncates_by_char_boundary() {
    assert_eq!(bounded_preview("你好世界", 2), "你好");
}
