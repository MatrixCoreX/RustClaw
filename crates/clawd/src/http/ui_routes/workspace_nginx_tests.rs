use super::{nginx_config_is_rustclaw_site, nginx_ui_root_from_config};

#[test]
fn recognizes_rustclaw_nginx_site_and_ui_root() {
    let config = r#"
# RustClaw UI: static assets and authenticated API gateway.
server {
    root /var/www/html/rustclaw;
    location ^~ /v1/ { proxy_pass http://127.0.0.1:8788; }
    location ^~ /webd/ { proxy_pass http://127.0.0.1:8788; }
}
"#;
    assert!(nginx_config_is_rustclaw_site(config));
    assert_eq!(
        nginx_ui_root_from_config(config).as_deref(),
        Some(std::path::Path::new("/var/www/html/rustclaw"))
    );
}

#[test]
fn rejects_unrelated_nginx_site() {
    let config = "server { root /srv/example; location / { try_files $uri =404; } }";
    assert!(!nginx_config_is_rustclaw_site(config));
    assert!(nginx_ui_root_from_config(config).is_none());
}

#[test]
fn rejects_site_that_bypasses_webd_and_proxies_to_clawd() {
    let config = r#"
# RustClaw UI
server {
    root /var/www/html/rustclaw;
    location ^~ /v1/ { proxy_pass http://127.0.0.1:8787; }
    location ^~ /webd/ { proxy_pass http://127.0.0.1:8787; }
}
"#;
    assert!(!nginx_config_is_rustclaw_site(config));
}

#[test]
fn rejects_clawd_loopback_aliases_and_paths() {
    for upstream in [
        "http://127.0.0.1:8787/",
        "http://localhost:8787/v1/",
        "http://[::1]:8787/",
    ] {
        let config = format!(
            r#"
# RustClaw UI
server {{
    root /var/www/html/rustclaw;
    location ^~ /v1/ {{ proxy_pass {upstream}; }}
    location ^~ /webd/ {{ proxy_pass {upstream}; }}
}}
"#
        );
        assert!(!nginx_config_is_rustclaw_site(&config), "{upstream}");
    }
}
