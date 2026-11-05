use super::model_settings_integrity_tests::support::{assert_error, Fixture, ADMIN, SECRET, USER};
use super::*;
use axum::http::Method;

#[path = "configuration_read_offload_tests.rs"]
mod offload;

const ROUTES: [(&str, &str); 2] = [
    ("/v1/llm/config", "llm_config_read_failed"),
    ("/v1/skills/config", "skills_config_read_failed"),
];

fn read_fixture() -> Fixture {
    let fixture = Fixture::new();
    let path = fixture.root.join("configs/config.toml");
    let raw = fs::read_to_string(path).unwrap();
    fixture.write("config.toml", &format!("{raw}\n[skills]\nregistry_path='configs/skills_registry.toml'\n[server]\nrequest_timeout_seconds=30\n[database]\nsqlite_path='fixture.sqlite3'\nbusy_timeout_ms=1000\nskill_data_root='data/skills'\n[worker]\nconcurrency=1\n"));
    fixture.write("skills_registry.toml", "# fixture registry\n");
    claw_core::config::AppConfig::load(&fixture.root.join("configs/config.toml").to_string_lossy())
        .unwrap();
    fixture
}

fn inline(fixture: &Fixture, index: usize, key: &str) -> (StatusCode, Value) {
    let mut headers = HeaderMap::new();
    headers.insert(
        claw_core::product_identity::AUTH_KEY_HEADER,
        axum::http::HeaderValue::from_str(key).unwrap(),
    );
    let read = if index == 0 {
        read_llm_config
    } else {
        read_skills_config
    };
    let (status, Json(body)) = read(State(fixture.state.clone()), headers);
    (status, serde_json::to_value(body).unwrap())
}

#[tokio::test]
async fn configuration_reads_preserve_complete_success_and_credential_privacy_without_writes() {
    let fixture = read_fixture();
    let before = fixture.files();
    let registry = fs::read(fixture.root.join("configs/skills_registry.toml")).unwrap();
    for (index, (path, _)) in ROUTES.iter().enumerate() {
        for key in [ADMIN, USER] {
            let expected = inline(&fixture, index, key);
            assert_eq!(expected.0, StatusCode::OK);
            let response = fixture.api(key, Method::GET, path, None).await;
            assert_eq!(response, expected);
            fixture.assert_private(&response.1);
        }
    }
    assert_eq!(fixture.files(), before);
    assert_eq!(
        fs::read(fixture.root.join("configs/skills_registry.toml")).unwrap(),
        registry
    );
    assert!(!fixture.root.join("data/skills/.runtime-admission").exists());
}

#[tokio::test]
async fn configuration_reads_authorize_before_parsing_private_files() {
    let fixture = read_fixture();
    fixture.write("config.toml", &format!("api_key = {SECRET}\n"));
    for (path, _) in ROUTES {
        for (key, code) in [
            ("", "auth_key_required"),
            ("fixture-unknown", "auth_key_invalid"),
        ] {
            let response = fixture.api(key, Method::GET, path, None).await;
            assert_error(&response, StatusCode::UNAUTHORIZED, code);
            fixture.assert_private(&response.1);
        }
    }
}

#[tokio::test]
async fn configuration_reads_fail_safely_for_malformed_unreadable_and_missing_files() {
    for shape in ["malformed", "non_utf8", "directory", "missing"] {
        let fixture = read_fixture();
        let path = fixture.root.join("configs/config.toml");
        let original = fs::read(&path).unwrap();
        match shape {
            "malformed" => fixture.write("config.toml", &format!("api_key = {SECRET}\n")),
            "non_utf8" => fs::write(&path, [0xff, 0xfe]).unwrap(),
            "directory" => {
                fs::remove_file(&path).unwrap();
                fs::create_dir(&path).unwrap();
            }
            "missing" => fs::remove_file(&path).unwrap(),
            _ => unreachable!(),
        }
        let before = fixture.files();
        for (endpoint, code) in ROUTES {
            for key in [ADMIN, USER] {
                let response = fixture.api(key, Method::GET, endpoint, None).await;
                assert_error(&response, StatusCode::INTERNAL_SERVER_ERROR, code);
                fixture.assert_private(&response.1);
            }
        }
        assert_eq!(fixture.files(), before);
        if path.is_dir() {
            fs::remove_dir(&path).unwrap();
        }
        fs::write(path, original).unwrap();
        for (endpoint, _) in ROUTES {
            assert_eq!(
                fixture.api(USER, Method::GET, endpoint, None).await.0,
                StatusCode::OK
            );
        }
    }
}

