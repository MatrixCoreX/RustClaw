use super::*;
use crate::test_support::{docx_fixture, temp_path};

#[test]
fn skill_protocol_returns_structured_docx_evidence() {
    let path = temp_path("docx");
    docx_fixture(&path);
    let response = process_line(
        &json!({
            "request_id": "office-1",
            "args": {"action": "word.read", "path": path}
        })
        .to_string(),
    );
    assert_eq!(response.status, "ok");
    assert_eq!(response.request_id, "office-1");
    assert_eq!(response.extra["format"], "docx");
    std::fs::remove_file(path).ok();
}
