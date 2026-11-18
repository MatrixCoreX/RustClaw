use super::*;
use rusqlite::{params, types::Value as SqlValue, Connection};
use sha2::{Digest, Sha256};

pub(super) struct Fixture {
    pub(super) state: AppState,
    pub(super) actor: claw_core::types::AuthIdentity,
    pub(super) other: String,
}

#[derive(Clone, Copy)]
pub(super) enum Action {
    Correct,
    Feedback,
    Delete,
    Undo,
    Preview,
    Confirm,
    Clear,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let state = AppState::test_default_with_fixture_provider().with_seeded_db_schema();
        let key = crate::repo::auth::create_auth_key(&state, "admin").unwrap();
        let actor = crate::resolve_auth_identity_by_key(&state, &key)
            .unwrap()
            .unwrap();
        let other_key = crate::repo::auth::create_auth_key(&state, "user").unwrap();
        let other = crate::resolve_auth_identity_by_key(&state, &other_key)
            .unwrap()
            .unwrap()
            .principal_id;
        {
            let db = state.core.db.get().unwrap();
            crate::memory::indexing::ensure_retrieval_schema(&db).unwrap();
            crate::memory::ux::ensure_memory_ux_schema(&db).unwrap();
            crate::memory::embedding_jobs::initialize_embedding_runtime(&db, &state.policy.memory)
                .unwrap();
            let sibling = format!("fixture-sibling-{}", uuid::Uuid::new_v4());
            db.execute(
                "INSERT INTO auth_keys(user_key,role,enabled,created_at,principal_id)
                VALUES(?1,'user',1,'1',?2)",
                params![sibling, actor.principal_id],
            )
            .unwrap();
            db.execute("INSERT INTO credential_bindings(binding_id,credential_digest,principal_id,status,created_at)
                VALUES(?1,?2,?3,'active','1')",params![format!("fixture-binding-{}",uuid::Uuid::new_v4()),
                crate::repo::auth::credential_digest(&sibling),actor.principal_id]).unwrap();
            fact(
                &db,
                &key,
                &actor.principal_id,
                "actor-witness",
                "actor-private-text",
            );
            fact(
                &db,
                &other_key,
                &other,
                "other-witness",
                "other-private-text",
            );
            let now = crate::now_ts_u64() as i64;
            db.execute(
                "INSERT INTO memories(memory_id,user_id,chat_id,user_key,principal_id,
                scope_kind,scope_ref,channel,role,content,created_at,created_at_ts,memory_type,
                safety_flag,origin,row_revision,legacy_scope_inferred)
                VALUES('actor-recent',1,7,?1,?2,'principal',?2,'ui','user','recent-private-text',
                '1',?3,'episodic_event','normal','user_confirmed',1,0)",
                params![key, actor.principal_id, now],
            )
            .unwrap();
        }
        Self {
            state,
            actor,
            other,
        }
    }

    pub(super) fn request(&self, action: Action) -> (String, serde_json::Value) {
        use crate::memory::ux;
        let now = crate::now_ts_u64() as i64;
        match action {
            Action::Correct => (
                "/memory/actor-witness/correct".into(),
                json!({"expected_revision":1,"content":"revised private content"}),
            ),
            Action::Feedback => (
                "/memory/actor-witness/feedback".into(),
                json!({"expected_revision":1,"feedback_kind":"do_not_use"}),
            ),
            Action::Delete => (
                "/memory/actor-witness/delete".into(),
                json!({"expected_revision":1}),
            ),
            Action::Undo => {
                let db = self.state.core.db.get().unwrap();
                let result = ux::delete_memory_with_revision(
                    &db,
                    &self.actor.principal_id,
                    "actor-witness",
                    &ux::MemoryMutationRequest {
                        expected_revision: 1,
                    },
                    now,
                )
                .unwrap();
                (
                    "/memory/undo".into(),
                    json!({"revision_id":result.revision_id.unwrap()}),
                )
            }
            Action::Preview | Action::Confirm => {
                let db = self.state.core.db.get().unwrap();
                let mut export = ux::export_memory(&db, &self.actor.principal_id, now).unwrap();
                export.items.retain(|item| item.id == "actor-witness");
                assert_eq!(export.items.len(), 1);
                export.items[0].id = "incoming-witness".into();
                export.items[0].content = "distinct imported private content".into();
                export.checksum = format!(
                    "sha256:{:x}",
                    Sha256::digest(serde_json::to_vec(&export.items).unwrap())
                );
                if matches!(action, Action::Preview) {
                    ("/memory/import/preview".into(), json!({"export":export}))
                } else {
                    let preview = ux::preview_memory_import(
                        &db,
                        &self.actor.principal_id,
                        &ux::MemoryImportPreviewRequest { export },
                        now,
                    )
                    .unwrap();
                    (
                        "/memory/import/confirm".into(),
                        json!({"import_id":preview.import_id,"expected_payload_digest":preview.payload_digest}),
                    )
                }
            }
            Action::Clear => {
                let db = self.state.core.db.get().unwrap();
                let preview =
                    ux::clear_preview(&db, &self.actor.principal_id, "transcript_and_derived")
                        .unwrap();
                (
                    "/memory/clear/scoped".into(),
                    json!({"mode":preview.mode,"expected_transcript_rows":preview.transcript_rows,"expected_derived_rows":preview.derived_rows}),
                )
            }
        }
    }
}

fn fact(db: &Connection, key: &str, owner: &str, id: &str, content: &str) {
    let now = crate::now_ts_u64() as i64;
    db.execute("INSERT INTO memory_facts(memory_id,user_id,chat_id,user_key,principal_id,
        scope_kind,scope_ref,namespace,fact_key,fact_value,fact_text,source_kind,source_ref,
        created_at_ts,updated_at_ts,status,origin,row_revision,legacy_scope_inferred,
        source_refs_json,evidence_refs_json,trust_tier,sensitivity,last_verified_at_ts,modified_at_ts)
        VALUES(?1,1,7,?2,?3,'principal',?3,'fixture-profile',?1,?4,?4,'user_message','fixture',
        ?5,?5,'active','user_confirmed',1,0,'[]','[]','user_confirmed','normal',?5,?5)",params![id,key,owner,content,now]).unwrap();
}

pub(super) fn private_state(db: &Connection) -> Vec<Vec<Vec<SqlValue>>> {
    [
        "memories",
        "memory_facts",
        "user_preferences",
        "memory_revisions",
        "memory_retrieval_feedback",
        "memory_import_sessions",
        "memory_privacy_purge_queue",
        "memory_retrieval_index",
        "memory_embedding_jobs",
        "memory_vector_rows",
        "memory_vector_snapshots",
    ]
    .into_iter()
    .map(|table| {
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
    })
    .collect()
}
