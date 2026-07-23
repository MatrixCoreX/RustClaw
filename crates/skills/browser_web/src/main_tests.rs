use super::*;
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[test]
fn error_extra_exposes_machine_contract() {
    let details = json!({"exit_code": 9});
    let extra = error_extra("EXECUTION_FAILED", true, Some(&details));

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_kind"], "EXECUTION_FAILED");
    assert_eq!(extra["message_key"], "skill.browser_web.execution_failed");
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["details"]["exit_code"], 9);
}

#[test]
fn non_object_args_return_outer_error() {
    let response = handle(Request {
        request_id: "test-1".to_string(),
        args: json!("not an object"),
        _context: None,
        _user_id: 1,
        _chat_id: 1,
    });

    assert_eq!(response.status, "error");
    assert_eq!(
        response
            .extra
            .as_ref()
            .and_then(|value| value.get("error_kind"))
            .and_then(Value::as_str),
        Some("INVALID_INPUT")
    );
}

#[test]
fn browser_only_accepts_explicit_page_extraction_action() {
    let response = handle(Request {
        request_id: "test-search".to_string(),
        args: json!({"action": "search_page", "query": "rust"}),
        _context: None,
        _user_id: 1,
        _chat_id: 1,
    });

    assert_eq!(response.status, "error");
    assert_eq!(response.error_text.as_deref(), Some("unsupported_action"));
    assert_eq!(
        response
            .extra
            .as_ref()
            .and_then(|value| value.get("error_code"))
            .and_then(Value::as_str),
        Some("INVALID_ACTION")
    );
}

#[test]
fn success_extra_preserves_helper_json_and_adds_source_skill() {
    let extra = browser_web_success_extra(
        r#"{"items":[{"title":"Rust","final_url":"https://example.com"}],"citations":["https://example.com"]}"#,
    )
    .expect("structured extra");

    assert_eq!(
        extra
            .get("source_skill")
            .and_then(serde_json::Value::as_str),
        Some("browser_web")
    );
    assert_eq!(
        extra
            .pointer("/items/0/title")
            .and_then(serde_json::Value::as_str),
        Some("Rust")
    );
    assert!(browser_web_success_extra("plain text fallback").is_none());
}

#[test]
fn parses_open_extract_contract_and_domain_policy() {
    let object = json!({
        "action": "open_extract",
        "urls": ["https://example.com", "https://docs.example.com/page"],
        "max_pages": 5,
        "wait_until": "load",
        "content_mode": "raw",
        "max_text_chars": 4096,
        "min_content_chars": 120,
        "fail_fast": true,
        "wait_map_path": "configs/browser_web_wait_map.json",
        "domains_allow": ["example.com"],
        "domains_deny": ["blocked.example.com"]
    })
    .as_object()
    .expect("object")
    .clone();

    let args = parse_open_extract_args(&object).expect("valid args");

    assert_eq!(args.action, "open_extract");
    assert_eq!(args.urls.as_ref().map(Vec::len), Some(2));
    assert_eq!(args.max_pages, Some(5));
    assert_eq!(args.wait_until.as_deref(), Some("load"));
    assert_eq!(args.content_mode.as_deref(), Some("raw"));
    assert_eq!(args.max_text_chars, Some(4096));
    assert_eq!(args.min_content_chars, Some(120));
    assert_eq!(args.fail_fast, Some(true));
    assert_eq!(args.domains_allow, Some(vec!["example.com".to_string()]));
}

#[test]
fn open_extract_requires_urls_and_strict_array_items() {
    let missing = json!({"action": "open_extract"})
        .as_object()
        .expect("object")
        .clone();
    assert!(parse_open_extract_args(&missing).is_err());

    let wrong_item = json!({
        "action": "open_extract",
        "urls": ["https://example.com", 7]
    })
    .as_object()
    .expect("object")
    .clone();
    assert_eq!(
        parse_open_extract_args(&wrong_item).unwrap_err(),
        "urls_items_invalid"
    );
}

#[test]
fn numeric_and_enum_limits_fail_closed() {
    for max_pages in [0, 11] {
        let object = json!({
            "action": "open_extract",
            "url": "https://example.com",
            "max_pages": max_pages
        })
        .as_object()
        .expect("object")
        .clone();
        assert!(parse_open_extract_args(&object).is_err());
    }

    let invalid_mode = json!({
        "action": "open_extract",
        "url": "https://example.com",
        "content_mode": "debug"
    })
    .as_object()
    .expect("object")
    .clone();
    assert!(parse_open_extract_args(&invalid_mode).is_err());
}

#[test]
fn target_policy_blocks_private_credentials_and_domain_escape() {
    for target in [
        "ftp://example.com/file",
        "https://user:secret@example.com/",
        "http://127.0.0.1/",
        "http://169.254.169.254/latest/meta-data/",
        "http://service.local/",
    ] {
        assert!(
            validate_browser_target(target, &[], &[]).is_err(),
            "{target} must be blocked"
        );
    }

    assert_eq!(
        validate_browser_target(
            "https://1.1.1.1/path#fragment",
            &[],
            &["1.1.1.1".to_string()]
        )
        .unwrap_err()
        .code,
        "DOMAIN_BLOCKED"
    );
    assert_eq!(
        validate_browser_target("https://1.1.1.1/path", &["example.com".to_string()], &[])
            .unwrap_err()
            .code,
        "DOMAIN_NOT_ALLOWED"
    );
    assert_eq!(
        validate_browser_target("https://1.1.1.1/path#fragment", &[], &[]).expect("public target"),
        "https://1.1.1.1/path"
    );
}

#[test]
fn reserved_network_ranges_are_not_public() {
    for address in [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(198, 18, 0, 1)),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V6("fc00::1".parse().expect("unique local")),
    ] {
        assert!(!is_public_ip(address));
    }
    assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
}

#[test]
fn workspace_paths_reject_traversal_and_symlink_escape() {
    let workspace =
        std::env::temp_dir().join(format!("rustclaw-browser-web-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).expect("workspace");

    let directory =
        resolve_workspace_directory(&workspace, "skills_output/browser").expect("inside dir");
    assert!(directory.starts_with(workspace.canonicalize().expect("canonical test workspace")));
    assert_eq!(
        resolve_workspace_directory(&workspace, "../outside")
            .unwrap_err()
            .code,
        "WORKSPACE_PATH_OUTSIDE"
    );

    let config = workspace.join("wait-map.json");
    std::fs::write(&config, "{}").expect("config");
    assert_eq!(
        resolve_workspace_file(&workspace, "wait-map.json").expect("inside file"),
        config.canonicalize().expect("canonical config")
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
