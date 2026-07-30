use super::{nginx_config_is_agent_site, nginx_ui_root_from_config};

#[test]
fn recognizes_agent_nginx_site_and_ui_root() {
    let config = r#"
# Agent Runtime UI: static assets and authenticated API gateway.
server {
    root /var/www/html/agent-runtime;
    location ^~ /v1/ { proxy_pass http://127.0.0.1:8788; }
    location ^~ /webd/ { proxy_pass http://127.0.0.1:8788; }
}
"#;
    assert!(nginx_config_is_agent_site(config));
    assert_eq!(
        nginx_ui_root_from_config(config).as_deref(),
        Some(std::path::Path::new("/var/www/html/agent-runtime"))
    );
}

#[test]
fn rejects_unrelated_nginx_site() {
    let config = "server { root /srv/example; location / { try_files $uri =404; } }";
    assert!(!nginx_config_is_agent_site(config));
    assert!(nginx_ui_root_from_config(config).is_none());
}

#[test]
fn rejects_site_that_bypasses_webd_and_proxies_to_clawd() {
    let config = r#"
# Agent Runtime UI
server {
    root /var/www/html/agent-runtime;
    location ^~ /v1/ { proxy_pass http://127.0.0.1:8787; }
    location ^~ /webd/ { proxy_pass http://127.0.0.1:8787; }
}
"#;
    assert!(!nginx_config_is_agent_site(config));
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
# Agent Runtime UI
server {{
    root /var/www/html/agent-runtime;
    location ^~ /v1/ {{ proxy_pass {upstream}; }}
    location ^~ /webd/ {{ proxy_pass {upstream}; }}
}}
"#
        );
        assert!(!nginx_config_is_agent_site(&config), "{upstream}");
    }
}
