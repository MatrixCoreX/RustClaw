use super::*;

#[test]
fn inline_text_is_complete_and_reports_sizes() {
    let result = BoundedResult::text("hello", 32, None, "answer").expect("inline result");
    assert!(result.complete);
    assert_eq!(result.value, "hello");
    assert_eq!(result.original_size_bytes, Some(5));
    assert_eq!(result.returned_size_bytes, Some(5));
    assert!(result.continuation.is_none());
}

#[test]
fn oversized_text_spills_complete_bytes_and_returns_a_range() {
    let root = tempfile::tempdir().expect("temp storage");
    let spill = ArtifactSpill::new(root.path(), "log_analyze").expect("spill root");
    let source = "日志 evidence ".repeat(200);
    let result = BoundedResult::text(&source, 64, Some(&spill), "analysis")
        .expect("bounded artifact result");

    assert!(!result.complete);
    assert_eq!(
        result.partial_reason.as_deref(),
        Some("inline_protocol_budget")
    );
    assert_eq!(result.original_size_bytes, Some(source.len() as u64));
    assert_eq!(result.artifacts.len(), 1);
    assert_eq!(
        std::fs::read(&result.artifacts[0].path).expect("artifact bytes"),
        source.as_bytes()
    );
    assert_eq!(
        result
            .continuation
            .as_ref()
            .map(|value| value.kind.as_str()),
        Some("artifact_range")
    );
}

#[test]
fn oversized_text_without_recovery_never_silently_truncates() {
    let error = BoundedResult::text(&"x".repeat(100), 10, None, "answer")
        .expect_err("recovery is mandatory");
    assert_eq!(error.code, "bounded_result_recovery_unavailable");
}
