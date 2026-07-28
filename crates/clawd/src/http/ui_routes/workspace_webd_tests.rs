use super::{parse_webd_listen_address, rewrite_webd_exposure};

const CONFIG: &str = r#"
# Keep this comment.
[webd]
enabled = true
listen = "0.0.0.0:8788"
upstream = "http://127.0.0.1:8787"
"#;

#[test]
fn closes_external_listener_without_changing_port_or_other_settings() {
    let (updated, changed) = rewrite_webd_exposure(CONFIG, false).expect("rewrite");
    assert!(changed);
    assert!(updated.contains("# Keep this comment."));
    assert!(updated.contains("listen = \"127.0.0.1:8788\""));
    assert!(updated.contains("upstream = \"http://127.0.0.1:8787\""));
}

#[test]
fn opens_loopback_listener_without_changing_port() {
    let loopback = CONFIG.replace("0.0.0.0:8788", "127.0.0.1:9191");
    let (updated, changed) = rewrite_webd_exposure(&loopback, true).expect("rewrite");
    assert!(changed);
    assert!(updated.contains("listen = \"0.0.0.0:9191\""));
}

#[test]
fn identical_exposure_is_a_noop() {
    let (updated, changed) = rewrite_webd_exposure(CONFIG, true).expect("rewrite");
    assert!(!changed);
    assert_eq!(updated, CONFIG);
}

#[test]
fn accepts_ipv4_and_ipv6_socket_addresses() {
    assert_eq!(
        parse_webd_listen_address("127.0.0.1:8788").unwrap().port(),
        8788
    );
    assert_eq!(
        parse_webd_listen_address("[::1]:9191").unwrap().port(),
        9191
    );
    assert!(parse_webd_listen_address("localhost:8788").is_err());
}
