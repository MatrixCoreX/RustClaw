use agent_desktop::{
    profile::{api_path, https_origin, Connection, ProfileStore},
    transfers::renderer_headers,
};
use std::collections::HashMap;

#[test]
fn packaged_root_accepts_tauri_opaque_empty_path_but_not_remote_navigation() {
    for url in [
        "tauri://localhost",
        "tauri://localhost/",
        "tauri://localhost/index.html",
        "http://tauri.localhost/",
    ] {
        assert!(
            agent_desktop::webview_origin::local_page(
                &reqwest::Url::parse(url).unwrap(),
                "/index.html"
            ),
            "{url}"
        );
    }
    for url in [
        "https://device.local/",
        "tauri://localhost/evil.html",
        "tauri://evil/",
        "http://tauri.localhost:8080/",
        "http://user@tauri.localhost/",
        "http://tauri.localhost.evil.test/",
    ] {
        assert!(
            !agent_desktop::webview_origin::local_page(
                &reqwest::Url::parse(url).unwrap(),
                "/index.html"
            ),
            "{url}"
        );
    }
}

#[test]
fn custom_assets_remain_scoped_on_windows_and_webkit() {
    let prefix = "/session/v1/aipps/demo/assets/";
    for origin in ["device://localhost", "http://device.localhost"] {
        let accepted = reqwest::Url::parse(&format!("{origin}{prefix}index.html")).unwrap();
        assert!(agent_desktop::webview_origin::device_asset(
            &accepted, prefix
        ));
    }
    for url in [
        "http://device.localhost:8080/session/v1/aipps/demo/assets/index.html",
        "http://device.localhost.evil.test/session/v1/aipps/demo/assets/index.html",
        "http://user@device.localhost/session/v1/aipps/demo/assets/index.html",
        "http://device.localhost/session/v1/aipps/other/assets/index.html",
        "device://localhost/other/v1/aipps/demo/assets/index.html",
        "http://device.localhost/session/v1/aipps/demo/assets/../private",
    ] {
        assert!(
            !agent_desktop::webview_origin::device_asset(
                &reqwest::Url::parse(url).unwrap(),
                prefix
            ),
            "{url}"
        );
    }
}

#[test]
fn addresses_and_paths_cannot_expand_the_admitted_target() {
    for url in [
        "http://192.168.1.2",
        "file:///tmp/a",
        "https://user@device.local",
        "https://device.local/v1",
        "https://device.local?x=1",
        "https://device.local/#a",
        "https://device.local\\@evil.test",
    ] {
        assert!(https_origin(url).is_err(), "{url}");
    }
    for url in [
        "https://device.local:8443",
        "https://[::1]:8443",
        "https://192.168.1.2",
    ] {
        assert!(https_origin(url).is_ok());
    }
    for path in [
        "/v1/../admin",
        "/v1/%2e%2e/admin",
        "/v1/%252e/admin",
        "/v1//evil",
        "//evil/v1/tasks",
        "/v1/a%2fb",
        "/v1/a\\b",
        "/v1/a#b",
        "https://evil.test/v1/a",
        "/webd/not-a-contract",
    ] {
        assert!(api_path(path).is_err(), "{path}");
    }
    for path in [
        "/v1/tasks",
        "/webd/login",
        "/v1/tasks/123/events?cursor=2",
        "/v1/search?q=https%3A%2F%2Fexample.com",
    ] {
        assert!(api_path(path).is_ok());
    }
}

#[test]
fn renderer_cannot_set_sensitive_or_proxy_headers() {
    for name in [
        "Host",
        "Cookie",
        "Authorization",
        "X-Agent-Key",
        "X-Agent-Csrf-Token",
        "Origin",
        "X-Forwarded-Proto",
        "Proxy-Authorization",
        "Connection",
    ] {
        assert!(
            renderer_headers(HashMap::from([(name.into(), "value".into())])).is_err(),
            "{name}"
        );
    }
    assert!(renderer_headers(HashMap::from([("Range".into(), "bytes=1-10".into())])).is_ok());
}

#[test]
fn profiles_are_immutable_isolated_and_contain_no_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let store = ProfileStore::new(directory.path().into()).unwrap();
    let connection = Connection::Https {
        origin: "https://device.local".into(),
        ca_pem: None,
        ca_sha256: None,
    };
    let a = store.add("one".into(), connection.clone()).unwrap();
    let b = store.add("two".into(), connection).unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(store.list().unwrap().len(), 2);
    store.forget(a.id).unwrap();
    assert_eq!(store.list().unwrap()[0].id, b.id);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(directory.path().join("profiles-v1.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    let bad = r#"{"kind":"https","origin":"https://a.test","ca_pem":null,"ca_sha256":null,"password":"must-not-persist"}"#;
    assert!(serde_json::from_str::<Connection>(bad).is_err());
}
