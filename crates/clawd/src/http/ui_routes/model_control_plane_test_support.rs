use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

use axum::{
    body::{to_bytes, Body},
    http::{HeaderMap, Method, Request, StatusCode},
    Json, Router,
};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::http::ui_routes::build_ui_router;
use crate::AppState;

pub(super) const MANAGED_KEY: &str = "fixture-managed-model-key";
pub(super) const INLINE_KEY: &str = "fixture-inline-model-key";

pub(super) struct Fixture {
    pub root: PathBuf,
    pub endpoint: String,
    pub requests: Arc<AtomicUsize>,
    pub managed_seen: Arc<AtomicUsize>,
    router: Router,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    pub async fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("model-control-plane-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("configs")).unwrap();
        std::fs::write(
            root.join("configs/config.toml"),
            r#"
[llm]
selected_vendor = "minimax"
selected_model = "fixture-model"
[llm.minimax]
base_url = "https://provider.example.test/v1"
api_key = ""
model = "fixture-model"
models = ["fixture-model"]
"#,
        )
        .unwrap();
        claw_core::secrets::model_environment::save(&root, "minimax", MANAGED_KEY).unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let managed_seen = Arc::new(AtomicUsize::new(0));
        let count = requests.clone();
        let seen = managed_seen.clone();
        let app = Router::new().fallback(move |headers: HeaderMap| {
            count.fetch_add(1, Ordering::SeqCst);
            let authorization = headers.get("authorization").and_then(|v| v.to_str().ok());
            if authorization == Some(format!("Bearer {MANAGED_KEY}").as_str()) {
                seen.fetch_add(1, Ordering::SeqCst);
            }
            assert!(authorization == Some(format!("Bearer {MANAGED_KEY}").as_str()) || authorization == Some(format!("Bearer {INLINE_KEY}").as_str()), "fixture_credential_override");
            async { Json(json!({"choices":[{"message":{"role":"assistant","content":"fixture-answer"},"finish_reason":"stop"}]})) }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let mut state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        state.skill_rt.workspace_root = root.clone();
        state.seed_test_auth_identity("fixture-model-admin", "admin");
        state.seed_test_auth_identity("fixture-model-user", "user");
        let router = Router::new()
            .nest("/v1", build_ui_router())
            .with_state(state);
        Self {
            root,
            endpoint,
            requests,
            managed_seen,
            router,
            server,
        }
    }

    pub fn llm_body(&self, key: Option<&str>) -> Value {
        let mut body = json!({"selected_vendor":"minimax", "selected_model":"fixture-model",
            "vendor_base_url":self.endpoint, "vendor_api_format":"openai_compat"});
        if let Some(key) = key {
            body["vendor_api_key"] = json!(key);
        }
        body
    }

    pub fn writes(&self) -> Vec<(&'static str, Value)> {
        vec![
            ("/llm/config", self.llm_body(None)),
            (
                "/admin/model-config",
                json!({"image_generation":{"vendor":"custom", "model":"fixture-model", "base_url":self.endpoint}}),
            ),
            (
                "/admin/provider-keys",
                json!({"llm":{"minimax":"fixture-rotated-model-key"}}),
            ),
        ]
    }

    pub fn snapshot(&self) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
        let mut paths = ["config", "image", "audio", "video", "music"]
            .map(|name| self.root.join(format!("configs/{name}.toml")))
            .to_vec();
        paths.push(claw_core::secrets::model_environment::path(&self.root));
        paths.push(claw_core::git_remote_config::git_credential_store_path(
            &self.root,
        ));
        paths
            .into_iter()
            .map(|path| {
                let value = std::fs::read(&path).ok();
                (path, value)
            })
            .collect()
    }

    pub async fn api(
        &self,
        role: Option<&str>,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(format!("/v1{path}"));
        if let Some(role) = role {
            request = request.header("X-Agent-Key", format!("fixture-model-{role}"));
        }
        let body = match body {
            Some(value) => {
                request = request.header("Content-Type", "application/json");
                Body::from(value.to_string())
            }
            None => Body::empty(),
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 2 * 1024 * 1024)
            .await
            .unwrap();
        (status, serde_json::from_slice(&body).unwrap())
    }
}
