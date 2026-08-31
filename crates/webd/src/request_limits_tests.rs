use std::net::{IpAddr, Ipv4Addr};

use axum::http::{header, HeaderMap, HeaderValue, Method};
use claw_core::config::WebdRequestLimitsConfig;

use super::{classify_request, LimitRejection, RequestClass, RequestLease, RequestLimits};

fn client_ip(last_octet: u8) -> IpAddr {
    IpAddr::V4(Ipv4Addr::new(198, 51, 100, last_octet))
}

fn limits(mut config: WebdRequestLimitsConfig) -> RequestLimits {
    config.max_header_bytes = 128;
    config.max_json_body_bytes = 32;
    RequestLimits::new(config, 1024)
}

fn rejection(result: Result<RequestLease, LimitRejection>) -> LimitRejection {
    match result {
        Ok(lease) => {
            drop(lease);
            panic!("request should have been rejected");
        }
        Err(rejection) => rejection,
    }
}

#[test]
fn route_classes_are_machine_paths_not_natural_language() {
    assert_eq!(
        classify_request(&Method::POST, "/v1/tasks"),
        RequestClass::TaskSubmit
    );
    assert_eq!(
        classify_request(&Method::GET, "/v1/tasks/task-1/events"),
        RequestClass::Sse
    );
    assert_eq!(
        classify_request(&Method::POST, "/v1/skills/import/upload"),
        RequestClass::Upload
    );
    assert_eq!(
        classify_request(&Method::POST, "/v1/nni/assets/transfer"),
        RequestClass::HighCost
    );
    assert_eq!(
        classify_request(&Method::GET, "/v1/nni/assets/transfer"),
        RequestClass::General
    );
}

#[test]
fn ordinary_and_upload_body_limits_are_distinct() {
    let limits = limits(WebdRequestLimitsConfig::default());
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from_static("33"));
    assert_eq!(
        limits.validate_headers(&headers, RequestClass::General),
        Err("webd_request_body_too_large")
    );
    assert_eq!(
        limits.validate_headers(&headers, RequestClass::Upload),
        Ok(())
    );

    headers.insert(header::CONTENT_ENCODING, HeaderValue::from_static("gzip"));
    assert_eq!(
        limits.validate_headers(&headers, RequestClass::Upload),
        Err("webd_request_content_encoding_unsupported")
    );
}

#[test]
fn oversized_or_ambiguous_headers_are_rejected() {
    let limits = limits(WebdRequestLimitsConfig::default());
    let mut headers = HeaderMap::new();
    headers.insert("x-large", HeaderValue::from_bytes(&[b'x'; 129]).unwrap());
    assert_eq!(
        limits.validate_headers(&headers, RequestClass::General),
        Err("webd_request_headers_too_large")
    );

    let mut duplicate_length = HeaderMap::new();
    duplicate_length.append(header::CONTENT_LENGTH, HeaderValue::from_static("1"));
    duplicate_length.append(header::CONTENT_LENGTH, HeaderValue::from_static("1"));
    assert_eq!(
        limits.validate_headers(&duplicate_length, RequestClass::General),
        Err("webd_content_length_invalid")
    );
}

#[test]
fn concurrency_lease_is_released_when_request_finishes() {
    let config = WebdRequestLimitsConfig {
        global_concurrency: 1,
        per_ip_rpm: 10,
        ..WebdRequestLimitsConfig::default()
    };
    let limits = limits(config);
    let first = limits
        .try_acquire(client_ip(1), Some("session-1"), RequestClass::General, 60)
        .unwrap();
    let rejected =
        rejection(limits.try_acquire(client_ip(2), Some("session-2"), RequestClass::General, 60));
    assert_eq!(rejected.error_code, "webd_global_concurrency_limited");

    drop(first);
    assert!(limits
        .try_acquire(client_ip(2), Some("session-2"), RequestClass::General, 60)
        .is_ok());
}

#[test]
fn sse_has_a_separate_per_ip_connection_budget() {
    let config = WebdRequestLimitsConfig {
        sse_per_ip_concurrency: 1,
        ..WebdRequestLimitsConfig::default()
    };
    let limits = limits(config);
    let first = limits
        .try_acquire(client_ip(3), None, RequestClass::Sse, 60)
        .unwrap();
    let rejected = rejection(limits.try_acquire(client_ip(3), None, RequestClass::Sse, 60));
    assert_eq!(rejected.error_code, "webd_sse_concurrency_limited");

    drop(first);
    assert!(limits
        .try_acquire(client_ip(3), None, RequestClass::Sse, 60)
        .is_ok());
}

#[test]
fn sse_lifetime_is_bounded_and_never_disabled_by_zero() {
    let configured = limits(WebdRequestLimitsConfig {
        sse_max_lifetime_seconds: 90,
        ..WebdRequestLimitsConfig::default()
    });
    assert_eq!(configured.sse_max_lifetime().as_secs(), 90);

    let zero = limits(WebdRequestLimitsConfig {
        sse_max_lifetime_seconds: 0,
        ..WebdRequestLimitsConfig::default()
    });
    assert_eq!(zero.sse_max_lifetime().as_secs(), 1);
}

#[test]
fn class_rate_limit_recovers_in_the_next_window() {
    let config = WebdRequestLimitsConfig {
        task_per_ip_rpm: 1,
        per_ip_rpm: 10,
        ..WebdRequestLimitsConfig::default()
    };
    let limits = limits(config);
    drop(
        limits
            .try_acquire(client_ip(4), None, RequestClass::TaskSubmit, 119)
            .unwrap(),
    );
    let rejected = rejection(limits.try_acquire(client_ip(4), None, RequestClass::TaskSubmit, 119));
    assert_eq!(rejected.error_code, "webd_task_rate_limited");
    assert_eq!(rejected.retry_after_seconds, 1);

    assert!(limits
        .try_acquire(client_ip(4), None, RequestClass::TaskSubmit, 120)
        .is_ok());
}
