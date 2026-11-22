use rusqlite::{params, Connection};

// Graph edges survive checkpoint replacement of the parent's result JSON. Read them
// in the cancellation transaction before the graph readiness is made terminal.
pub(super) fn unfinished_child_task_ids(
    db: &Connection,
    parent_task_id: &str,
) -> anyhow::Result<Vec<String>> {
    let mut statement = db.prepare(
        "SELECT node.child_task_id
         FROM child_task_graph_nodes AS node
         JOIN tasks AS task ON task.task_id = node.child_task_id
         WHERE node.parent_task_id = ?1
           AND task.status IN ('queued', 'running')
         ORDER BY node.created_at, node.child_task_id",
    )?;
    let rows = statement
        .query_map(params![parent_task_id], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}
