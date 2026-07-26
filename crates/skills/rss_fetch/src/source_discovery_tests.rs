use super::*;
use crate::{RssCategoryConfig, RssConfig};
use std::collections::HashMap;

fn config_with_category(category: &str, sources: &[&str]) -> RootConfig {
    let mut categories = HashMap::new();
    categories.insert(
        category.to_string(),
        RssCategoryConfig {
            sources: Some(sources.iter().map(|value| value.to_string()).collect()),
            ..RssCategoryConfig::default()
        },
    );
    RootConfig {
        rss: RssConfig {
            default_category: Some(category.to_string()),
            discovery: Some(RssDiscoveryConfig {
                enabled: Some(true),
                min_active_sources: Some(3),
                promotion_successes: Some(3),
                max_candidates_per_category: Some(10),
                quarantine_after_failures: Some(3),
            }),
            categories,
            ..RssConfig::default()
        },
    }
}

#[test]
fn source_health_reports_machine_discovery_signal() {
    let cfg = config_with_category("general", &["https://example.com/feed.xml"]);
    let output = source_health(&cfg, &Map::new()).expect("health response");
    let extra = output.extra.expect("structured health");

    assert_eq!(extra["action"], "source_health");
    assert_eq!(extra["needs_discovery"], true);
    assert_eq!(extra["categories"][0]["category"], "general");
    assert_eq!(extra["categories"][0]["active_count"], 1);
    assert_eq!(
        extra["categories"][0]["recommended_action"],
        "discover_sources"
    );
}

#[test]
fn source_health_rejects_unknown_category_structurally() {
    let cfg = config_with_category("general", &[]);
    let args = json!({"category": "missing"})
        .as_object()
        .expect("object")
        .clone();
    let error = source_health(&cfg, &args).expect_err("unknown category");

    assert_eq!(error.extra["error_kind"], "category_not_configured");
    assert_eq!(error.extra["invalid_argument"], "category");
    assert_eq!(error.extra["available_categories"], json!(["general"]));
}

#[test]
fn feed_document_requires_parseable_items() {
    assert_eq!(
        validate_feed_document("<html><body>not a feed</body></html>")
            .expect_err("HTML must not pass"),
        "no_parseable_feed_items"
    );
}

#[test]
fn feed_document_accepts_rss_and_returns_samples() {
    let body = r#"
        <rss><channel><title>Example</title>
        <item><title>First item</title><link>https://example.com/1</link></item>
        <item><title>Second item</title><link>https://example.com/2</link></item>
        </channel></rss>
    "#;
    let validated = validate_feed_document(body).expect("valid RSS");

    assert_eq!(validated.item_count, 2);
    assert_eq!(validated.sample_titles, vec!["First item", "Second item"]);
}

#[test]
fn public_url_syntax_rejects_local_and_credentialed_targets() {
    for url in [
        "file:///etc/passwd",
        "http://localhost/feed",
        "http://service.internal/feed",
        "http://127.0.0.1/feed",
        "http://169.254.169.254/latest/meta-data",
        "http://10.0.0.1/feed",
        "http://user:secret@example.com/feed",
        "http://[::1]/feed",
    ] {
        assert!(
            validate_public_url_syntax(url).is_err(),
            "{url} should be rejected"
        );
    }
}

#[test]
fn public_url_syntax_accepts_normal_https_feed() {
    validate_public_url_syntax("https://feeds.example.com/news.xml?lang=en")
        .expect("public hostname syntax");
}

#[test]
fn discovery_requires_evidence_url_before_network() {
    let mut cfg = config_with_category("general", &[]);
    let args = json!({
        "action": "discover_sources",
        "category": "general",
        "candidates": [{
            "url": "https://feeds.example.com/news.xml",
            "discovered_from": "not-a-url"
        }]
    })
    .as_object()
    .expect("object")
    .clone();
    let error = discover_sources(&mut cfg, &args).expect_err("invalid evidence");

    assert_eq!(error.extra["error_kind"], "no_valid_source_candidates");
    assert_eq!(
        error.extra["results"][0]["error_code"],
        "invalid_discovery_evidence:invalid_url"
    );
    assert!(cfg
        .rss
        .categories
        .get("general")
        .and_then(|category| category.candidate_entries.as_ref())
        .is_none());
}

#[test]
fn promotion_requires_explicit_machine_confirmation() {
    let mut cfg = config_with_category("general", &[]);
    cfg.rss
        .categories
        .get_mut("general")
        .expect("category")
        .candidate_entries = Some(vec![CandidateSourceEntry {
        url: "https://feeds.example.com/news.xml".to_string(),
        success_count: 3,
        status: "eligible".to_string(),
        ..CandidateSourceEntry::default()
    }]);
    let args = json!({
        "category": "general",
        "urls": ["https://feeds.example.com/news.xml"]
    })
    .as_object()
    .expect("object")
    .clone();
    let error = promote_sources(&mut cfg, &args).expect_err("confirmation required");

    assert_eq!(
        error.extra["error_kind"],
        "source_promotion_confirmation_required"
    );
    assert_eq!(error.extra["confirmation_field"], "confirm");
    assert_eq!(error.extra["confirmation_value"], true);
}

#[test]
fn refresh_without_candidates_returns_structured_error() {
    let mut cfg = config_with_category("general", &[]);
    let args = json!({"category": "general"})
        .as_object()
        .expect("object")
        .clone();
    let error = refresh_candidates(&mut cfg, &args).expect_err("no candidates");

    assert_eq!(error.extra["error_kind"], "no_source_candidates");
    assert_eq!(error.extra["action"], "refresh_candidates");
}

#[test]
fn candidate_status_becomes_eligible_at_configured_threshold() {
    let entry = CandidateSourceEntry {
        success_count: 3,
        ..CandidateSourceEntry::default()
    };
    assert_eq!(status_after_success(&entry, 3), "eligible");
    assert_eq!(status_after_success(&entry, 4), "candidate");
}

#[test]
fn private_and_reserved_ip_ranges_are_not_public() {
    for ip in [
        "0.0.0.0",
        "10.0.0.1",
        "100.64.0.1",
        "127.0.0.1",
        "169.254.1.1",
        "172.16.0.1",
        "192.168.0.1",
        "192.0.2.1",
        "198.18.0.1",
        "198.51.100.1",
        "203.0.113.1",
        "224.0.0.1",
        "::1",
        "fc00::1",
        "fe80::1",
        "2001:db8::1",
    ] {
        assert!(
            !is_public_ip(ip.parse().expect("test IP")),
            "{ip} should not be public"
        );
    }
    assert!(is_public_ip("8.8.8.8".parse().expect("public IPv4")));
    assert!(is_public_ip(
        "2606:4700:4700::1111".parse().expect("public IPv6")
    ));
}

#[test]
fn synthetic_egress_range_is_only_allowed_as_a_dns_transport_result() {
    let address = "198.18.0.10".parse().expect("synthetic egress IPv4");
    assert!(!is_public_ip(address));
    assert!(is_synthetic_egress_ip(address));
    assert!(validate_public_url_syntax("http://198.18.0.10/feed").is_err());
}
