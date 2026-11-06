use super::super::*;

pub(super) const OWNER: &str = "fixture-catalog-owner";
pub(super) const OTHER: &str = "fixture-catalog-other";
pub(super) const ADMIN: &str = "fixture-catalog-admin";
pub(super) const TASK: &str = "fixture-catalog-task";
pub(super) const SECRET: &str = "fixture_catalog_private_source_line";

pub(super) struct Fixture {
    pub state: AppState,
    pub root: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("agent-model-catalog-read-{}", uuid::Uuid::new_v4()));
        // Exclusive creation: Drop may remove only the directory we own.
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
        for (key, role) in [(OWNER, "user"), (OTHER, "user"), (ADMIN, "admin")] {
            fixture.state.seed_test_auth_identity(key, role);
        }
        fixture.state.seed_ask_task_row(TASK, 1, 1, "{}");
        fixture.sql("UPDATE tasks SET user_key='fixture-catalog-owner'");
        fixture.write_config("fixture-model");
        fixture
    }

    pub fn write_config(&self, model: &str) {
        self.write(
            "configs/config.toml",
            &format!(
                "[llm]\nselected_vendor = \"fixture\"\nselected_model = \"{model}\"\n\
                 [llm.fixture]\nmodel = \"{model}\"\napi_key = \"{SECRET}\"\n\
                 api_format = \"openai_compat\"\ninput_modalities = [\"text\"]\n"
            ),
        );
    }

    pub fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
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

    pub async fn catalog(&self, key: &str) -> (StatusCode, Value) {
        let (status, Json(response)) =
            get_model_catalog(State(self.state.clone()), headers(key)).await;
        (status, serde_json::to_value(response).unwrap())
    }

    pub async fn debug(&self, key: &str, task: &str, teaching: bool) -> (StatusCode, Value) {
        let (status, Json(response)) = task_debug_detail(
            State(self.state.clone()),
            headers(key),
            AxumPath(task.to_string()),
            Query(TeachingTraceQuery {
                teaching: Some(teaching),
                ..Default::default()
            }),
        )
        .await;
        (status, serde_json::to_value(response).unwrap())
    }

    pub fn assert_private(&self, response: &Value) {
        let public = response.to_string();
        for forbidden in [SECRET, self.root.to_str().unwrap(), "error_detail"] {
            assert!(!public.contains(forbidden), "private field exposed");
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(super) fn headers(key: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if !key.is_empty() {
        headers.insert(
            claw_core::product_identity::AUTH_KEY_HEADER,
            axum::http::HeaderValue::from_str(key).unwrap(),
        );
    }
    headers
}

pub(super) fn assert_error(response: &(StatusCode, Value), status: StatusCode, code: &str) {
    assert_eq!(response.0, status);
    assert_eq!(response.1["ok"], false);
    assert_eq!(response.1["error"], code);
}

pub(super) fn isolated_process_case(full_name: &str, name: &str, values: &[(&str, &str)]) -> bool {
    const CHILD: &str = "APP_MODEL_CATALOG_FIXTURE_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        return false;
    }
    let mut command = std::process::Command::new(std::env::current_exe().unwrap());
    command.env_clear().env(CHILD, name).args([
        "--exact",
        full_name,
        "--nocapture",
        "--test-threads=1",
    ]);
    for name in [
        "PATH",
        "LD_LIBRARY_PATH",
        "DYLD_LIBRARY_PATH",
        "DYLD_FALLBACK_LIBRARY_PATH",
        "SYSTEMROOT",
        "TMPDIR",
        "TEMP",
        "APP_PRODUCT_IDENTITY_CONFIG",
    ] {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
    let output = command.envs(values.iter().copied()).output().unwrap();
    assert!(
        output.status.success(),
        "isolated catalogue case {name} failed\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("running 1 test"));
    assert!(stdout.contains("test result: ok. 1 passed; 0 failed; 0 ignored;"));
    true
}
