use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    extract::{Form, State},
    http::{HeaderMap, HeaderValue},
    routing::post,
    Json, Router,
};
use serde_json::{json, Value};
use tokio::sync::{Notify, Semaphore};

use super::super::*;

pub(super) const ADMIN_KEY: &str = "fixture-channel-registration-admin";

#[derive(Clone)]
struct RegistrationServer {
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
    requests: Arc<AtomicUsize>,
    result: Value,
    action: &'static str,
}

async fn registration(
    State(state): State<RegistrationServer>,
    headers: HeaderMap,
    Form(form): Form<std::collections::HashMap<String, String>>,
) -> Json<Value> {
    assert!(!headers.contains_key("x-agent-key"));
    assert!(!headers.contains_key("authorization"));
    state.requests.fetch_add(1, Ordering::SeqCst);
    if state.action == "begin" && form.get("action").map(String::as_str) == Some("init") {
        assert_eq!(form.len(), 1);
        return Json(json!({"supported_auth_methods":["client_secret"]}));
    }
    assert_eq!(form.get("action").map(String::as_str), Some(state.action));
    if state.action == "poll" {
        assert_eq!(
            form.get("device_code").map(String::as_str),
            Some("fixture-device-code")
        );
    } else {
        assert_eq!(
            form.get("archetype").map(String::as_str),
            Some("PersonalAgent")
        );
        assert_eq!(
            form.get("auth_method").map(String::as_str),
            Some("client_secret")
        );
        assert_eq!(
            form.get("request_user_info").map(String::as_str),
            Some("open_id")
        );
        assert!(!form.contains_key("device_code"));
    }
    state.entered.notify_one();
    state.release.acquire().await.unwrap().forget();
    Json(state.result)
}

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) state: AppState,
    pub(super) platform: AgentAppChannel,
    pub(super) session_id: i64,
    entered: Arc<Notify>,
    release: Arc<Semaphore>,
    requests: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
    env_name: String,
    prior_env: Option<OsString>,
}

impl Fixture {
    pub(super) async fn new(platform: AgentAppChannel, pending: bool) -> Self {
        let result = if pending {
            json!({"error":"authorization_pending"})
        } else {
            json!({"client_id":"fixture-app-id","client_secret":"fixture-app-secret","user_info":{"open_id":"fixture-open-id"}})
        };
        Self::with_result(platform, result).await
    }

    pub(super) async fn with_result(platform: AgentAppChannel, result: Value) -> Self {
        Self::with_action(platform, result, "poll").await
    }

    pub(super) async fn for_begin(platform: AgentAppChannel, result: Value) -> Self {
        Self::with_action(platform, result, "begin").await
    }