#[tokio::test]
async fn configuration_reads_keep_the_active_relative_and_absolute_path_contract() {
    for absolute in [false, true] {
        let mut fixture = read_fixture();
        let raw = fs::read_to_string(fixture.root.join("configs/config.toml")).unwrap();
        let alternate = fixture.root.join("active/config.toml");
        fs::create_dir(alternate.parent().unwrap()).unwrap();
        fs::write(&alternate, raw.replace("fixture-model", "fixture-active")).unwrap();
        fixture.state.reload_ctx.config_path_for_reload = if absolute {
            alternate.to_string_lossy().into_owned()
        } else {
            "active/config.toml".to_string()
        };
        fixture.write("config.toml", &format!("api_key = {SECRET}\n"));
        let before = fixture.files();
        for (endpoint, _) in ROUTES {
            let response = fixture.api(USER, Method::GET, endpoint, None).await;
            assert_eq!(response.0, StatusCode::OK);
            if endpoint == ROUTES[0].0 {
                assert_eq!(response.1["data"]["selected_model"], "fixture-active");
            }
            fixture.assert_private(&response.1);
        }
        assert_eq!(fixture.files(), before);
    }
}

#[tokio::test]
async fn configuration_reads_keep_admission_and_split_config_errors_private() {
    for shape in ["registry_missing", "split_config", "generation_pointer"] {
        let fixture = read_fixture();
        match shape {
            "registry_missing" => {
                fs::remove_file(fixture.root.join("configs/skills_registry.toml")).unwrap()
            }
            "split_config" => fixture.write("image.toml", &format!("api_key = {SECRET}\n")),
            "generation_pointer" => {
                let root = fixture.root.join("data/skills/.runtime-admission");
                fs::create_dir_all(&root).unwrap();
                fs::write(root.join("current-generation.json"), format!("{{{SECRET}")).unwrap();
            }
            _ => unreachable!(),
        }
        let response = fixture.api(ADMIN, Method::GET, ROUTES[1].0, None).await;
        assert_error(
            &response,
            StatusCode::INTERNAL_SERVER_ERROR,
            "skill_admission_read_failed",
        );
        fixture.assert_private(&response.1);
        assert_eq!(
            fixture.api(ADMIN, Method::GET, ROUTES[0].0, None).await.0,
            StatusCode::OK
        );
    }
}

#[tokio::test]
async fn configuration_reads_do_not_cache_revoked_credentials_or_hide_database_failure() {
    let fixture = read_fixture();
    for (path, _) in ROUTES {
        assert_eq!(
            fixture.api(ADMIN, Method::GET, path, None).await.0,
            StatusCode::OK
        );
    }
    fixture.sql("UPDATE auth_keys SET enabled=0 WHERE user_key='fixture-settings-admin'");
    for (path, _) in ROUTES {
        assert_error(
            &fixture.api(ADMIN, Method::GET, path, None).await,
            StatusCode::UNAUTHORIZED,
            "auth_key_invalid",
        );
    }
    fixture.sql("DROP TABLE auth_keys");
    for (path, _) in ROUTES {
        let response = fixture.api(USER, Method::GET, path, None).await;
        assert_error(
            &response,
            StatusCode::INTERNAL_SERVER_ERROR,
            "auth_lookup_failed",
        );
        fixture.assert_private(&response.1);
    }
}

#[tokio::test]
async fn configuration_reads_do_not_expose_worker_panic_details_and_can_recover() {
    fn failed(_: State<AppState>, _: HeaderMap) -> (StatusCode, Json<ApiResponse<Value>>) {
        panic!("fixture-only panic detail {SECRET}");
    }
    let fixture = read_fixture();
    let (status, Json(body)) = configuration_read_request(
        fixture.state.clone(),
        HeaderMap::new(),
        failed,
        "llm_config_read_failed",
    )
    .await;
    let response = (status, serde_json::to_value(body).unwrap());
    assert_error(
        &response,
        StatusCode::INTERNAL_SERVER_ERROR,
        "llm_config_read_failed",
    );
    fixture.assert_private(&response.1);
    assert_eq!(
        fixture.api(USER, Method::GET, ROUTES[0].0, None).await.0,
        StatusCode::OK
    );
}
