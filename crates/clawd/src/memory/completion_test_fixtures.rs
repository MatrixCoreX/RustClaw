use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use axum::{routing::post, Json, Router};
use rusqlite::params;
use serde_json::json;

use super::capacity_test_fixtures::Fixture;

const USER_CONTENT: &str =
    "I prefer markdown for technical explanations. Please keep this preference for future answers.";

pub(super) struct GenerationFixture {
    pub(super) fixture: Fixture,
    pub(super) task: crate::ClaimedTask,
    pub(super) job_id: String,
    requests: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

impl GenerationFixture {
    pub(super) async fn new() -> Self {
        Self::for_kind("extract").await
    }

    pub(super) async fn for_kind(kind: &str) -> Self {
        let mut fixture = Fixture::file();
        let task = crate::ClaimedTask {
            claim_attempt: 1,
            task_id: format!("generation-test-{}", uuid::Uuid::new_v4().simple()),
            user_id: 1,
            chat_id: 7,
            user_key: Some(fixture.key.clone()),
            channel: "ui".into(),
            external_user_id: None,
            external_chat_id: None,
            kind: "ask".into(),
            payload_json: "{}".into(),
        };
        let output = if kind == "extract" {
            json!({"memory_actions":[{
            "action":"upsert","kind":"preference","scope":"user","key":"response_format",
            "value":"markdown","normalized_value":"markdown","confidence":1.0,
            "ttl_policy":"long_term","expires_at_ts":null,
            "source":{"source_kind":"current_user_message","source_ref":format!("task:{}:user",task.task_id),
                "source_text":USER_CONTENT,"memory_ids":[]},
            "reason":"fixture","risk":{"sensitive":false,"injection_like":false}
        }]}).to_string()
        } else {
            json!({
            "summary":"The user prefers markdown for technical explanations and future answers.",
            "fact_candidates":[{"should_persist":true,"kind":"user_profile_fact",
                "namespace":"user_profile","fact":"The user prefers markdown for technical explanations.",
                "confidence":1.0,"reason":"fixture","fact_key":"response_format",
                "fact_value":"markdown","conflict_group":"","expires_at_ts":null}],
            "knowledge_candidates":[]
        }).to_string()
        };
        let response = json!({"choices":[{"message":{"role":"assistant","content":output},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":11,"completion_tokens":7,"total_tokens":18}});
        let requests = Arc::new(AtomicUsize::new(0));
        let observed = requests.clone();
        let app = Router::new().route(
            "/chat/completions",
            post(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                let response = response.clone();
                async move { Json(response) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        {
            let state = fixture.state_mut();
            state.skill_rt.workspace_root =
                std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
            state.policy.memory.long_term_enabled = true;
            state.policy.memory.enable_preference_extraction = true;
            state.policy.memory.llm_preference_fallback_enabled = true;
            let mut runtime = (*state.core.llm_providers[0]).clone();
            runtime.config.name = "memory-generation-fixture".into();
            runtime.config.provider_type = "openai_compat".into();
            runtime.config.base_url = base_url;
            runtime.config.params.stream = Some(false);
            runtime.config.supports_tools = false;
            runtime.client = reqwest::Client::builder().no_proxy().build().unwrap();
            state.core.active_provider_type = Some("openai_compat".into());
            state.core.llm_providers = vec![Arc::new(runtime)];
        }
        {
            let db = fixture.state().core.db.get().unwrap();
            db.execute(
                "INSERT INTO tasks(task_id, user_id, chat_id, user_key, principal_id,
                channel, kind, payload_json, status, created_at, updated_at)
                VALUES (?1, 1, 7, ?2, ?3, 'ui', 'ask', '{}', 'succeeded', '1', '1')",
                params![task.task_id, fixture.key, fixture.principal],
            )
            .unwrap();
        }
        let initial = super::settings::resolve_task_memory_settings(fixture.state(), &task)
            .unwrap()
            .expect("fresh fixture identity");
        assert!(
            !initial.generate_memory,
            "new-install opt-in remains required"
        );
        let actor = crate::resolve_auth_identity_by_key(fixture.state(), &fixture.key)
            .unwrap()
            .expect("authenticated fixture actor");
        {
            let db = fixture.state().core.db.get().unwrap();
            // Make the same explicit, revision-checked account choice as the UI;
            // do not override the global new-install default or managed policy.
            let choice = super::settings::update_memory_settings(
                &db,
                &actor,
                &super::settings::MemorySettingsUpdateRequest {
                    scope: super::settings::MemorySettingScope::Principal,
                    target_principal_id: None,
                    conversation_id: None,
                    use_mode: Some(super::settings::MemorySettingMode::Enabled),
                    generate_mode: Some(super::settings::MemorySettingMode::Enabled),
                    external_context_policy: None,
                    expected_revision: Some(0),
                    long_term_enabled: None,
                },
                fixture.state().policy.memory.long_term_enabled,
            )
            .expect("normal authenticated memory opt-in");
            assert!(choice.use_memory && choice.generate_memory);
        }
        let settings = super::settings::resolve_task_memory_settings(fixture.state(), &task)
            .expect("fixture settings lookup")
            .expect("fixture settings identity");
        assert!(
            settings.generate_memory,
            "fixture generation must be enabled: {settings:?}"
        );
        let capacity_before = {
            let db = fixture.state().core.db.get().unwrap();
            super::capacity::automatic_generation_availability(&db, &fixture.principal)
                .expect("fixture initial capacity")
        };
        assert_eq!(
            capacity_before,
            super::capacity::GenerationAvailability::Available
        );
        assert!(
            super::eligibility::build_turn_eligibility(&task, &json!({}), &settings)
                .durable_candidate_allowed,
            "fixture source must be eligible"
        );
        let ids = super::jobs::persist_turn_and_enqueue(fixture.state(), &task, USER_CONTENT,
            Some("Markdown output preserves code fences, nested lists, tables, and technical examples for later reference."),
            false).expect("normal durable enqueue");
        if ids.is_empty() {
            let db = fixture.state().core.db.get().unwrap();
            let source_counts: (i64, i64) = db
                .query_row(
                    "SELECT COUNT(*), COUNT(DISTINCT principal_id) FROM memories",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            let capacity_after =
                super::capacity::automatic_generation_availability(&db, &fixture.principal)
                    .expect("fixture capacity after source persistence");
            panic!("real source-turn enqueue must create generation work; sources={source_counts:?}; capacity={capacity_after:?}; settings={settings:?}");
        }
        let job_id = {
            let db = fixture.state().core.db.get().unwrap();
            // Select one existing job kind, still through the real enqueue/worker.
            db.execute(
                "UPDATE memory_jobs SET status = 'cancelled', cancel_requested = 1
                WHERE job_kind != ?1",
                [kind],
            )
            .unwrap();
            db.execute(
                "UPDATE memory_jobs SET not_before_ts = 1 WHERE job_kind = ?1",
                [kind],
            )
            .unwrap();
            db.query_row(
                "SELECT job_id FROM memory_jobs WHERE job_kind = ?1",
                [kind],
                |row| row.get(0),
            )
            .unwrap()
        };
        Self {
            fixture,
            task,
            job_id,
            requests,
            server,
        }
    }

    pub(super) async fn run(&self) -> anyhow::Result<bool> {
        let result =
            super::jobs::run_one_memory_job(self.fixture.state(), "generation-worker").await;
        self.trace_job_state();
        result
    }

    pub(super) async fn run_reopened(&self) -> anyhow::Result<bool> {
        let path: String = self.fixture.state().core.db.get()?.query_row(
            "SELECT file FROM pragma_database_list WHERE name = 'main'",
            [],
            |row| row.get(0),
        )?;
        assert!(
            !path.is_empty(),
            "recovery must reopen an isolated SQLite file"
        );
        let manager = r2d2_sqlite::SqliteConnectionManager::file(path).with_init(
            |db: &mut rusqlite::Connection| {
                db.busy_timeout(std::time::Duration::from_millis(50))?;
                db.pragma_update(None, "foreign_keys", "ON")?;
                Ok(())
            },
        );
        let mut state = self.fixture.state().clone();
        state.core.db = r2d2::Pool::builder().max_size(2).build(manager)?;
        state.metrics = Default::default();
        let result = super::jobs::run_one_memory_job(&state, "reopened-worker").await;
        self.trace_job_state();
        assert_eq!(
            state.task_llm_call_count(&self.task.task_id),
            0,
            "a new worker state must recover from SQLite, not call the provider again"
        );
        result
    }

    fn trace_job_state(&self) {
        let db = self.fixture.state().core.db.get().unwrap();
        let state = db.query_row(
            "SELECT status, error_code, checkpoint_json, attempt FROM memory_jobs WHERE job_id = ?1",
            [&self.job_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?,
                row.get::<_, String>(2)?, row.get::<_, i64>(3)?)),
        );
        match state {
            Ok((status, error_code, checkpoint, attempt)) => eprintln!(
                "memory_completion_fixture_state={}",
                json!({"job_id":&self.job_id,"status":status,
                "error_code":error_code,"checkpoint":checkpoint,"attempt":attempt,
                "provider_requests":self.requests.load(Ordering::SeqCst)})
            ),
            Err(error) => eprintln!("memory_completion_fixture_probe_error={error}"),
        }
    }

    pub(super) fn sql(&self, sql: &str) {
        self.fixture
            .state()
            .core
            .db
            .get()
            .unwrap()
            .execute_batch(sql)
            .unwrap();
    }

    pub(super) fn status(&self) -> String {
        self.fixture
            .state()
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT status FROM memory_jobs WHERE job_id = ?1",
                [&self.job_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    pub(super) fn scalar(&self, sql: &str) -> i64 {
        super::capacity_test_fixtures::scalar(&self.fixture.state().core.db.get().unwrap(), sql)
    }

    pub(super) fn assert_one_provider_call(&self) {
        assert_eq!(self.requests.load(Ordering::SeqCst), 1);
        assert_eq!(
            self.fixture.state().task_llm_call_count(&self.task.task_id),
            1
        );
    }

    pub(super) fn assert_no_provider_call(&self) {
        assert_eq!(self.requests.load(Ordering::SeqCst), 0);
        assert_eq!(
            self.fixture.state().task_llm_call_count(&self.task.task_id),
            0
        );
    }
}

impl Drop for GenerationFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
