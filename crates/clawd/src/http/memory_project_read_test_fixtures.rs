use super::super::memory_read_race_fixtures::Setup;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use std::path::PathBuf;
use std::time::Duration;
use tower::ServiceExt;

struct OwnedDir(PathBuf);

impl OwnedDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "memory-project-read-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
}

impl Drop for OwnedDir {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove the owned project workspace fixture");
    }
}

pub(super) struct ProjectFixture {
    // Drop the isolated runtime and its DB before the owned workspace directory.
    pub(super) setup: Setup,
    root: OwnedDir,
}

impl ProjectFixture {
    pub(super) fn new(admin: bool) -> Self {
        let mut setup = Setup::new(admin);
        let root = OwnedDir::new();
        let workspace = root.0.join("current");
        std::fs::create_dir_all(workspace.join(".git")).unwrap();
        setup.fixture.state_mut().skill_rt.workspace_root = workspace;
        Self { setup, root }
    }

    pub(super) fn initialize(&self) -> String {
        crate::memory::project_identity::resolve_project_identity(
            &self.setup.direct,
            &self.setup.fixture.state().skill_rt.workspace_root,
        )
        .expect("explicit admitted project initialization")
        .project_ref
    }

    pub(super) fn late_mapping(&self) -> String {
        let late = self.root.0.join("late");
        std::fs::create_dir_all(late.join(".git")).unwrap();
        let project =
            crate::memory::project_identity::resolve_project_identity(&self.setup.direct, &late)
                .unwrap();
        crate::memory::project_identity::link_project_path_alias(
            &self.setup.direct,
            &project.project_ref,
            &self.setup.fixture.state().skill_rt.workspace_root,
        )
        .unwrap();
        project.project_ref
    }

    pub(super) fn writable_pool(&mut self) {
        let path = self
            .setup
            .direct
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        let manager = r2d2_sqlite::SqliteConnectionManager::file(path).with_init(
            |db: &mut rusqlite::Connection| {
                db.busy_timeout(Duration::from_secs(5))?;
                db.pragma_update(None, "foreign_keys", "ON")?;
                Ok(())
            },
        );
        self.setup.fixture.state_mut().core.db =
            r2d2::Pool::builder().max_size(1).build(manager).unwrap();
    }

    pub(super) fn count(&self, table: &str) -> i64 {
        assert!(matches!(
            table,
            "memory_project_identities" | "memory_project_aliases"
        ));
        self.setup
            .direct
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    pub(super) fn migration_count(&self) -> i64 {
        self.setup.direct.query_row(
            "SELECT COUNT(*) FROM runtime_schema_migrations WHERE migration_id='008_memory_context_identity_settings_v1'",
            [], |row| row.get(0),
        ).unwrap()
    }

    pub(super) fn get(&self) -> (StatusCode, serde_json::Value) {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let request = Request::builder()
                .uri("/memory/projects/current")
                .header(
                    claw_core::product_identity::AUTH_KEY_HEADER,
                    &self.setup.fixture.key,
                )
                .body(Body::empty())
                .unwrap();
            let response = super::super::memory_routes::router()
                .with_state(self.setup.fixture.state().clone())
                .oneshot(request)
                .await
                .unwrap();
            let status = response.status();
            let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            (status, serde_json::from_slice(&body).unwrap())
        })
    }

    pub(super) fn assert_scope(
        &self,
        status: StatusCode,
        reply: &serde_json::Value,
        expected: &str,
    ) {
        assert_eq!(status, StatusCode::OK, "{reply}");
        assert_eq!(reply["ok"], true);
        assert_eq!(reply["data"]["kind"], "project");
        assert_eq!(reply["data"]["scope_ref"], expected);
        assert_eq!(reply["data"]["principal_id"], self.setup.fixture.principal);
        self.assert_no_private_metadata(reply);
    }

    pub(super) fn assert_no_private_metadata(&self, reply: &serde_json::Value) {
        let rendered = reply.to_string();
        for secret in [
            &self.setup.fixture.key,
            &self.setup.sibling_key,
            &self.setup.other_key,
        ] {
            assert!(!rendered.contains(secret));
        }
        assert!(!rendered.contains("canonical_locator"));
        assert!(!rendered.contains(&self.root.0.to_string_lossy().to_string()));
    }
}
