use super::*;
use crate::memory::capacity_test_fixtures::Fixture;

pub(super) struct Setup {
    pub(super) fixture: Fixture,
    pub(super) actor: AuthIdentity,
    pub(super) other: AuthIdentity,
    pub(super) sibling: String,
}

pub(super) fn policy(chunk_bytes: u64) -> UploadPolicy {
    UploadPolicy {
        chunk_bytes,
        parser_lease_seconds: 10,
    }
}

impl Setup {
    pub(super) fn new(file: bool) -> Self {
        let fixture = if file {
            Fixture::file()
        } else {
            Fixture::memory()
        };
        let actor = crate::resolve_auth_identity_by_key(fixture.state(), &fixture.key)
            .unwrap()
            .unwrap();
        let key = crate::repo::auth::create_auth_key(fixture.state(), "user").unwrap();
        let other = crate::resolve_auth_identity_by_key(fixture.state(), &key)
            .unwrap()
            .unwrap();
        let sibling = format!("upload-fixture-sibling-{}", uuid::Uuid::new_v4().simple());
        {
            let db = fixture.state().core.db.get().unwrap();
            db.execute(
                "INSERT INTO auth_keys(user_key,role,enabled,created_at,principal_id)
                VALUES(?1,'user',1,'1',?2)",
                params![sibling, actor.principal_id],
            )
            .unwrap();
            db.execute("INSERT INTO credential_bindings(binding_id,credential_digest,principal_id,status,created_at)
                VALUES(?1,?2,?3,'active','1')",params![uuid::Uuid::new_v4().simple().to_string(),
                    crate::repo::auth::credential_digest(&sibling),actor.principal_id]).unwrap();
        }
        Self {
            fixture,
            actor,
            other,
            sibling,
        }
    }

    pub(super) fn upload(&self, bytes: &[u8], chunk: u64) -> UploadStatus {
        let db = self.fixture.state().core.db.get().unwrap();
        create(
            &db,
            &self.actor,
            &CreateUpload {
                file_digest: digest(bytes),
                total_bytes: bytes.len() as u64,
            },
            policy(chunk),
            100,
        )
        .unwrap()
    }

    pub(super) fn put_all(&self, id: &str, bytes: &[u8], chunk: usize) {
        let db = self.fixture.state().core.db.get().unwrap();
        let mut offset = 0;
        for value in bytes.chunks(chunk) {
            append(&db, &self.actor, id, offset, &digest(value), value, 101).unwrap();
            offset += value.len() as u64;
        }
    }
}

pub(super) fn count(db: &Connection, table: &str) -> i64 {
    assert!(matches!(
        table,
        "memory_import_uploads"
            | "memory_import_upload_chunks"
            | "memory_import_upload_items"
            | "memory_import_sessions"
            | "memory_facts"
    ));
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

pub(super) fn error<T>(result: anyhow::Result<T>, expected: UploadError) {
    let error = match result {
        Ok(_) => panic!("expected typed upload rejection"),
        Err(error) => error,
    };
    assert_eq!(
        error.downcast_ref::<UploadError>(),
        Some(&expected),
        "{error:?}"
    );
}

pub(super) fn revoke(db: &Connection, actor: &AuthIdentity, which: &str, other: &AuthIdentity) {
    match which {
        "key" => {
            db.execute(
                "UPDATE auth_keys SET enabled=0 WHERE user_key=?1",
                [&actor.user_key],
            )
            .unwrap();
        }
        "binding" => {
            db.execute(
                "UPDATE credential_bindings SET status='revoked' WHERE credential_digest=?1",
                [crate::repo::auth::credential_digest(&actor.user_key)],
            )
            .unwrap();
        }
        "role" => {
            db.execute(
                "UPDATE auth_keys SET role='admin' WHERE user_key=?1",
                [&actor.user_key],
            )
            .unwrap();
        }
        "principal" => {
            db.execute(
                "UPDATE auth_keys SET principal_id=?1 WHERE user_key=?2",
                params![other.principal_id, actor.user_key],
            )
            .unwrap();
            db.execute(
                "UPDATE credential_bindings SET principal_id=?1 WHERE credential_digest=?2",
                params![
                    other.principal_id,
                    crate::repo::auth::credential_digest(&actor.user_key)
                ],
            )
            .unwrap();
        }
        _ => panic!("unknown fixture identity mutation"),
    }
}