    async fn with_action(platform: AgentAppChannel, result: Value, action: &'static str) -> Self {
        assert!(
            !service_is_running(platform.service()),
            "native fixture requires no real channel service"
        );
        let root =
            std::env::temp_dir().join(format!("channel-registration-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("configs/channels")).unwrap();
        std::fs::create_dir_all(root.join("docker/config/channels")).unwrap();
        let raw = format!(
            "[{}]\nenabled = false\napp_id = \"\"\napp_secret = \"\"\n",
            platform.channel()
        );
        for path in ["configs/channels", "docker/config/channels"] {
            std::fs::write(
                root.join(path).join(format!("{}.toml", platform.channel())),
                &raw,
            )
            .unwrap();
        }
        // A failed old implementation may attempt a shell; no launch script or
        // channel executable exists in this owned workspace.
        assert!(!root.join("component_start").exists());
        let mut state = AppState::test_default_with_fixture_provider();
        state.core.db = Self::database_pool(&root);
        let mut state = state.with_seeded_db_schema();
        state.skill_rt.workspace_root = root.clone();
        state.core.http_client = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap();
        state.seed_test_auth_identity(ADMIN_KEY, "admin");
        state.seed_test_auth_identity("fixture-channel-user", "user");
        let session_id = {
            let mut db = state.core.db.get().unwrap();
            let expiry = current_unix_ts().saturating_add(600).to_string();
            let session = create_pending_channel_bind_session(
                &mut db,
                platform.channel(),
                ADMIN_KEY,
                &expiry,
            )
            .unwrap();
            attach_pending_channel_bind_session_install_flow(
                &mut db,
                session.id,
                "fixture-device-code",
                "https://registration.example.test/verify",
                1,
                &expiry,
            )
            .unwrap();
            session.id
        };
        {
            let digest = channel_configuration_fingerprint(&state, platform).unwrap();
            let mut db = state.core.db.get().unwrap();
            let session = get_pending_channel_bind_session_by_id(&db, session_id)
                .unwrap()
                .unwrap();
            crate::repo::channel_registration::prepare_registration(&mut db, &session, &digest)
                .unwrap();
            crate::repo::channel_registration::record_registration_device(
                &mut db,
                &session,
                "fixture-device-code",
                &digest,
            )
            .unwrap();
        }
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Semaphore::new(0));
        let requests = Arc::new(AtomicUsize::new(0));
        let server_state = RegistrationServer {
            entered: entered.clone(),
            release: release.clone(),
            requests: requests.clone(),
            result,
            action,
        };
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/oauth/v1/app/registration", post(registration))
            .with_state(server_state);
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let env_name = format!("APP_{}", platform.accounts_env());
        let prior_env = std::env::var_os(&env_name);
        std::env::set_var(&env_name, &endpoint);
        assert_eq!(official_accounts_base_url(platform), endpoint);
        Self {
            root,
            state,
            platform,
            session_id,
            entered,
            release,
            requests,
            server,
            env_name,
            prior_env,
        }
    }

    pub(super) fn headers(key: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert("x-agent-key", HeaderValue::from_str(key).unwrap());
        headers
    }

    fn database_pool(root: &std::path::Path) -> r2d2::Pool<r2d2_sqlite::SqliteConnectionManager> {
        let manager = r2d2_sqlite::SqliteConnectionManager::file(root.join("registration.db"))
            .with_init(|connection| {
                connection.pragma_update(None, "foreign_keys", "ON")?;
                connection.busy_timeout(Duration::from_secs(10))?;
                Ok(())
            });
        r2d2::Pool::builder().max_size(2).build(manager).unwrap()
    }

    pub(super) fn reopen_database(&mut self) {
        let placeholder = r2d2::Pool::builder()
            .max_size(1)
            .build(r2d2_sqlite::SqliteConnectionManager::memory())
            .unwrap();
        drop(std::mem::replace(&mut self.state.core.db, placeholder));
        self.state.core.db = Self::database_pool(&self.root);
    }

    pub(super) fn poll(
        &self,
    ) -> tokio::task::JoinHandle<(
        StatusCode,
        Json<ApiResponse<FeishuBindSessionStatusResponse>>,
    )> {
        let state = self.state.clone();
        let platform = self.platform;
        let id = self.session_id;
        tokio::spawn(async move {
            get_channel_bind_session(state, Self::headers(ADMIN_KEY), id, platform).await
        })
    }

    pub(super) async fn wait_for_poll(&self) {
        self.wait_for_response_gate().await;
    }

    pub(super) async fn wait_for_response_gate(&self) {
        tokio::time::timeout(Duration::from_secs(15), self.entered.notified())
            .await
            .unwrap();
    }

    pub(super) fn finish_poll(&self) {
        self.release_response();
    }

    pub(super) fn release_response(&self) {
        self.release.add_permits(1);
    }
    pub(super) fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    pub(super) fn begin(
        &self,
    ) -> tokio::task::JoinHandle<(
        StatusCode,
        Json<ApiResponse<FeishuBindSessionStatusResponse>>,
    )> {
        let state = self.state.clone();
        let platform = self.platform;
        tokio::spawn(async move {
            start_channel_bind_session(
                state,
                Self::headers(ADMIN_KEY),
                StartFeishuBindSessionRequest {
                    expires_in_seconds: Some(600),
                },
                platform,
            )
            .await
        })
    }

    pub(super) fn latest_session(&self) -> PendingChannelBindSession {
        let db = self.state.core.db.get().unwrap();
        let id: i64 = db
            .query_row(
                "SELECT MAX(id) FROM pending_channel_bind_sessions",
                [],
                |row| row.get(0),
            )
            .unwrap();
        get_pending_channel_bind_session_by_id(&db, id)
            .unwrap()
            .unwrap()
    }

    pub(super) fn snapshot(&self) -> Vec<Vec<u8>> {
        ["configs/channels", "docker/config/channels"]
            .into_iter()
            .map(|path| {
                std::fs::read(
                    self.root
                        .join(path)
                        .join(format!("{}.toml", self.platform.channel())),
                )
                .unwrap()
            })
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        if let Some(value) = &self.prior_env {
            std::env::set_var(&self.env_name, value);
        } else {
            std::env::remove_var(&self.env_name);
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
