use super::*;
use crate::child_task_contract::{
    ChildTaskBudget, ChildTaskMergePolicy, ChildTaskPermissionProfile, ChildTaskSpec,
};

fn spec(
    parent: &str,
    child: &str,
    permission_profile: ChildTaskPermissionProfile,
    scope: Value,
) -> ChildTaskSpec {
    ChildTaskSpec {
        parent_task_id: parent.to_string(),
        child_task_id: child.to_string(),
        role: "worker".to_string(),
        scope,
        permission_profile,
        required: true,
        budget: ChildTaskBudget::readonly_default(),
        result_contract: json!({"output_format": "machine_json"}),
        merge_policy: ChildTaskMergePolicy::StructuredFindings,
    }
}

#[test]
fn graph_rejects_cycles_and_missing_dependencies() {
    let cyclic = vec![
        spec(
            "parent",
            "a",
            ChildTaskPermissionProfile::ReadOnly,
            json!({"dependencies": ["b"]}),
        ),
        spec(
            "parent",
            "b",
            ChildTaskPermissionProfile::ReadOnly,
            json!({"dependencies": ["a"]}),
        ),
    ];
    assert_eq!(
        prepare_child_task_graph(&cyclic, 2)
            .expect_err("cycle must fail")
            .to_string(),
        "child_graph_cycle"
    );
    let missing = vec![spec(
        "parent",
        "a",
        ChildTaskPermissionProfile::ReadOnly,
        json!({"dependencies": ["missing"]}),
    )];
    assert_eq!(
        prepare_child_task_graph(&missing, 2)
            .expect_err("missing dependency must fail")
            .to_string(),
        "child_graph_dependency_missing"
    );
}

#[test]
fn disjoint_writers_are_ready_and_overlapping_writers_are_serialized() {
    let disjoint = vec![
        spec(
            "parent",
            "writer-a",
            ChildTaskPermissionProfile::LocalWorktree,
            json!({"owned_paths": ["crates/a"]}),
        ),
        spec(
            "parent",
            "writer-b",
            ChildTaskPermissionProfile::LocalWorktree,
            json!({"owned_paths": ["crates/b"]}),
        ),
    ];
    let graph = prepare_child_task_graph(&disjoint, 2).expect("disjoint graph");
    assert!(graph.edges.is_empty());
    assert!(graph.nodes.iter().all(|node| node.readiness == "ready"));

    let overlapping = vec![
        spec(
            "parent",
            "writer-a",
            ChildTaskPermissionProfile::LocalWorktree,
            json!({"owned_paths": ["crates/shared"]}),
        ),
        spec(
            "parent",
            "writer-b",
            ChildTaskPermissionProfile::LocalWorktree,
            json!({"owned_paths": ["crates/shared/src"]}),
        ),
    ];
    let graph = prepare_child_task_graph(&overlapping, 2).expect("serialized graph");
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].0, "writer-a");
    assert_eq!(graph.edges[0].1, "writer-b");
    assert_eq!(graph.edges[0].3, "path_ownership_serialization");
    assert_eq!(graph.nodes[0].readiness, "ready");
    assert_eq!(graph.nodes[1].readiness, "blocked_dependency");
}

