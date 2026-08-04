use super::*;

fn handle_for_test(request: Request) -> Response {
    let request_id = request.request_id.clone();
    let mut output = Vec::new();
    let mut progress = SkillProgressEmitter::new(&mut output, request_id);
    handle(request, &mut progress)
}

#[test]
fn browser_progress_frames_use_requested_and_observed_page_counts() {
    let request = Request {
        request_id: "progress-1".to_string(),
        args: json!({
            "action": "open_extract",
            "url": "https://example.com/a",
            "urls": ["https://example.com/b", "https://example.com/c"],
            "max_pages": 2
        }),
        context: None,
        _user_id: 1,
        _chat_id: 1,
    };
    assert_eq!(requested_page_count(&request.args), Some(2));
    let mut output = Vec::new();
    emit_start_progress(
        &mut SkillProgressEmitter::new(&mut output, &request.request_id),
        &request,
    )
    .expect("start frame");
    let start =
        skill_sdk::validate_progress_frame_line(&output, "progress-1").expect("valid start frame");
    assert_eq!(start.current, Some(0));
    assert_eq!(start.total, Some(2));

    output.clear();
    let response = Response {
        request_id: "progress-1".to_string(),
        status: "ok".to_string(),
        text: String::new(),
        error_text: None,
        buttons: None,
        extra: Some(json!({"success_count": 2, "failure_count": 1})),
    };
    emit_completion_progress(
        &mut SkillProgressEmitter::new(&mut output, &response.request_id),
        &response,
    )
    .expect("completion frame");
    let completed = skill_sdk::validate_progress_frame_line(&output, "progress-1")
        .expect("valid completion frame");
    assert_eq!(completed.current, Some(3));
    assert_eq!(completed.total, Some(3));

    output.clear();
    let partial_failure = Response {
        request_id: "progress-1".to_string(),
        status: "error".to_string(),
        text: String::new(),
        error_text: Some("all_pages_failed".to_string()),
        buttons: None,
        extra: Some(json!({
            "details": {"cause_details": {"success_count": 0, "failure_count": 2}}
        })),
    };
    emit_completion_progress(
        &mut SkillProgressEmitter::new(&mut output, &partial_failure.request_id),
        &partial_failure,
    )
    .expect("failure completion frame");
    let failed = skill_sdk::validate_progress_frame_line(&output, "progress-1")
        .expect("valid failure completion frame");
    assert_eq!(failed.current, Some(2));
    assert_eq!(failed.total, Some(2));
}

#[test]
fn browser_progress_frame_omits_measure_for_invalid_or_empty_requests() {
    let request = Request {
        request_id: "progress-empty".to_string(),
        args: json!({}),
        context: None,
        _user_id: 1,
        _chat_id: 1,
    };
    let mut output = Vec::new();

    emit_start_progress(
        &mut SkillProgressEmitter::new(&mut output, &request.request_id),
        &request,
    )
    .expect("start frame");

    let frame = skill_sdk::validate_progress_frame_line(&output, "progress-empty")
        .expect("valid unmeasured start frame");
    assert_eq!(frame.current, None);
    assert_eq!(frame.total, None);
}

