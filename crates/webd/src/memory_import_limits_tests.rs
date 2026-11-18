use super::*;
use axum::http::{HeaderValue, Method};
use claw_core::config::WebdRequestLimitsConfig;
use std::net::{IpAddr, Ipv4Addr};

#[test]
fn only_the_exact_chunk_path_gets_the_existing_upload_policy() {
    let base = "/v1/memory/import/uploads/memory_upload_fixture";
    assert_eq!(
        classify_request(&Method::PUT, &format!("{base}/chunks")),
        RequestClass::Upload
    );
    for (method, path) in [
        (Method::GET, format!("{base}/chunks")),
        (Method::GET, base.to_owned()),
        (Method::DELETE, base.to_owned()),
        (Method::POST, "/v1/memory/import/uploads".to_owned()),
        (Method::GET, "/v1/memory/import/policy".to_owned()),
    ] {
        assert_eq!(classify_request(&method, &path), RequestClass::General);
    }
    assert_eq!(
        classify_request(&Method::POST, &format!("{base}/preview")),
        RequestClass::HighCost
    );
}

#[test]
fn similarly_named_nested_or_encoded_paths_do_not_expand_the_upload_exception() {
    for path in [
        "/v1/memory/import/uploads//chunks",
        "/v1/memory/import/uploads/a/b/chunks",
        "/v1/memory/import/uploads/a/chunks/extra",
        "/v1/memory/import/uploads/a%2Fb/chunks",
        "/v1/memory/import/uploadsx/a/chunks",
    ] {
        assert_eq!(classify_request(&Method::PUT, path), RequestClass::General);
    }
}

#[test]
fn chunk_class_uses_configured_upload_bytes_not_a_second_body_limit() {
    let mut config = WebdRequestLimitsConfig::default();
    config.max_json_body_bytes = 32;
    let limits = RequestLimits::new(config, 128);
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("100"));
    let path = "/v1/memory/import/uploads/memory_upload_fixture/chunks";
    assert!(limits
        .validate_headers(&headers, classify_request(&Method::PUT, path))
        .is_ok());
    assert_eq!(
        limits.validate_headers(&headers, RequestClass::General),
        Err("webd_request_body_too_large")
    );
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("129"));
    assert_eq!(
        limits.validate_headers(&headers, classify_request(&Method::PUT, path)),
        Err("webd_request_body_too_large")
    );
    let digest = format!("sha256:{}", "0".repeat(64));
    headers.insert("x-content-sha256", digest.parse().unwrap());
    headers.insert(
        crate::AUTH_KEY_HEADER,
        HeaderValue::from_static("untrusted-client-key"),
    );
    let outgoing = crate::build_outgoing_headers(
        &headers,
        "127.0.0.1:8787",
        "127.0.0.1:1234".parse().unwrap(),
        false,
        Some("trusted-session-key"),
    );
    assert_eq!(outgoing["x-content-sha256"].to_str().unwrap(), digest);
    assert_eq!(outgoing[crate::AUTH_KEY_HEADER], "trusted-session-key");
}

#[test]
fn parser_and_upload_use_the_existing_separate_resource_permits() {
    let mut config = WebdRequestLimitsConfig::default();
    config.upload_concurrency = 1;
    config.high_cost_concurrency = 1;
    let limits = RequestLimits::new(config, 128);
    let ip = IpAddr::V4(Ipv4Addr::new(198, 51, 100, 1));
    let _upload = limits
        .try_acquire(ip, Some("fixture"), RequestClass::Upload, 1)
        .unwrap();
    let _parse = limits
        .try_acquire(ip, Some("fixture"), RequestClass::HighCost, 1)
        .unwrap();
    for (class, code) in [
        (RequestClass::Upload, "webd_upload_concurrency_limited"),
        (RequestClass::HighCost, "webd_high_cost_concurrency_limited"),
    ] {
        let error = match limits.try_acquire(ip, Some("fixture"), class, 1) {
            Ok(_) => panic!("resource permit unexpectedly granted"),
            Err(error) => error,
        };
        assert_eq!(error.error_code, code);
    }
}
