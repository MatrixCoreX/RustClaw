use super::{
    bounded_content_excerpt, normalize_action, parse_doc_extra, Metadata, ParsePayload,
    EXTRA_CONTENT_EXCERPT_CHARS,
};
use serde_json::json;

#[test]
fn normalize_action_accepts_parse_alias() {
    assert_eq!(normalize_action("parse_doc"), Some("parse_doc"));
    assert_eq!(normalize_action("parse"), Some("parse_doc"));
    assert_eq!(normalize_action("unknown"), None);
}

#[test]
fn parse_doc_extra_exposes_path_and_content_excerpt() {
    let req = json!({
        "args": {
            "path": "README.md"
        }
    });
    let payload = ParsePayload {
        text: "RustClaw is a local agent runtime.".to_string(),
        tables: vec![],
        sections: vec![],
        metadata: Some(Metadata {
            title: "RustClaw".to_string(),
            pages: 1,
            doc_type: "md".to_string(),
            path: "/home/guagua/rustclaw/README.md".to_string(),
            encoding: "utf-8-or-lossy".to_string(),
            truncated: false,
            truncation_notice: None,
            page_range_applied: None,
        }),
        status: "ok".to_string(),
        error_code: None,
        error: None,
    };

    let extra = parse_doc_extra(&req, &payload);

    assert_eq!(
        extra.get("path").and_then(|value| value.as_str()),
        Some("/home/guagua/rustclaw/README.md")
    );
    assert_eq!(
        extra.get("requested_path").and_then(|value| value.as_str()),
        Some("README.md")
    );
    assert_eq!(
        extra
            .get("content_excerpt")
            .and_then(|value| value.as_str()),
        Some("RustClaw is a local agent runtime.")
    );
    assert_eq!(
        extra
            .get("content_excerpt_truncated")
            .and_then(|value| value.as_bool()),
        Some(false)
    );
}

#[test]
fn parse_doc_extra_falls_back_to_requested_path_without_metadata() {
    let req = json!({
        "args": {
            "path": "AGENTS.md"
        }
    });
    let payload = ParsePayload {
        text: "Agent development rules".to_string(),
        tables: vec![],
        sections: vec![],
        metadata: None,
        status: "ok".to_string(),
        error_code: None,
        error: None,
    };

    let extra = parse_doc_extra(&req, &payload);

    assert_eq!(
        extra.get("path").and_then(|value| value.as_str()),
        Some("AGENTS.md")
    );
}

#[test]
fn bounded_content_excerpt_limits_long_text_without_suffix() {
    let text = "x".repeat(EXTRA_CONTENT_EXCERPT_CHARS + 5);

    let excerpt = bounded_content_excerpt(&text, EXTRA_CONTENT_EXCERPT_CHARS);

    assert_eq!(excerpt.len(), EXTRA_CONTENT_EXCERPT_CHARS);
    assert!(excerpt.chars().all(|ch| ch == 'x'));
}
