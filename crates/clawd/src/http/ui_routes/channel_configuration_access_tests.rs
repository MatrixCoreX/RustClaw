use std::collections::BTreeMap;

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request};
use tower::ServiceExt;

use super::*;

struct Fixture {
    root: PathBuf,
    router: axum::Router,
}

impl Fixture {
    fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("channel-config-access-{}", uuid::Uuid::new_v4()));
        for dir in ["configs/channels", "docker/config/channels"] {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
        fs::write(
            root.join("configs/config.toml"),
            "[server]\nrequest_timeout_seconds = 30\n\n[database]\nsqlite_path = \"data/fixture.db\"\nbusy_timeout_ms = 1000\n\n[worker]\n\n[llm]\nselected_vendor = \"\"\nselected_model = \"\"\n",
        )
        .unwrap();
        for channel in ["telegram", "wechat", "feishu", "lark"] {
            let raw = format!("[{channel}]\nenabled = false\napp_id = \"fixture-app-id\"\napp_secret = \"fixture-{channel}-secret\"\nbot_token = \"fixture-telegram-token\"\n");
            for dir in ["configs/channels", "docker/config/channels"] {
                fs::write(root.join(dir).join(format!("{channel}.toml")), &raw).unwrap();
            }
        }
        let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        state.skill_rt.workspace_root = root.clone();
        state.seed_test_auth_identity("fixture-channel-access-admin", "admin");
        state.seed_test_auth_identity("fixture-channel-access-user", "user");
        let router = axum::Router::new()
            .nest("/v1", build_ui_router())
            .with_state(state);
        Self { root, router }
    }

    fn snapshot(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut paths = vec![self.root.join("configs/config.toml")];
        for channel in ["telegram", "wechat", "feishu", "lark"] {
            for dir in ["configs/channels", "docker/config/channels"] {
                paths.push(self.root.join(dir).join(format!("{channel}.toml")));
            }
        }
        paths
            .into_iter()
            .map(|path| {
                let data = fs::read(&path).unwrap();
                (path, data)
            })
            .collect()
    }

    async fn api(
        &self,
        role: Option<&str>,
        method: Method,
        channel: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(format!("/v1/{channel}/config"));
        if let Some(role) = role {
            request = request.header("x-agent-key", format!("fixture-channel-access-{role}"));
        }
        let body = if let Some(body) = body {
            request = request.header("content-type", "application/json");
            Body::from(serde_json::to_vec(&body).unwrap())
        } else {
            Body::empty()
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let raw = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&raw).unwrap())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn channel_config_reads_authenticate_and_preserve_secret_free_user_metadata() {
    let fixture = Fixture::new();
    let before = fixture.snapshot();
    let mut failures = Vec::new();
    for role in [None, Some("user")] {
        for channel in ["telegram", "wechat", "feishu", "lark"] {
            let (status, body) = fixture.api(role, Method::GET, channel, None).await;
            let expected = if role.is_none() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::OK
            };
            let secret_visible = body.to_string().contains("fixture-feishu-secret")
                || body.to_string().contains("fixture-lark-secret")
                || body.to_string().contains("fixture-telegram-token");
            let data_matches_role = if role.is_none() {
                body["data"].is_null()
            } else {
                body["data"].is_object()
            };
            let metadata_preserved = role.is_none()
                || !matches!(channel, "feishu" | "lark")
                || (body["data"]["bind_ready"] == true
                    && body["data"]["app_id"] == "fixture-app-id"
                    && body["data"]["app_secret"] == "");
            if status != expected || !data_matches_role || secret_visible || !metadata_preserved {
                failures.push(json!({"channel":channel,"anonymous":role.is_none(),"status":status.as_u16(),"secret_visible":secret_visible}));
            }
            if status == expected {
                assert_eq!(body["ok"], role.is_some());
            }
        }
    }
    assert_eq!(fixture.snapshot(), before);
    assert!(
        failures.is_empty(),
        "channel_configuration_read_access={failures:?}"
    );
}

#[tokio::test]
async fn channel_config_writes_deny_ordinary_users_without_file_changes() {
    let mut failures = Vec::new();
    for role in [None, Some("user")] {
        for channel in ["feishu", "lark"] {
            let fixture = Fixture::new();
            let before = fixture.snapshot();
            let (status, body) = fixture.api(role, Method::POST, channel, Some(json!({"app_id":"fixture-rotated-id", "app_secret":"fixture-rotated-secret"}))).await;
            let expected = if role.is_none() {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::FORBIDDEN
            };
            let unchanged = fixture.snapshot() == before;
            if status != expected || !unchanged || !body["data"].is_null() {
                failures.push(json!({"channel":channel,"anonymous":role.is_none(),"status":status.as_u16(),"files_unchanged":unchanged}));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "channel_configuration_write_access={failures:?}"
    );
}

#[tokio::test]
async fn channel_config_administrator_reads_and_writes_remain_available() {
    let fixture = Fixture::new();
    for channel in ["telegram", "wechat", "feishu", "lark"] {
        let (status, body) = fixture.api(Some("admin"), Method::GET, channel, None).await;
        assert_eq!(status, StatusCode::OK, "{channel}:{body:?}");
        assert_eq!(body["ok"], true);
        if matches!(channel, "feishu" | "lark") {
            assert_eq!(
                body["data"]["app_secret"],
                format!("fixture-{channel}-secret")
            );
        }
    }
    for channel in ["feishu", "lark"] {
        let (status, body) = fixture
            .api(
                Some("admin"),
                Method::POST,
                channel,
                Some(json!({"app_id":"fixture-rotated-id", "app_secret":"fixture-rotated-secret"})),
            )
            .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
        for dir in ["configs/channels", "docker/config/channels"] {
            let value: toml::Value = toml::from_str(
                &fs::read_to_string(fixture.root.join(dir).join(format!("{channel}.toml")))
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(
                value[channel]["app_id"].as_str(),
                Some("fixture-rotated-id")
            );
        }
    }
}

#[tokio::test]
async fn channel_config_claimed_role_fields_do_not_authorize_writes() {
    for channel in ["feishu", "lark"] {
        let fixture = Fixture::new();
        let before = fixture.snapshot();
        let (status, body) = fixture.api(Some("user"), Method::POST, channel, Some(json!({"app_id":"fixture-rotated-id", "app_secret":"fixture-rotated-secret", "role":"admin", "isAdmin":true,"user_key":"fixture-channel-access-admin"}))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["ok"], false);
        assert_eq!(fixture.snapshot(), before);
    }
}

#[tokio::test]
async fn channel_config_parse_errors_never_project_credential_bearing_source() {
    let mut failures = Vec::new();
    for role in ["user", "admin"] {
        for channel in ["telegram", "wechat", "feishu", "lark"] {
            let fixture = Fixture::new();
            let path = if channel == "telegram" {
                fixture.root.join("configs/config.toml")
            } else {
                fixture
                    .root
                    .join("configs/channels")
                    .join(format!("{channel}.toml"))
            };
            fs::write(path, "app_secret = \"fixture-malformed-secret\n").unwrap();
            let before = fixture.snapshot();
            let (status, body) = fixture.api(Some(role), Method::GET, channel, None).await;
            let secret_visible = body.to_string().contains("fixture-malformed-secret");
            if status != StatusCode::INTERNAL_SERVER_ERROR || secret_visible {
                failures.push(json!({"channel":channel,"admin":role=="admin","status":status.as_u16(),"secret_visible":secret_visible}));
            }
            assert_eq!(fixture.snapshot(), before);
        }
    }
    assert!(
        failures.is_empty(),
        "channel_configuration_error_privacy={failures:?}"
    );
}
