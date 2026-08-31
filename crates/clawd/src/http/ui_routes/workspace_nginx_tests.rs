use super::{
    nginx_config_is_agent_site, nginx_local_https_is_enabled, nginx_ui_root_from_config,
    normalize_local_https_fingerprint, normalize_local_mdns_hostname,
};

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

#[test]
fn local_https_status_requires_the_managed_marker_and_tls_listener() {
    let staged = r#"
# Agent Runtime UI: local-CA HTTPS entry with a loopback-only WEBD upstream.
server {
    listen 0.0.0.0:80;
}
server {
    listen 0.0.0.0:443 ssl;
}
"#;
    assert!(nginx_local_https_is_enabled(staged));
    assert!(!nginx_local_https_is_enabled("server { listen 443 ssl; }"));
    assert!(!nginx_local_https_is_enabled(
        "# Agent Runtime UI: local-CA HTTPS entry\nserver { listen 80; }"
    ));
}

#[test]
fn local_https_fingerprint_contract_is_strict_and_case_normalized() {
    let fingerprint = (0..32).map(|_| "ab").collect::<Vec<_>>().join(":");
    let uppercase = fingerprint.to_ascii_uppercase();
    assert_eq!(
        normalize_local_https_fingerprint(&format!(" {fingerprint}\n")).as_deref(),
        Some(uppercase.as_str())
    );
    assert!(normalize_local_https_fingerprint("AA:BB").is_none());
    assert!(normalize_local_https_fingerprint(&fingerprint.replace(':', "-")).is_none());
}

#[test]
fn local_https_script_keeps_http_available_during_client_activation() {
    let script = include_str!("../../../../../scripts/configure-local-lan-https.sh");
    assert!(script.contains("--prepare-only"));
    assert!(script.contains("local_lan_https=prepared"));
    assert!(script.contains("local_lan_https=enabled"));
    assert!(script.contains("try_files \\$uri \\$uri/ /index.html;"));
    assert!(!script.contains("return 308 https://\\$host\\$request_uri;"));
}

#[test]
fn local_mdns_hostname_contract_accepts_one_safe_dns_label() {
    for (input, expected) in [
        ("home-agent", "home-agent"),
        ("HOME-AGENT", "home-agent"),
        ("home-agent.local", "home-agent"),
        ("a1", "a1"),
    ] {
        assert_eq!(
            normalize_local_mdns_hostname(input).as_deref(),
            Some(expected)
        );
    }
    for invalid in [
        "",
        "-home",
        "home-",
        "home.local.local",
        "home_agent",
        "home agent",
        "设备",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ] {
        assert!(
            normalize_local_mdns_hostname(invalid).is_none(),
            "{invalid}"
        );
    }
}

#[test]
fn local_mdns_script_supports_linux_and_macos_without_shell_evaluation() {
    let script = include_str!("../../../../../scripts/configure-local-mdns.sh");
    assert!(script.contains("hostnamectl set-hostname \"$LOCAL_HOSTNAME\""));
    assert!(script.contains("scutil --set LocalHostName \"$LOCAL_HOSTNAME\""));
    assert!(script.contains("systemctl restart avahi-daemon"));
    assert!(!script.contains("eval "));
}
