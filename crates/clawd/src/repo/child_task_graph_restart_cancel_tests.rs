use super::*;
use crate::child_task_contract::{
    ChildTaskBudget, ChildTaskMergePolicy, ChildTaskPermissionProfile, ChildTaskSpec,
};

#[test]
fn restart_repairs_cancelled_graph_with_unfinished_children_only() {
    let mut db = Connection::open_in_memory().expect("open db");
    db.execute_batch(
        "CREATE TABLE tasks (
            task_id TEXT PRIMARY KEY,
            payload_json TEXT NOT NULL DEFAULT '{}', status TEXT NOT NULL, result_json TEXT,
            error_text TEXT, lease_owner TEXT, lease_expires_at INTEGER NOT NULL DEFAULT 0,
            updated_at TEXT NOT NULL
        );",
    )
    .expect("task schema");
    ensure_child_task_graph_schema(&db).expect("graph schema");
    let specs = ["running-child", "queued-child", "completed-child"].map(|id| ChildTaskSpec {
        parent_task_id: "cancelled-parent".to_string(),
        child_task_id: id.to_string(),
        role: "explorer".to_string(),
        scope: json!({}),
        permission_profile: ChildTaskPermissionProfile::ReadOnly,
        required: true,
        budget: ChildTaskBudget::readonly_default(),
        result_contract: json!({"output_format": "machine_json"}),
        merge_policy: ChildTaskMergePolicy::StructuredFindings,
    });
    let graph = prepare_child_task_graph(&specs, 3).expect("prepare graph");
    let tx = db.transaction().expect("transaction");
    persist_child_task_graph(&tx, &graph, "1").expect("persist graph");
    tx.execute_batch(
        "INSERT INTO tasks(task_id,status,result_json,lease_owner,lease_expires_at,updated_at)
         VALUES ('cancelled-parent','canceled','{}',NULL,0,'1'),
                ('running-child','running','{}','old-worker',9999999999,'1'),
                ('queued-child','queued',NULL,NULL,0,'1'),
                ('completed-child','succeeded','{\"preserved\":true}',NULL,0,'1'),
                ('unrelated-child','queued',NULL,NULL,0,'1');",
    )
    .expect("insert pre-restart tasks");
    mark_parent_graph_cancelled(&tx, "cancelled-parent", "2").expect("cancel graph only");
    tx.commit().expect("commit interrupted cancellation");

    assert_eq!(
        reconcile_child_task_graphs_after_restart(&db, "3").expect("repair restart"),
        1
    );
    for child in ["running-child", "queued-child"] {
        let row = db
            .query_row(
                "SELECT status,lease_owner,lease_expires_at FROM tasks WHERE task_id=?1",
                params![child],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Option<String>>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .expect("repaired child");
        assert_eq!(row, ("canceled".to_string(), None, 0));
    }
    assert_eq!(
        db.query_row(
            "SELECT result_json FROM tasks WHERE task_id='completed-child'",
            [],
            |r| r.get::<_, String>(0)
        )
        .expect("completed sibling"),
        "{\"preserved\":true}"
    );
    assert_eq!(
        db.query_row(
            "SELECT status FROM tasks WHERE task_id='unrelated-child'",
            [],
            |r| r.get::<_, String>(0)
        )
        .expect("unrelated task"),
        "queued"
    );
    assert_eq!(
        reconcile_child_task_graphs_after_restart(&db, "4").expect("repeat repair"),
        0
    );
}