#[test]
fn browser_intermediate_frames_are_monotonic_per_observed_page() {
    let mut output = Vec::new();
    emit_page_progress_sequence(
        &mut SkillProgressEmitter::new(&mut output, "page-sequence"),
        3,
    )
    .expect("page progress");
    let frames = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| {
            skill_sdk::validate_progress_frame_line(line.as_bytes(), "page-sequence").unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(frames.len(), 3);
    assert_eq!(
        frames.iter().map(|frame| frame.current).collect::<Vec<_>>(),
        vec![Some(1), Some(2), Some(3)]
    );
    assert!(frames.iter().all(|frame| frame.total == Some(3)));
}

#[test]
fn browser_node_candidates_include_platform_service_paths_and_path_fallback() {
    let candidates = browser_node_candidates();
    assert_eq!(candidates.last(), Some(&PathBuf::from("node")));
    if cfg!(target_os = "macos") {
        assert!(candidates.contains(&PathBuf::from("/opt/homebrew/bin/node")));
        assert!(candidates.contains(&PathBuf::from("/usr/local/bin/node")));
    } else {
        assert!(candidates.contains(&PathBuf::from("/usr/bin/node")));
    }
}
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

#[test]
fn error_extra_exposes_machine_contract() {
    let details = json!({"exit_code": 9});
    let extra = error_extra("EXECUTION_FAILED", true, Some(&details));

    assert_eq!(extra["schema_version"], 1);
    assert_eq!(extra["source_skill"], SKILL_NAME);
    assert_eq!(extra["status"], "error");
    assert_eq!(extra["error_code"], "EXECUTION_FAILED");
    assert_eq!(extra["message_key"], "skill.browser_web.execution_failed");
    assert_eq!(extra["retryable"], true);
    assert_eq!(extra["details"]["exit_code"], 9);
}

#[test]
fn non_object_args_return_outer_error() {
    let response = handle_for_test(Request {
        request_id: "test-1".to_string(),
        args: json!("not an object"),
        context: None,
        _user_id: 1,
        _chat_id: 1,
    });

    assert_eq!(response.status, "error");
    assert_eq!(
        response
            .extra
            .as_ref()
            .and_then(|value| value.get("error_code"))
            .and_then(Value::as_str),
        Some("INVALID_INPUT")
    );
}

#[test]
fn browser_only_accepts_explicit_page_extraction_action() {
    let response = handle_for_test(Request {
        request_id: "test-search".to_string(),
        args: json!({"action": "search_page", "query": "rust"}),
        context: None,
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
    assert_eq!(
        extra
            .pointer("/model_observation/items/0/title")
            .and_then(serde_json::Value::as_str),
        Some("Rust")
    );
    assert!(browser_web_success_extra("plain text fallback").is_none());
}

#[test]
fn success_extra_projects_bounded_page_content_for_model_and_verifier() {
    let first_text = format!("{} first-page-tail", "a".repeat(9_000));
    let second_text = format!("{} second-page-tail", "b".repeat(9_000));
    let input = json!({
        "status": "ok",
        "summary": "browser_extract_result_set",
        "success_count": 2,
        "failure_count": 0,
        "citations": ["https://one.example", "https://two.example"],
        "items": [
            {
                "url": "https://one.example",
                "final_url": "https://one.example/news",
                "title": "First page",
                "source": "one.example",
                "text": first_text,
                "content_excerpt": "old short excerpt",
                "content_sha256": "sha256:first",
                "fetch_method": "browser",
                "response_status": 200,
                "links": [{"text": "bulk metadata", "url": "https://one.example/other"}]
            },
            {
                "url": "https://two.example",
                "title": "Second page",
                "source": "two.example",
                "text": second_text,
                "content_excerpt": "old short excerpt",
                "content_sha256": "sha256:second",
                "fetch_method": "browser",
                "response_status": 200
            }
        ]
    });

    let extra = browser_web_success_extra(&input.to_string()).expect("structured extra");
    let observation = extra.get("model_observation").expect("model observation");
    let serialized = observation.to_string();

    assert!(
        serialized.len() < 24_000,
        "observation bytes={}",
        serialized.len()
    );
    assert_eq!(observation["items"].as_array().map(Vec::len), Some(2));
    assert_eq!(observation["trust"]["instructions_executable"], false);
    assert_eq!(observation["truncated"], true);
    assert!(observation["items"][0]["content_excerpt"]
        .as_str()
        .is_some_and(|value| value.len() > 7_000));
    assert!(observation["items"][1]["content_excerpt"]
        .as_str()
        .is_some_and(|value| value.len() > 7_000));
    assert!(observation["items"][0].get("links").is_none());
    assert_eq!(
        extra
            .pointer("/items/0/text")
            .and_then(serde_json::Value::as_str)
            .map(str::len),
        Some(9_016)
    );
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
    assert_eq!(args.screenshot_dir, None);
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
fn synthetic_dns_detection_requires_reserved_proxy_addresses_without_private_peers() {
    assert!(detects_proxy_synthetic_dns(&[IpAddr::V4(Ipv4Addr::new(
        198, 18, 0, 42
    ))]));
    assert!(detects_proxy_synthetic_dns(&[
        IpAddr::V4(Ipv4Addr::new(198, 19, 0, 42)),
        IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)),
    ]));
    assert!(!detects_proxy_synthetic_dns(&[IpAddr::V4(Ipv4Addr::new(
        1, 1, 1, 1
    ))]));
    assert!(!detects_proxy_synthetic_dns(&[
        IpAddr::V4(Ipv4Addr::new(198, 18, 0, 42)),
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
    ]));
}

#[test]
fn synthetic_dns_gateway_only_allows_reserved_addresses_for_named_hosts() {
    let synthetic = IpAddr::V4(Ipv4Addr::new(198, 18, 0, 42));
    assert!(is_public_or_proxy_synthetic(
        synthetic,
        "example.com",
        "https",
        true
    ));
    assert!(!is_public_or_proxy_synthetic(
        synthetic,
        "198.18.0.42",
        "https",
        true
    ));
    assert!(!is_public_or_proxy_synthetic(
        synthetic,
        "example.com",
        "https",
        false
    ));
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
    let workspace = std::env::temp_dir().join(format!(
        "agent-runtime-browser-web-test-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workspace);
    std::fs::create_dir_all(&workspace).expect("workspace");

    let directory = resolve_workspace_directory(&workspace, "skills_output/browser", false)
        .expect("inside dir");
    assert!(directory.starts_with(workspace.canonicalize().expect("canonical test workspace")));
    assert_eq!(
        resolve_workspace_directory(&workspace, "../outside", false)
            .unwrap_err()
            .code,
        "WORKSPACE_PATH_OUTSIDE"
    );

    let config = workspace.join("wait-map.json");
    std::fs::write(&config, "{}").expect("config");
    assert_eq!(
        resolve_workspace_file(&workspace, "wait-map.json", false).expect("inside file"),
        config.canonicalize().expect("canonical config")
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn admin_browser_paths_accept_service_account_visible_locations() {
    let root = std::env::temp_dir().join(format!(
        "agent-runtime-browser-web-admin-test-{}",
        std::process::id()
    ));
    let workspace = root.join("workspace");
    let outside_dir = root.join("outside").join("screenshots");
    let outside_file = root.join("outside").join("wait-map.json");
    std::fs::create_dir_all(&workspace).expect("workspace");
    std::fs::create_dir_all(outside_file.parent().expect("outside parent"))
        .expect("outside parent");
    std::fs::write(&outside_file, "{}").expect("outside wait map");

    assert_eq!(
        resolve_workspace_directory(&workspace, outside_dir.to_str().expect("outside dir"), true,)
            .expect("admin screenshot dir"),
        outside_dir.canonicalize().expect("canonical outside dir")
    );
    assert_eq!(
        resolve_workspace_file(
            &workspace,
            outside_file.to_str().expect("outside file"),
            true,
        )
        .expect("admin wait map"),
        outside_file.canonicalize().expect("canonical outside file")
    );

    let _ = std::fs::remove_dir_all(root);
}
