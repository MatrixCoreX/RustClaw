use crate::{task_result_retention::DeliveryCleanupEnrollment, AppState};
use serde_json::{json, Value};

pub(crate) struct Fixture {
    pub(crate) state: AppState,
    pub(crate) root: std::path::PathBuf,
    pub(crate) task_id: String,
    pub(crate) enrollment: DeliveryCleanupEnrollment,
}
impl Fixture {
    pub(crate) fn new() -> Self {
        let root =
            std::env::temp_dir().join(format!("agent-delivery-retention-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let mut state = AppState::test_default_with_fixture_provider();
        state.skill_rt.workspace_root = root.clone();
        state.core.db.get().unwrap().execute_batch("CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY, kind TEXT NOT NULL, status TEXT NOT NULL,
            idempotency_key TEXT UNIQUE, payload_json TEXT NOT NULL, result_json TEXT, error_text TEXT);").unwrap();
        Self {
            state,
            root,
            task_id: uuid::Uuid::new_v4().to_string(),
            enrollment: DeliveryCleanupEnrollment {
                schema_version: 1,
                work_id: format!("nni-work-{}", uuid::Uuid::new_v4().simple()),
                attempt: 1,
                capability: "fixture.download".to_string(),
                origin: "https://media.example.org".to_string(),
                job_id: uuid::Uuid::new_v4().to_string(),
                payload_sha256: "a".repeat(64),
            },
        }
    }
    pub(crate) fn payload(&self) -> Value {
        json!({"_task_source":"distributed_work","_requested_capability":self.enrollment.capability,
            "_delivery_cleanup":self.enrollment,"args":{"share":"private-fixture-share"}})
    }
    pub(crate) fn insert(&self, task_id: &str, kind: &str, key: &str, payload: &Value) {
        self.state.core.db.get().unwrap().execute(
            "INSERT INTO tasks(task_id,kind,status,idempotency_key,payload_json,result_json,error_text) VALUES(?1,?2,'succeeded',?3,?4,?5,?6)",
            rusqlite::params![task_id,kind,key,payload.to_string(),json!({"text":"original-result","nested":{"output":"private-article"}}).to_string(),"private-error-detail"]).unwrap();
    }
    pub(crate) fn enroll(&self) {
        crate::task_result_retention::register_enrollment(
            &self.root,
            &self.task_id,
            &self.enrollment,
        )
        .unwrap();
    }
    pub(crate) fn row(&self, task_id: &str) -> (String, String, Option<String>) {
        self.state
            .core
            .db
            .get()
            .unwrap()
            .query_row(
                "SELECT payload_json,result_json,error_text FROM tasks WHERE task_id=?1",
                [task_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
