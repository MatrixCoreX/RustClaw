use super::validate_clawd_internal_listen;

#[test]
fn clawd_internal_listener_accepts_only_loopback_addresses() {
    assert_eq!(
        validate_clawd_internal_listen("127.0.0.1:18787").expect("IPv4 loopback"),
        "127.0.0.1:18787"
    );
    assert_eq!(
        validate_clawd_internal_listen("[::1]:18787").expect("IPv6 loopback"),
        "[::1]:18787"
    );
    assert!(validate_clawd_internal_listen("0.0.0.0:8787").is_err());
    assert!(validate_clawd_internal_listen("192.168.1.10:8787").is_err());
    assert!(validate_clawd_internal_listen("invalid").is_err());
}
