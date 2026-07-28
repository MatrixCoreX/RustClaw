use super::{
    bounded_tail, browser_playwright_install_commands, build_ui_router,
    dependency_command_candidates, dependency_install_commands, dependency_version_text,
    dependency_version_text_for, detect_browser_playwright_manifest, host_dependency_catalog,
    linux_dependency_package, playwright_managed_browser_available, prepare_dependency_install,
};
use crate::AppState;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::Value;
use tower::ServiceExt;

#[test]
fn dependency_catalog_ids_are_unique_and_machine_safe() {
    let catalog = host_dependency_catalog();
    let mut ids = catalog.iter().map(|entry| entry.id).collect::<Vec<_>>();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), catalog.len());
    assert!(ids.iter().all(|id| {
        !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }));
    let sandbox = catalog
        .iter()
        .find(|entry| entry.id == "sandbox_backend")
        .expect("sandbox dependency");
    assert!(sandbox.required);
    assert!(catalog.iter().any(|entry| entry.id == "npx"));
    assert!(catalog.iter().any(|entry| entry.id == "browser_playwright"));
}

#[test]
fn command_detection_includes_service_safe_platform_paths() {
    let candidates = dependency_command_candidates("ffmpeg");
    assert!(candidates
        .iter()
        .any(|candidate| candidate.ends_with("ffmpeg")));
    if cfg!(target_os = "macos") {
        assert!(candidates
            .iter()
            .any(|candidate| candidate == std::path::Path::new("/usr/local/bin/ffmpeg")));
        assert!(candidates
            .iter()
            .any(|candidate| candidate == std::path::Path::new("/opt/homebrew/bin/ffmpeg")));
    } else {
        assert!(candidates
            .iter()
            .any(|candidate| candidate == std::path::Path::new("/usr/bin/ffmpeg")));
    }
}

#[test]
fn version_parser_uses_stdout_then_stderr_and_bounds_output() {
    assert_eq!(
        dependency_version_text(b"tool 1.2.3\nmore\n", b"ignored"),
        Some("tool 1.2.3".to_string())
    );
    assert_eq!(
        dependency_version_text(b"", b"nginx version: nginx/1.27\n"),
        Some("nginx version: nginx/1.27".to_string())
    );
    let long = "x".repeat(500);
    assert_eq!(
        dependency_version_text(long.as_bytes(), b"")
            .expect("bounded version")
            .chars()
            .count(),
        240
    );
    assert_eq!(
        dependency_version_text_for(
            "zip",
            b"Copyright Info-ZIP\nThis is Zip 3.0 (July 5th 2008)\n",
            b"",
        ),
        Some("This is Zip 3.0 (July 5th 2008)".to_string())
    );
    assert_eq!(
        dependency_version_text_for(
            "lsof",
            b"",
            b"lsof version information:\n    revision: 4.99.4\n",
        ),
        Some("lsof 4.99.4".to_string())
    );
}

#[test]
fn install_commands_are_whitelisted_by_catalog_and_platform_manager() {
    let ffmpeg = host_dependency_catalog()
        .into_iter()
        .find(|entry| entry.id == "ffmpeg")
        .expect("ffmpeg catalog entry");
    if cfg!(target_os = "macos") {
        let brew = dependency_install_commands(&ffmpeg, "homebrew").expect("brew command");
        assert_eq!(brew.len(), 1);
        assert_eq!(brew[0].args, vec!["install", "ffmpeg"]);
    }

    if unsafe { libc::geteuid() } == 0 {
        let apt = dependency_install_commands(&ffmpeg, "apt").expect("apt commands");
        assert_eq!(apt.len(), 2);
        assert_eq!(apt[0].args, vec!["update", "-qq"]);
        assert_eq!(apt[1].args, vec!["install", "-y", "ffmpeg"]);
    }

    let docker = host_dependency_catalog()
        .into_iter()
        .find(|entry| entry.id == "docker")
        .expect("docker catalog entry");
    assert_eq!(linux_dependency_package(&docker, "apt"), Some("docker.io"));
    assert_eq!(linux_dependency_package(&docker, "pacman"), Some("docker"));
}

#[test]
fn installed_dependency_is_not_scheduled_again() {
    let bash = host_dependency_catalog()
        .into_iter()
        .find(|entry| entry.id == "bash")
        .expect("bash catalog entry");
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(matches!(
        prepare_dependency_install(&bash, &workspace_root),
        Err("dependency_already_installed")
    ));
}

