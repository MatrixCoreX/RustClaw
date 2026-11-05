// Shared, exclusively owned fixtures for the native settings tests.
use super::super::*;
use axum::{
    body::{to_bytes, Body},
    http::{Method, Request},
};
use tower::ServiceExt;

pub(in crate::http::ui_routes) const ADMIN: &str = "fixture-settings-admin";
pub(in crate::http::ui_routes) const USER: &str = "fixture-settings-user";
pub(in crate::http::ui_routes) const SECRET: &str = "fixture_settings_original_secret_123456789";
pub(super) const NEW_SECRET: &str = "fixture_settings_replacement_secret_987654321";
pub(super) const FILES: [&str; 5] = [
    "config.toml",
    "image.toml",
    "audio.toml",
    "video.toml",
    "music.toml",
];
pub(super) const MODELS: &str = "/v1/admin/model-config";
pub(super) const KEYS: &str = "/v1/admin/provider-keys";

pub(in crate::http::ui_routes) struct Fixture {
    pub state: AppState,
    pub root: PathBuf,
}
impl Fixture {
    pub fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("agent-model-settings-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let mut state = AppState::test_default_with_fixture_provider();
        state.core.db = r2d2::Pool::builder()
            .max_size(1)
            .build(r2d2_sqlite::SqliteConnectionManager::memory())
            .unwrap();
        state.skill_rt.workspace_root = root.clone();
        let fixture = Self {
            state: state.with_seeded_db_schema(),
            root,
        };
        fs::create_dir(fixture.root.join("configs")).unwrap();
        fixture.state.seed_test_auth_identity(ADMIN, "admin");
        fixture.state.seed_test_auth_identity(USER, "user");
        fixture.write("config.toml",&format!("# fixture sentinel\n[llm]\nselected_vendor='custom'\nselected_model='fixture-model'\n[llm.custom]\nmodel='fixture-model'\napi_key='{SECRET}'\n[sentinel]\nkeep='main'\n"));
        for (name, section) in [
            ("image.toml", "image_vision"),
            ("audio.toml", "audio_transcribe"),
            ("video.toml", "video_generation"),
            ("music.toml", "music_generation"),
        ] {
            fixture.write(name,&format!("# fixture sentinel\n[{section}]\ndefault_vendor='fixture'\ndefault_model='fixture-model'\n[{section}.providers.fixture]\nbase_url='https://fixture.invalid/v1'\napi_key='{SECRET}'\n[sentinel]\nkeep='{name}'\n"));
        }
        fixture
    }
    pub fn write(&self, name: &str, contents: &str) {
        fs::write(self.root.join("configs").join(name), contents).unwrap();
    }
    pub fn files(&self) -> BTreeMap<String, Option<Vec<u8>>> {
        FILES
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    fs::read(self.root.join("configs").join(name)).ok(),
                )
            })
            .collect()
    }
    pub fn sql(&self, sql: &str) {
        self.state
            .core
            .db
            .get()
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }
    pub async fn api(
        &self,
        key: &str,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let app = Router::new()
            .nest("/v1", build_ui_router())
            .with_state(self.state.clone());
        let mut request = Request::builder().method(method).uri(path);
        if !key.is_empty() {
            request = request.header(claw_core::product_identity::AUTH_KEY_HEADER, key);
        }
        let request = if let Some(body) = body {
            request
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap()
        } else {
            request.body(Body::empty()).unwrap()
        };
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }
    pub fn assert_private(&self, response: &Value) {
        let raw = response.to_string();
        for text in [SECRET, NEW_SECRET, self.root.to_str().unwrap()] {
            assert!(!raw.contains(text), "private configuration exposed");
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn model_payload() -> Value {
    let mut payload = json!({"llm":{"vendor":"custom","model":"fixture-next"}});
    for section in [
        "image_vision",
        "audio_transcribe",
        "video_generation",
        "music_generation",
    ] {
        payload[section] = json!({"vendor":"fixture","model":"fixture-next"});
    }
    payload
}
pub(super) fn key_payload() -> Value {
    json!({"llm":{"custom":NEW_SECRET},"image":{"image_vision":{"fixture":NEW_SECRET}},
        "audio":{"audio_transcribe":{"fixture":NEW_SECRET}},"video":{"video_generation":{"fixture":NEW_SECRET}},
        "music":{"music_generation":{"fixture":NEW_SECRET}}})
}
pub(in crate::http::ui_routes) fn assert_error(
    response: &(StatusCode, Value),
    status: StatusCode,
    code: &str,
) {
    assert_eq!(response.0, status);
    assert_eq!(response.1, json!({"ok":false,"data":null,"error":code}));
}