#[test]
fn persisted_graph_promotes_successors_and_cancels_required_dependents() {
    let mut db = Connection::open_in_memory().expect("open db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("task schema");
    ensure_child_task_graph_schema(&db).expect("graph schema");
    let specs = vec![
        spec(
            "parent",
            "writer",
            ChildTaskPermissionProfile::LocalWorktree,
            json!({"owned_paths": ["src"]}),
        ),
        spec(
            "parent",
            "reviewer",
            ChildTaskPermissionProfile::ReadOnly,
            json!({"dependencies": ["writer"]}),
        ),
    ];
    let graph = prepare_child_task_graph(&specs, 2).expect("prepare graph");
    let tx = db.transaction().expect("transaction");
    persist_child_task_graph(&tx, &graph, "1").expect("persist graph");
    for child in ["writer", "reviewer"] {
        tx.execute(
            "INSERT INTO tasks(task_id, status, updated_at) VALUES (?1, 'queued', '1')",
            params![child],
        )
        .expect("insert task");
    }
    tx.commit().expect("commit");

    let steering = record_child_graph_steering(
        &db,
        "reviewer",
        Some("checkpoint-1"),
        "user_followup",
        Some("revised objective"),
        Some(&json!({"allowed_capabilities": ["filesystem.read_text_range"]})),
        "2",
    )
    .expect("record steering")
    .expect("steering directive");
    assert_eq!(steering["steering_version"], 1);

    record_child_graph_terminal(&db, "parent", "writer", "succeeded", "2").expect("record success");
    let snapshot = graph_snapshot(&db, "parent")
        .expect("snapshot")
        .expect("graph");
    let reviewer = snapshot["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["child_task_id"] == "reviewer")
        .expect("reviewer node");
    assert_eq!(reviewer["readiness"], "ready");
    assert_eq!(reviewer["steering_version"], 1);
    assert_eq!(reviewer["steering"]["checkpoint_id"], "checkpoint-1");

    db.execute(
        "UPDATE child_task_graph_nodes SET readiness = 'blocked_dependency'
         WHERE child_task_id = 'reviewer'",
        [],
    )
    .expect("reset reviewer");
    db.execute(
        "UPDATE child_task_graph_nodes SET readiness = 'failed'
         WHERE child_task_id = 'writer'",
        [],
    )
    .expect("fail writer");
    record_child_graph_terminal(&db, "parent", "writer", "failed", "3").expect("record failure");
    let status: String = db
        .query_row(
            "SELECT status FROM tasks WHERE task_id = 'reviewer'",
            [],
            |row| row.get(0),
        )
        .expect("reviewer status");
    assert_eq!(status, "canceled");
}

#[test]
fn restart_reconciliation_uses_task_rows_to_release_successor() {
    let mut db = Connection::open_in_memory().expect("open db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            status TEXT NOT NULL,
            result_json TEXT,
            error_text TEXT,
            lease_owner TEXT,
            lease_expires_at INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("task schema");
    ensure_child_task_graph_schema(&db).expect("graph schema");
    let specs = vec![
        spec(
            "parent",
            "writer",
            ChildTaskPermissionProfile::LocalWorktree,
            json!({"owned_paths": ["src"]}),
        ),
        spec(
            "parent",
            "reviewer",
            ChildTaskPermissionProfile::ReadOnly,
            json!({"dependencies": ["writer"]}),
        ),
    ];
    let graph = prepare_child_task_graph(&specs, 2).expect("prepare graph");
    let tx = db.transaction().expect("transaction");
    persist_child_task_graph(&tx, &graph, "1").expect("persist graph");
    tx.execute(
        "INSERT INTO tasks(task_id, status, updated_at)
         VALUES ('parent', 'running', '1')",
        [],
    )
    .expect("insert parent");
    tx.execute(
        "INSERT INTO tasks(task_id, status, result_json, updated_at)
         VALUES ('writer', 'succeeded', '{}', '2')",
        [],
    )
    .expect("insert terminal writer");
    tx.execute(
        "INSERT INTO tasks(task_id, status, updated_at)
         VALUES ('reviewer', 'queued', '1')",
        [],
    )
    .expect("insert reviewer");
    tx.execute(
        "UPDATE child_task_graph_nodes SET readiness = 'running'
         WHERE child_task_id = 'writer'",
        [],
    )
    .expect("simulate crash gap");
    tx.commit().expect("commit crash state");

    assert_eq!(
        reconcile_child_task_graphs_after_restart(&db, "3").expect("reconcile restart"),
        1
    );
    let snapshot = graph_snapshot(&db, "parent")
        .expect("snapshot")
        .expect("graph");
    let nodes = snapshot["nodes"].as_array().expect("nodes");
    let writer = nodes
        .iter()
        .find(|node| node["child_task_id"] == "writer")
        .expect("writer");
    let reviewer = nodes
        .iter()
        .find(|node| node["child_task_id"] == "reviewer")
        .expect("reviewer");
    assert_eq!(writer["readiness"], "succeeded");
    assert_eq!(reviewer["readiness"], "ready");
}