#[test]
fn playwright_install_is_fixed_to_the_browser_skill_package() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let commands = browser_playwright_install_commands(&workspace_root)
        .expect("browser Playwright install command");
    assert_eq!(commands.len(), 2);
    assert_eq!(
        commands[0].args,
        vec!["ci", "--omit=dev", "--no-audit", "--no-fund"]
    );
    assert!(commands[0]
        .current_dir
        .as_deref()
        .is_some_and(|path| path.ends_with("crates/skills/browser_web")));
    assert!(commands[0].program.ends_with("npm"));
    assert!(commands[1].program.ends_with("node"));
    assert!(commands[1].args[0].ends_with("node_modules/playwright/cli.js"));
    assert_eq!(&commands[1].args[1..], ["install", "chromium"]);
}

#[test]
fn playwright_manifest_detection_reads_only_the_browser_skill_manifest() {
    let root = std::env::temp_dir().join(format!(
        "rustclaw-playwright-dependency-test-{}",
        uuid::Uuid::new_v4()
    ));
    let package_dir = root.join("crates/skills/browser_web/node_modules/playwright");
    std::fs::create_dir_all(&package_dir).expect("create Playwright fixture");
    std::fs::write(package_dir.join("package.json"), r#"{"version":"1.58.2"}"#)
        .expect("write Playwright fixture");

    let browser_skill_dir = root.join("crates/skills/browser_web");
    assert_eq!(
        detect_browser_playwright_manifest(&browser_skill_dir).as_deref(),
        Some("1.58.2")
    );
    assert!(!playwright_managed_browser_available(&browser_skill_dir));

    std::fs::remove_dir_all(root).expect("remove Playwright fixture");
}

#[test]
fn operation_logs_keep_only_the_bounded_tail() {
    assert_eq!(bounded_tail("abcdef", 4), "cdef");
    assert_eq!(bounded_tail("短文本", 2), "文本");
}

#[tokio::test]
async fn dependency_snapshot_requires_ui_authentication() {
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    let response = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state)
        .oneshot(
            Request::builder()
                .uri("/v1/system/dependencies")
                .body(Body::empty())
                .expect("dependency request"),
        )
        .await
        .expect("dependency response");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn install_endpoint_rejects_unknown_dependency_tokens() {
    const KEY: &str = "rk-dependency-admin-test";
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state
        .core
        .db
        .get()
        .expect("main db")
        .execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, 'admin', 1, '1', NULL)",
            rusqlite::params![KEY],
        )
        .expect("seed auth key");
    let response = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/system-dependencies/install")
                .header("content-type", "application/json")
                .header("x-rustclaw-key", KEY)
                .body(Body::from(r#"{"dependency_id":"; rm -rf /"}"#))
                .expect("install request"),
        )
        .await
        .expect("install response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), 16 * 1024)
        .await
        .expect("bounded response");
    let value: Value = serde_json::from_slice(&body).expect("response JSON");
    assert_eq!(value["data"]["error_code"], "dependency_unknown");
}

#[tokio::test]
async fn install_endpoint_requires_admin_role() {
    const KEY: &str = "rk-dependency-user-test";
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state
        .core
        .db
        .get()
        .expect("main db")
        .execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, 'user', 1, '1', NULL)",
            rusqlite::params![KEY],
        )
        .expect("seed auth key");
    let response = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/system-dependencies/install")
                .header("content-type", "application/json")
                .header("x-rustclaw-key", KEY)
                .body(Body::from(r#"{"dependency_id":"ffmpeg"}"#))
                .expect("install request"),
        )
        .await
        .expect("install response");
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn authenticated_snapshot_reports_bounded_machine_dependency_fields() {
    const KEY: &str = "rk-dependency-snapshot-test";
    let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
    state
        .core
        .db
        .get()
        .expect("main db")
        .execute(
            "INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
             VALUES (?1, 'admin', 1, '1', NULL)",
            rusqlite::params![KEY],
        )
        .expect("seed auth key");
    let response = axum::Router::new()
        .nest("/v1", build_ui_router())
        .with_state(state)
        .oneshot(
            Request::builder()
                .uri("/v1/system/dependencies")
                .header("x-rustclaw-key", KEY)
                .body(Body::empty())
                .expect("dependency snapshot request"),
        )
        .await
        .expect("dependency snapshot response");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), 128 * 1024)
        .await
        .expect("bounded dependency response");
    let value: Value = serde_json::from_slice(&body).expect("dependency JSON");
    assert_eq!(value["data"]["schema_version"], 1);
    assert!(value["data"]["summary"]["total"].as_u64().unwrap_or(0) >= 20);
    assert!(value["data"]["dependencies"].as_array().is_some());
    let encoded = String::from_utf8(body.to_vec()).expect("UTF-8 response");
    assert!(!encoded.contains(KEY));
    assert!(!encoded.contains("PATH="));
}
