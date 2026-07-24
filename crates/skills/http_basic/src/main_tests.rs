use super::{
    bounded_preview, domain_matches, error_extra, execute, host_matches_no_proxy, http_observation,
    is_proxy_synthetic_ip, is_public_ip, is_sensitive_header, is_textual_content, read_limited,
    redirect_switches_to_get, resolve_output_path, should_forward_header,
    should_inject_rustclaw_key_for_base, validate_target_url, FetchPolicy, HttpArtifact,
    HttpObservationInput, RequestMethod, SKILL_NAME,
};
use reqwest::StatusCode;
use serde_json::json;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

fn test_policy() -> FetchPolicy {
    FetchPolicy {
        timeout: Duration::from_secs(1),
        max_response_bytes: 1024,
        max_redirects: 3,
        domains_allow: Vec::new(),
        domains_deny: Vec::new(),
    }
}

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
    let (text, extra) = http_observation(HttpObservationInput {
        action: "get",
        requested_url: "http://127.0.0.1:8787/missing",
        final_url: "http://127.0.0.1:8787/missing",
        status: 404,
        success_status: false,
        content_type: Some("text/plain"),
        size_bytes: 9,
        body_sha256: "sha256:test",
        redirects: Vec::new(),
        network_route: "direct",
        preview: "not found",
        preview_truncated: false,
        artifact: None,
    });

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
        sha256: "sha256:artifact".to_string(),
    };
    let (text, extra) = http_observation(HttpObservationInput {
        action: "get",
        requested_url: "https://example.com/data.json",
        final_url: "https://example.com/data.json",
        status: 200,
        success_status: true,
        content_type: Some("application/json"),
        size_bytes: 11,
        body_sha256: "sha256:body",
        redirects: vec![json!({
            "status_code": 302,
            "from": "https://example.com/old",
            "to": "https://example.com/data.json"
        })],
        network_route: "trusted_egress_proxy",
        preview: "{\"ok\":true}",
        preview_truncated: false,
        artifact: Some(&artifact),
    });

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
    assert_eq!(
        extra.get("artifact_sha256").and_then(|v| v.as_str()),
        Some("sha256:artifact")
    );
    assert_eq!(
        extra
            .pointer("/trust/classification")
            .and_then(|v| v.as_str()),
        Some("untrusted_external_content")
    );
    assert_eq!(
        extra.get("redirect_count").and_then(|v| v.as_u64()),
        Some(1)
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
    assert_eq!(err.code, "output_path_outside_workspace");

    let traversal = resolve_output_path(
        &workspace,
        "document/http/download",
        Some("../outside.body"),
    )
    .expect_err("parent traversal should be rejected");
    assert_eq!(traversal.code, "output_path_outside_workspace");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn bounded_preview_truncates_by_char_boundary() {
    assert_eq!(bounded_preview("你好世界", 2), "你好");
}

#[test]
fn observe_get_cannot_smuggle_a_workspace_write() {
    let error = execute(
        json!({
            "action": "get",
            "url": "https://example.com/file.bin",
            "download": true
        }),
        None,
    )
    .expect_err("get must remain read-only");

    assert_eq!(error.code, "download_action_required");
}

#[test]
fn private_and_reserved_network_ranges_are_blocked() {
    for ip in [
        IpAddr::V4(Ipv4Addr::LOCALHOST),
        IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
        IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1)),
        IpAddr::V4(Ipv4Addr::new(192, 0, 2, 1)),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
        IpAddr::V6("fc00::1".parse().expect("unique-local address")),
        IpAddr::V6("fe80::1".parse().expect("link-local address")),
        IpAddr::V6("2001:db8::1".parse().expect("documentation address")),
    ] {
        assert!(!is_public_ip(ip), "{ip} must be blocked");
    }
    assert!(is_public_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    assert!(is_public_ip(IpAddr::V6(
        "2606:4700:4700::1111".parse().expect("public IPv6 address")
    )));
    assert!(is_proxy_synthetic_ip(IpAddr::V4(Ipv4Addr::new(
        198, 18, 0, 1
    ))));
    assert!(is_proxy_synthetic_ip(IpAddr::V4(Ipv4Addr::new(
        198, 19, 255, 254
    ))));
    assert!(!is_proxy_synthetic_ip(IpAddr::V4(Ipv4Addr::new(
        192, 168, 1, 1
    ))));
}

