use super::*;
use rusqlite::{params, types::Value as SqlValue, Connection};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub(super) enum Action {
    Settings,
    Reindex,
    Pause,
    Resume,
    Cancel,
    LegacyDelete,
    LegacyExpire,
    LegacyClear,
    Link,
    Unlink,
}
struct OwnedDirectory(PathBuf);
impl OwnedDirectory {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("memory-write-fixture-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for OwnedDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub(super) struct Fixture {
    pub(super) state: AppState,
    pub(super) actor: claw_core::types::AuthIdentity,
    pub(super) other: String,
    pub(super) project: String,
    pub(super) fact_id: i64,
    _directory: OwnedDirectory,
}
impl Fixture {
    pub(super) fn new(action: Action) -> Self {
        let base = base::Fixture::new();
        let mut state = base.state;
        let actor = base.actor;
        let directory = OwnedDirectory::new();
        let current = directory.0.join("current");
        let other_path = directory.0.join("previous");
        std::fs::create_dir(&current).unwrap();
        std::fs::create_dir(&other_path).unwrap();
        state.skill_rt.workspace_root = current.clone();
        let (project, fact_id) = {
            let db = state.core.db.get().unwrap();
            for table in ["memory_facts", "memories"] {
                db.execute(
                    &format!("UPDATE {table} SET user_id=?1,chat_id=?2 WHERE principal_id=?3"),
                    params![actor.user_id, actor.chat_id, actor.principal_id],
                )
                .unwrap();
            }
            memory::indexing::rebuild_retrieval_index(&db, &state.policy.memory).unwrap();
            let fact_id = db
                .query_row(
                    "SELECT id FROM memory_facts WHERE memory_id='actor-witness'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            memory::project_identity::resolve_project_identity(&db, &current).unwrap();
            let project = memory::project_identity::resolve_project_identity(&db, &other_path)
                .unwrap()
                .project_ref;
            if matches!(action, Action::Unlink) {
                memory::project_identity::link_project_path_alias(&db, &project, &current).unwrap();
            }
            let profile =
                memory::vector_store::register_configured_profile(&db, &state.policy.memory)
                    .unwrap();
            if matches!(action, Action::Pause | Action::Resume) {
                memory::embedding_jobs::set_profile_paused(
                    &db,
                    &actor.principal_id,
                    &profile.profile_id,
                    matches!(action, Action::Resume),
                )
                .unwrap();
            }
            if matches!(action, Action::Cancel) {
                let settings = memory::settings::resolve_principal_memory_settings(
                    &db,
                    &actor.principal_id,
                    state.policy.memory.long_term_enabled,
                )
                .unwrap();
                let (_, _, rows) = memory::embedding_jobs::enqueue_reindex(
                    &db,
                    &state.policy.memory,
                    &actor.principal_id,
                    &settings.policy_digest,
                    true,
                )
                .unwrap();
                assert!(rows > 0, "cancellation must target real queued source rows");
            }
            (project, fact_id)
        };
        Self {
            state,
            actor,
            other: base.other,
            project,
            fact_id,
            _directory: directory,
        }
    }
    pub(super) fn request(&self, action: Action) -> (&'static str, String, serde_json::Value) {
        let post = |path: &str, body| ("POST", path.to_string(), body);
        match action {
            Action::Settings => post(
                "/memory/settings",
                json!({"scope":"principal","expected_revision":0,"use_mode":"disabled","generate_mode":"disabled","external_context_policy":"exclude"}),
            ),
            Action::Reindex | Action::Pause | Action::Resume | Action::Cancel => {
                let db = self.state.core.db.get().unwrap();
                let settings = memory::settings::resolve_principal_memory_settings(
                    &db,
                    &self.actor.principal_id,
                    self.state.policy.memory.long_term_enabled,
                )
                .unwrap();
                let action = match action {
                    Action::Reindex => "reindex",
                    Action::Pause => "pause",
                    Action::Resume => "resume",
                    _ => "cancel",
                };
                post(
                    &format!("/memory/vector/{action}"),
                    json!({"expected_policy_digest":settings.policy_digest}),
                )
            }
            Action::LegacyDelete => (
                "DELETE",
                format!("/memory/fact:{}", self.fact_id),
                json!({}),
            ),
            Action::LegacyExpire => {
                post(&format!("/memory/fact:{}/expire", self.fact_id), json!({}))
            }
            Action::LegacyClear => post("/memory/clear", json!({"scope":"all"})),
            Action::Link => post(
                "/memory/projects/current/link",
                json!({"project_ref":self.project}),
            ),
            Action::Unlink => post(
                "/memory/projects/current/unlink",
                json!({"project_ref":self.project}),
            ),
        }
    }
    pub(super) fn change_settings(&self) {
        let db = self.state.core.db.get().unwrap();
        let request = serde_json::from_value(
            json!({"scope":"principal","expected_revision":0,"use_mode":"enabled"}),
        )
        .unwrap();
        memory::settings::update_memory_settings(
            &db,
            &self.actor,
            &request,
            self.state.policy.memory.long_term_enabled,
        )
        .unwrap();
    }
    pub(super) fn state(&self) -> Vec<Vec<Vec<SqlValue>>> {
        let db = self.state.core.db.get().unwrap();
        let mut result = base::private_state(&db);
        for table in [
            "memory_runtime_settings",
            "memory_onboarding_state",
            "memory_project_identities",
            "memory_project_aliases",
            "memory_embedding_profiles",
            "memory_embedding_controls",
            "memory_generation_outputs",
        ] {
            result.push(table_rows(&db, table));
        }
        result.push(table_rows(
            &self.state.core.audit_db.get().unwrap(),
            "audit_logs",
        ));
        result
    }
    pub(super) fn assert_success(&self, action: Action, reply: &serde_json::Value) {
        let db = self.state.core.db.get().unwrap();
        match action {
            Action::Settings => {
                assert_eq!(reply["data"]["revision"], 1);
                assert_eq!(reply["data"]["requested"]["use_mode"], "disabled");
            }
            Action::Reindex => {
                assert_eq!(reply["data"]["status"], "queued");
                assert!(reply["data"]["queued_rows"].as_u64().unwrap() > 0);
            }
            Action::Pause | Action::Resume => {
                let profile = memory::vector_store::register_configured_profile(
                    &db,
                    &self.state.policy.memory,
                )
                .unwrap();
                assert_eq!(
                    memory::embedding_jobs::profile_paused(
                        &db,
                        &self.actor.principal_id,
                        &profile.profile_id
                    )
                    .unwrap(),
                    matches!(action, Action::Pause)
                );
            }
            Action::Cancel => {
                assert!(db.query_row("SELECT COUNT(*) FROM memory_embedding_jobs WHERE principal_id=?1 AND cancel_requested=1",[&self.actor.principal_id],|row|row.get::<_,i64>(0)).unwrap()>0);
                assert_eq!(reply["data"]["status"], "cancelled");
            }
            Action::LegacyDelete | Action::LegacyExpire | Action::LegacyClear => {
                let expected = if matches!(action, Action::LegacyExpire) {
                    "expired"
                } else {
                    "deleted"
                };
                assert_eq!(
                    db.query_row(
                        "SELECT status FROM memory_facts WHERE id=?1",
                        [self.fact_id],
                        |row| row.get::<_, String>(0)
                    )
                    .unwrap(),
                    expected
                );
            }
            Action::Link => {
                assert_eq!(reply["data"]["scope_ref"], self.project);
            }
            Action::Unlink => {
                assert_eq!(
                    db.query_row(
                        "SELECT COUNT(*) FROM memory_project_aliases WHERE project_ref=?1",
                        [&self.project],
                        |row| row.get::<_, i64>(0)
                    )
                    .unwrap(),
                    0
                );
            }
        }
    }
}
fn table_rows(db: &Connection, table: &str) -> Vec<Vec<SqlValue>> {
    let mut statement = db
        .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
        .unwrap();
    let columns = statement.column_count();
    let rows = statement
        .query_map([], |row| {
            (0..columns)
                .map(|column| row.get(column))
                .collect::<rusqlite::Result<Vec<SqlValue>>>()
        })
        .unwrap();
    rows.collect::<rusqlite::Result<Vec<_>>>().unwrap()
}