#[test]
fn local_rustclaw_endpoint_requires_an_authenticated_key_context() {
    let policy = test_policy();
    let blocked = validate_target_url("http://127.0.0.1:8787/v1/tasks", &policy, false)
        .expect_err("unauthenticated local fetch must fail");
    assert_eq!(blocked.code, "private_network_blocked");

    let allowed = validate_target_url("http://127.0.0.1:8787/v1/tasks", &policy, true)
        .expect("authenticated RustClaw endpoint");
    assert!(allowed.rustclaw_local);

    let other_port = validate_target_url("http://127.0.0.1:8788/", &policy, true)
        .expect_err("other local services remain blocked");
    assert_eq!(other_port.code, "private_network_blocked");
}

#[test]
fn local_rustclaw_endpoint_uses_the_configured_loopback_origin() {
    assert!(should_inject_rustclaw_key_for_base(
        "http://127.0.0.1:49557/v1/health",
        Some("http://localhost:49557")
    ));
    assert!(should_inject_rustclaw_key_for_base(
        "http://[::1]:49557/v1/health",
        Some("http://127.0.0.1:49557")
    ));
    assert!(!should_inject_rustclaw_key_for_base(
        "http://127.0.0.1:49558/v1/health",
        Some("http://127.0.0.1:49557")
    ));
    assert!(!should_inject_rustclaw_key_for_base(
        "http://127.0.0.1:49557/v1/health",
        Some("https://127.0.0.1:49557")
    ));
    assert!(!should_inject_rustclaw_key_for_base(
        "http://127.0.0.1:49557/v1/health",
        Some("http://example.com:49557")
    ));
}

#[test]
fn domain_policy_matches_only_exact_hosts_and_subdomains() {
    assert!(domain_matches("docs.example.com", "example.com"));
    assert!(domain_matches("example.com", "example.com"));
    assert!(!domain_matches("example.com.evil.invalid", "example.com"));
    assert!(!domain_matches("notexample.com", "example.com"));

    let mut policy = test_policy();
    policy.domains_allow = vec!["example.com".to_string()];
    let error = validate_target_url("https://1.1.1.1/", &policy, false)
        .expect_err("domain allowlist must apply before fetch");
    assert_eq!(error.code, "domain_not_allowed");
}

#[test]
fn no_proxy_matching_is_exact_or_suffix_bounded() {
    assert!(host_matches_no_proxy(
        "api.example.com",
        Some("localhost,.example.com,10.0.0.0/8")
    ));
    assert!(host_matches_no_proxy("example.com", Some("example.com")));
    assert!(!host_matches_no_proxy(
        "example.com.evil.invalid",
        Some("example.com")
    ));
    assert!(!host_matches_no_proxy(
        "notexample.com",
        Some("example.com")
    ));
    assert!(host_matches_no_proxy("anything.invalid", Some("*")));
}

#[test]
fn response_reader_fails_loudly_past_the_byte_limit() {
    let mut oversized = std::io::Cursor::new(vec![b'x'; 17]);
    let error = read_limited(&mut oversized, 16).expect_err("oversized response must fail");
    assert_eq!(error.code, "response_too_large");
    assert_eq!(
        error
            .extra
            .as_ref()
            .and_then(|value| value.get("max_response_bytes"))
            .and_then(|value| value.as_u64()),
        Some(16)
    );

    let mut exact = std::io::Cursor::new(vec![b'x'; 16]);
    assert_eq!(
        read_limited(&mut exact, 16)
            .expect("exact byte limit")
            .len(),
        16
    );
}

#[test]
fn content_type_policy_distinguishes_text_from_binary() {
    assert!(is_textual_content(
        Some("application/problem+json; charset=utf-8"),
        b"{}"
    ));
    assert!(is_textual_content(Some("text/html"), b"<p>ok</p>"));
    assert!(!is_textual_content(
        Some("application/octet-stream"),
        b"\0\x01"
    ));
    assert!(is_textual_content(None, "plain utf-8".as_bytes()));
    assert!(!is_textual_content(None, b"\xff\xfe"));
}

#[test]
fn redirect_and_credential_rules_are_machine_driven() {
    assert!(redirect_switches_to_get(
        StatusCode::SEE_OTHER,
        RequestMethod::PostJson
    ));
    assert!(redirect_switches_to_get(
        StatusCode::FOUND,
        RequestMethod::PostJson
    ));
    assert!(!redirect_switches_to_get(
        StatusCode::TEMPORARY_REDIRECT,
        RequestMethod::PostJson
    ));
    assert!(is_sensitive_header("Authorization"));
    assert!(is_sensitive_header("X-API-Key"));
    assert!(!is_sensitive_header("Accept"));
    assert!(should_forward_header("Authorization", true));
    assert!(!should_forward_header("Authorization", false));
    assert!(should_forward_header("Accept", false));
    assert!(!should_forward_header("X-Custom-Credential", false));
}
