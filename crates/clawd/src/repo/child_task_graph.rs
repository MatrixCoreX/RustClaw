use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Component, Path};

use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde_json::{json, Value};

use crate::child_task_contract::{ChildTaskPermissionProfile, ChildTaskSpec};

pub(crate) const CHILD_TASK_GRAPH_SCHEMA_VERSION: u64 = 1;

const CHILD_TASK_GRAPH_SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS child_task_graphs (
    parent_task_id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL,
    status TEXT NOT NULL,
    max_parallel INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS child_task_graph_nodes (
    parent_task_id TEXT NOT NULL,
    child_task_id TEXT PRIMARY KEY,
    role TEXT NOT NULL,
    required INTEGER NOT NULL,
    readiness TEXT NOT NULL,
    permission_profile TEXT NOT NULL,
    merge_policy TEXT NOT NULL,
    owned_paths_json TEXT NOT NULL,
    budget_json TEXT NOT NULL,
    model_policy_json TEXT NOT NULL,
    tool_policy_json TEXT NOT NULL,
    result_contract_json TEXT NOT NULL,
    steering_version INTEGER NOT NULL DEFAULT 0,
    steering_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_child_task_graph_nodes_parent_readiness
ON child_task_graph_nodes(parent_task_id, readiness, created_at);
CREATE TABLE IF NOT EXISTS child_task_graph_edges (
    parent_task_id TEXT NOT NULL,
    predecessor_task_id TEXT NOT NULL,
    successor_task_id TEXT NOT NULL,
    required INTEGER NOT NULL,
    edge_kind TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY(parent_task_id, predecessor_task_id, successor_task_id)
);
CREATE INDEX IF NOT EXISTS idx_child_task_graph_edges_successor
ON child_task_graph_edges(parent_task_id, successor_task_id);
";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ChildTaskDependency {
    pub(crate) child_task_id: String,
    pub(crate) required: bool,
    pub(crate) edge_kind: String,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedChildTaskNode<'a> {
    pub(crate) spec: &'a ChildTaskSpec,
    pub(crate) readiness: &'static str,
    pub(crate) owned_paths: Vec<String>,
    pub(crate) model_policy: Value,
    pub(crate) tool_policy: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct PreparedChildTaskGraph<'a> {
    pub(crate) parent_task_id: String,
    pub(crate) max_parallel: usize,
    pub(crate) nodes: Vec<PreparedChildTaskNode<'a>>,
    pub(crate) edges: Vec<(String, String, bool, String)>,
}

pub(crate) fn ensure_child_task_graph_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(CHILD_TASK_GRAPH_SCHEMA_SQL)?;
    crate::ensure_column_exists(
        db,
        "child_task_graph_nodes",
        "steering_version",
        "ALTER TABLE child_task_graph_nodes
         ADD COLUMN steering_version INTEGER NOT NULL DEFAULT 0",
    )?;
    crate::ensure_column_exists(
        db,
        "child_task_graph_nodes",
        "steering_json",
        "ALTER TABLE child_task_graph_nodes
         ADD COLUMN steering_json TEXT NOT NULL DEFAULT '{}'",
    )?;
    Ok(())
}

pub(crate) fn prepare_child_task_graph<'a>(
    specs: &'a [ChildTaskSpec],
    max_parallel: usize,
) -> anyhow::Result<PreparedChildTaskGraph<'a>> {
    let parent_task_id = specs
        .first()
        .map(|spec| spec.parent_task_id.clone())
        .ok_or_else(|| anyhow::anyhow!("child_graph_empty"))?;
    let mut known = BTreeSet::new();
    for spec in specs {
        if spec.parent_task_id != parent_task_id {
            anyhow::bail!("child_parent_mismatch");
        }
        if !known.insert(spec.child_task_id.clone()) {
            anyhow::bail!("child_graph_duplicate_node");
        }
    }

    let mut dependencies = BTreeMap::<String, Vec<ChildTaskDependency>>::new();
    let mut ownership = BTreeMap::<String, Vec<String>>::new();
    for spec in specs {
        let parsed_dependencies = dependencies_from_scope(&spec.scope)?;
        if parsed_dependencies
            .iter()
            .any(|dependency| !known.contains(&dependency.child_task_id))
        {
            anyhow::bail!("child_graph_dependency_missing");
        }
        if parsed_dependencies
            .iter()
            .any(|dependency| dependency.child_task_id == spec.child_task_id)
        {
            anyhow::bail!("child_graph_self_dependency");
        }
        dependencies.insert(spec.child_task_id.clone(), parsed_dependencies);
        ownership.insert(spec.child_task_id.clone(), owned_paths_from_scope(spec)?);
    }

    add_writer_serialization_edges(specs, &ownership, &mut dependencies);
    ensure_acyclic(&known, &dependencies)?;

    let mut ready_slots = max_parallel.clamp(1, specs.len().max(1));
    let mut nodes = Vec::with_capacity(specs.len());
    for spec in specs {
        let has_dependencies = dependencies
            .get(&spec.child_task_id)
            .is_some_and(|items| !items.is_empty());
        let readiness = if has_dependencies {
            "blocked_dependency"
        } else if ready_slots > 0 {
            ready_slots -= 1;
            "ready"
        } else {
            "blocked_capacity"
        };
        nodes.push(PreparedChildTaskNode {
            spec,
            readiness,
            owned_paths: ownership.remove(&spec.child_task_id).unwrap_or_default(),
            model_policy: machine_policy(&spec.scope, "model_policy")?,
            tool_policy: machine_policy(&spec.scope, "tool_policy")?,
        });
    }
    let edges = dependencies
        .into_iter()
        .flat_map(|(successor, predecessors)| {
            predecessors.into_iter().map(move |dependency| {
                (
                    dependency.child_task_id,
                    successor.clone(),
                    dependency.required,
                    dependency.edge_kind,
                )
            })
        })
        .collect();
    Ok(PreparedChildTaskGraph {
        parent_task_id,
        max_parallel: max_parallel.clamp(1, specs.len().max(1)),
        nodes,
        edges,
    })
}

pub(crate) fn persist_child_task_graph(
    tx: &Transaction<'_>,
    graph: &PreparedChildTaskGraph<'_>,
    now: &str,
) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO child_task_graphs (
            parent_task_id, schema_version, status, max_parallel, created_at, updated_at
         ) VALUES (?1, ?2, 'active', ?3, ?4, ?4)
         ON CONFLICT(parent_task_id) DO UPDATE SET
            schema_version = excluded.schema_version,
            status = 'active',
            max_parallel = excluded.max_parallel,
            updated_at = excluded.updated_at",
        params![
            graph.parent_task_id,
            CHILD_TASK_GRAPH_SCHEMA_VERSION,
            graph.max_parallel,
            now
        ],
    )?;
    for node in &graph.nodes {
        tx.execute(
            "INSERT INTO child_task_graph_nodes (
                parent_task_id, child_task_id, role, required, readiness,
                permission_profile, merge_policy, owned_paths_json, budget_json,
                model_policy_json, tool_policy_json, result_contract_json,
                steering_version, steering_json, created_at, updated_at
             ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                0, '{}', ?13, ?13
             )",
            params![
                graph.parent_task_id,
                node.spec.child_task_id,
                node.spec.role,
                node.spec.required,
                node.readiness,
                node.spec.permission_profile.as_str(),
                node.spec.merge_policy.as_str(),
                serde_json::to_string(&node.owned_paths)?,
                node.spec.budget.to_json().to_string(),
                node.model_policy.to_string(),
                node.tool_policy.to_string(),
                node.spec.result_contract.to_string(),
                now,
            ],
        )?;
    }
    for (predecessor, successor, required, edge_kind) in &graph.edges {
        tx.execute(
            "INSERT INTO child_task_graph_edges (
                parent_task_id, predecessor_task_id, successor_task_id,
                required, edge_kind, created_at
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                graph.parent_task_id,
                predecessor,
                successor,
                required,
                edge_kind,
                now
            ],
        )?;
    }
    Ok(())
}

pub(crate) fn replace_child_graph_node_for_retry(
    tx: &Transaction<'_>,
    parent_task_id: &str,
    previous_child_task_id: &str,
    child_task_id: &str,
    now: &str,
) -> anyhow::Result<bool> {
    let changed = tx.execute(
        "INSERT INTO child_task_graph_nodes (
            parent_task_id, child_task_id, role, required, readiness,
            permission_profile, merge_policy, owned_paths_json, budget_json,
            model_policy_json, tool_policy_json, result_contract_json,
            steering_version, steering_json, created_at, updated_at
         )
         SELECT parent_task_id, ?3, role, required, 'blocked_dependency',
                permission_profile, merge_policy, owned_paths_json, budget_json,
                model_policy_json, tool_policy_json, result_contract_json,
                0, '{}', ?4, ?4
         FROM child_task_graph_nodes
         WHERE parent_task_id = ?1 AND child_task_id = ?2",
        params![parent_task_id, previous_child_task_id, child_task_id, now],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    tx.execute(
        "INSERT OR IGNORE INTO child_task_graph_edges (
            parent_task_id, predecessor_task_id, successor_task_id,
            required, edge_kind, created_at
         )
         SELECT parent_task_id, predecessor_task_id, ?3,
                required, edge_kind, ?4
         FROM child_task_graph_edges
         WHERE parent_task_id = ?1 AND successor_task_id = ?2",
        params![parent_task_id, previous_child_task_id, child_task_id, now],
    )?;
    tx.execute(
        "INSERT OR IGNORE INTO child_task_graph_edges (
            parent_task_id, predecessor_task_id, successor_task_id,
            required, edge_kind, created_at
         )
         SELECT parent_task_id, ?3, successor_task_id,
                required, edge_kind, ?4
         FROM child_task_graph_edges
         WHERE parent_task_id = ?1 AND predecessor_task_id = ?2",
        params![parent_task_id, previous_child_task_id, child_task_id, now],
    )?;
    tx.execute(
        "DELETE FROM child_task_graph_edges
         WHERE parent_task_id = ?1
           AND (predecessor_task_id = ?2 OR successor_task_id = ?2)",
        params![parent_task_id, previous_child_task_id],
    )?;
    tx.execute(
        "DELETE FROM child_task_graph_nodes
         WHERE parent_task_id = ?1 AND child_task_id = ?2",
        params![parent_task_id, previous_child_task_id],
    )?;
    tx.execute(
        "UPDATE child_task_graphs
         SET status = 'active', updated_at = ?2
         WHERE parent_task_id = ?1",
        params![parent_task_id, now],
    )?;
    Ok(true)
}

pub(crate) fn reconcile_child_task_graph(
    db: &Connection,
    parent_task_id: &str,
    now: &str,
) -> anyhow::Result<Option<Value>> {
    let exists = db
        .query_row(
            "SELECT 1 FROM child_task_graphs WHERE parent_task_id = ?1",
            params![parent_task_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !exists {
        return Ok(None);
    }
    reconcile_graph_readiness(db, parent_task_id, now)?;
    graph_snapshot(db, parent_task_id)
}

pub(crate) fn reconcile_child_task_graphs_after_restart(
    db: &Connection,
    now: &str,
) -> anyhow::Result<usize> {
    let mut stmt = db.prepare(
        "SELECT graph.parent_task_id, task.status
         FROM child_task_graphs graph
         LEFT JOIN tasks task ON task.task_id = graph.parent_task_id
         WHERE graph.status = 'active'
         ORDER BY graph.parent_task_id",
    )?;
    let graphs = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (parent_task_id, parent_status) in &graphs {
        db.execute(
            "UPDATE child_task_graph_nodes
             SET readiness = (
                 SELECT task.status FROM tasks task
                 WHERE task.task_id = child_task_graph_nodes.child_task_id
             ), updated_at = ?2
             WHERE parent_task_id = ?1
               AND EXISTS (
                   SELECT 1 FROM tasks task
                   WHERE task.task_id = child_task_graph_nodes.child_task_id
                     AND task.status IN ('succeeded', 'failed', 'timeout', 'canceled')
               )",
            params![parent_task_id, now],
        )?;
        if matches!(
            parent_status.as_deref(),
            Some("failed" | "timeout" | "canceled")
        ) {
            let graph_status = format!("parent_{}", parent_status.as_deref().unwrap_or_default());
            db.execute(
                "UPDATE child_task_graphs
                 SET status = ?2, updated_at = ?3
                 WHERE parent_task_id = ?1",
                params![parent_task_id, graph_status, now],
            )?;
            db.execute(
                "UPDATE tasks
                 SET status = 'canceled', error_text = NULL,
                     lease_owner = NULL, lease_expires_at = 0, updated_at = ?2
                 WHERE task_id IN (
                     SELECT child_task_id FROM child_task_graph_nodes
                     WHERE parent_task_id = ?1
                       AND readiness NOT IN ('succeeded', 'failed', 'timeout', 'canceled')
                 )
                   AND status IN ('queued', 'running')",
                params![parent_task_id, now],
            )?;
            db.execute(
                "UPDATE child_task_graph_nodes
                 SET readiness = 'canceled', updated_at = ?2
                 WHERE parent_task_id = ?1
                   AND readiness NOT IN ('succeeded', 'failed', 'timeout', 'canceled')",
                params![parent_task_id, now],
            )?;
        } else {
            reconcile_graph_readiness(db, parent_task_id, now)?;
        }
    }
    Ok(graphs.len())
}

pub(crate) fn mark_child_graph_node_running(
    db: &Connection,
    child_task_id: &str,
    now: &str,
) -> anyhow::Result<()> {
    db.execute(
        "UPDATE child_task_graph_nodes
         SET readiness = 'running', updated_at = ?2
         WHERE child_task_id = ?1 AND readiness IN ('ready', 'running')",
        params![child_task_id, now],
    )?;
    Ok(())
}

pub(crate) fn record_child_graph_terminal(
    db: &Connection,
    parent_task_id: &str,
    child_task_id: &str,
    terminal_status: &str,
    now: &str,
) -> anyhow::Result<Option<Value>> {
    let readiness = match terminal_status {
        "succeeded" => "succeeded",
        "failed" => "failed",
        "timeout" => "timeout",
        "canceled" => "canceled",
        _ => return Ok(None),
    };
    let changed = db.execute(
        "UPDATE child_task_graph_nodes
         SET readiness = ?3, updated_at = ?4
         WHERE parent_task_id = ?1 AND child_task_id = ?2",
        params![parent_task_id, child_task_id, readiness, now],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    reconcile_graph_readiness(db, parent_task_id, now)?;
    graph_snapshot(db, parent_task_id)
}

pub(crate) fn record_child_graph_task_terminal(
    db: &Connection,
    child_task_id: &str,
    terminal_status: &str,
    now: &str,
) -> anyhow::Result<Option<Value>> {
    let parent_task_id = db
        .query_row(
            "SELECT parent_task_id FROM child_task_graph_nodes
             WHERE child_task_id = ?1 LIMIT 1",
            params![child_task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(parent_task_id) = parent_task_id else {
        return Ok(None);
    };
    record_child_graph_terminal(db, &parent_task_id, child_task_id, terminal_status, now)
}

pub(crate) fn record_child_graph_steering(
    db: &Connection,
    child_task_id: &str,
    checkpoint_id: Option<&str>,
    resume_trigger: &str,
    user_message: Option<&str>,
    new_constraints: Option<&Value>,
    now: &str,
) -> anyhow::Result<Option<Value>> {
    let row = db
        .query_row(
            "SELECT node.parent_task_id, node.steering_version
             FROM child_task_graph_nodes node
             JOIN child_task_graphs graph
               ON graph.parent_task_id = node.parent_task_id
             WHERE node.child_task_id = ?1
               AND graph.status = 'active'
               AND node.readiness IN
                   ('ready', 'running', 'blocked_dependency', 'blocked_capacity')
             LIMIT 1",
            params![child_task_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((parent_task_id, previous_version)) = row else {
        return Ok(None);
    };
    let steering_version = previous_version.saturating_add(1);
    let directive = json!({
        "schema_version": CHILD_TASK_GRAPH_SCHEMA_VERSION,
        "directive": "steer_child",
        "parent_task_id": parent_task_id,
        "child_task_id": child_task_id,
        "steering_version": steering_version,
        "checkpoint_id": checkpoint_id,
        "resume_trigger": resume_trigger,
        "user_message": user_message,
        "new_constraints": new_constraints,
        "created_at": now,
    });
    let changed = db.execute(
        "UPDATE child_task_graph_nodes
         SET steering_version = ?3, steering_json = ?4, updated_at = ?5
         WHERE child_task_id = ?1 AND steering_version = ?2",
        params![
            child_task_id,
            previous_version,
            steering_version,
            directive.to_string(),
            now
        ],
    )?;
    if changed == 1 {
        Ok(Some(directive))
    } else {
        anyhow::bail!("child_graph_steering_conflict")
    }
}

pub(crate) fn mark_parent_graph_cancelled(
    db: &Connection,
    parent_task_id: &str,
    now: &str,
) -> anyhow::Result<bool> {
    let changed = db.execute(
        "UPDATE child_task_graphs
         SET status = 'canceled', updated_at = ?2
         WHERE parent_task_id = ?1 AND status = 'active'",
        params![parent_task_id, now],
    )?;
    if changed == 0 {
        return Ok(false);
    }
    db.execute(
        "UPDATE child_task_graph_nodes
         SET readiness = 'canceled', updated_at = ?2
         WHERE parent_task_id = ?1
           AND readiness NOT IN ('succeeded', 'failed', 'timeout', 'canceled')",
        params![parent_task_id, now],
    )?;
    Ok(true)
}

pub(crate) fn terminate_parent_graph_children(
    state: &crate::AppState,
    db: &Connection,
    parent_task_id: &str,
    parent_status: &str,
    now: &str,
) -> anyhow::Result<Option<Value>> {
    let graph_status = match parent_status {
        "failed" => "parent_failed",
        "timeout" => "parent_timeout",
        "canceled" => "parent_canceled",
        _ => return Ok(None),
    };
    let changed = db.execute(
        "UPDATE child_task_graphs
         SET status = ?2, updated_at = ?3
         WHERE parent_task_id = ?1 AND status = 'active'",
        params![parent_task_id, graph_status, now],
    )?;
    if changed == 0 {
        return Ok(None);
    }
    let mut stmt = db.prepare(
        "SELECT child_task_id, role, required
         FROM child_task_graph_nodes
         WHERE parent_task_id = ?1
           AND readiness NOT IN ('succeeded', 'failed', 'timeout', 'canceled')
         ORDER BY created_at, child_task_id",
    )?;
    let unfinished = stmt
        .query_map(params![parent_task_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    for (child_task_id, role, required) in unfinished {
        let result = json!({
            "schema_version": CHILD_TASK_GRAPH_SCHEMA_VERSION,
            "source": "child_task_graph",
            "status": "canceled",
            "error_code": graph_status,
            "child_task_result": {
                "schema_version": CHILD_TASK_GRAPH_SCHEMA_VERSION,
                "parent_task_id": parent_task_id,
                "child_task_id": child_task_id,
                "role": role,
                "required": required,
                "status": "cancelled",
                "error_code": graph_status,
                "evidence_refs": [],
                "artifact_refs": [],
                "finding_refs": [],
            }
        });
        let task_changed = db.execute(
            "UPDATE tasks
             SET status = 'canceled', result_json = ?2, error_text = NULL,
                 lease_owner = NULL, lease_expires_at = 0, updated_at = ?3
             WHERE task_id = ?1 AND status IN ('queued', 'running')",
            params![child_task_id, result.to_string(), now],
        )?;
        if task_changed > 0 {
            state.worker.cancel_active_task(&child_task_id);
        }
    }
    db.execute(
        "UPDATE child_task_graph_nodes
         SET readiness = 'canceled', updated_at = ?2
         WHERE parent_task_id = ?1
           AND readiness NOT IN ('succeeded', 'failed', 'timeout', 'canceled')",
        params![parent_task_id, now],
    )?;
    graph_snapshot(db, parent_task_id)
}

pub(crate) fn graph_snapshot(
    db: &Connection,
    parent_task_id: &str,
) -> anyhow::Result<Option<Value>> {
    let graph = db
        .query_row(
            "SELECT schema_version, status, max_parallel, created_at, updated_at
             FROM child_task_graphs WHERE parent_task_id = ?1",
            params![parent_task_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((schema_version, status, max_parallel, created_at, updated_at)) = graph else {
        return Ok(None);
    };
    let mut stmt = db.prepare(
        "SELECT node.child_task_id, node.role, node.required, node.readiness,
                node.permission_profile,
                merge_policy, owned_paths_json, budget_json, model_policy_json,
                tool_policy_json, result_contract_json, steering_version,
                steering_json, task.status, task.result_json
         FROM child_task_graph_nodes node
         LEFT JOIN tasks task ON task.task_id = node.child_task_id
         WHERE node.parent_task_id = ?1
         ORDER BY node.created_at, node.child_task_id",
    )?;
    let nodes = stmt
        .query_map(params![parent_task_id], |row| {
            Ok(json!({
                "child_task_id": row.get::<_, String>(0)?,
                "role": row.get::<_, String>(1)?,
                "required": row.get::<_, bool>(2)?,
                "readiness": row.get::<_, String>(3)?,
                "permission_profile": row.get::<_, String>(4)?,
                "merge_policy": row.get::<_, String>(5)?,
                "owned_paths": parse_json_column(row.get::<_, String>(6)?, json!([])),
                "budget": parse_json_column(row.get::<_, String>(7)?, json!({})),
                "model_policy": parse_json_column(row.get::<_, String>(8)?, json!({})),
                "tool_policy": parse_json_column(row.get::<_, String>(9)?, json!({})),
                "result_contract": parse_json_column(row.get::<_, String>(10)?, json!({})),
                "steering_version": row.get::<_, i64>(11)?,
                "steering": parse_json_column(row.get::<_, String>(12)?, json!({})),
                "task_status": row.get::<_, Option<String>>(13)?,
                "runtime": child_runtime_projection(
                    row.get::<_, Option<String>>(14)?.as_deref()
                ),
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut edge_stmt = db.prepare(
        "SELECT predecessor_task_id, successor_task_id, required, edge_kind
         FROM child_task_graph_edges
         WHERE parent_task_id = ?1
         ORDER BY predecessor_task_id, successor_task_id",
    )?;
    let edges = edge_stmt
        .query_map(params![parent_task_id], |row| {
            Ok(json!({
                "predecessor_task_id": row.get::<_, String>(0)?,
                "successor_task_id": row.get::<_, String>(1)?,
                "required": row.get::<_, bool>(2)?,
                "edge_kind": row.get::<_, String>(3)?,
            }))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(json!({
        "schema_version": schema_version,
        "parent_task_id": parent_task_id,
        "status": status,
        "max_parallel": max_parallel,
        "created_at": created_at,
        "updated_at": updated_at,
        "nodes": nodes,
        "edges": edges,
    })))
}

fn reconcile_graph_readiness(
    db: &Connection,
    parent_task_id: &str,
    now: &str,
) -> anyhow::Result<()> {
    cancel_nodes_with_failed_required_dependencies(db, parent_task_id, now)?;
    let max_parallel = db.query_row(
        "SELECT max_parallel FROM child_task_graphs WHERE parent_task_id = ?1",
        params![parent_task_id],
        |row| row.get::<_, i64>(0),
    )?;
    let occupied = db.query_row(
        "SELECT COUNT(*) FROM child_task_graph_nodes
         WHERE parent_task_id = ?1 AND readiness IN ('ready', 'running')",
        params![parent_task_id],
        |row| row.get::<_, i64>(0),
    )?;
    let mut available = max_parallel.saturating_sub(occupied);
    if available > 0 {
        let mut stmt = db.prepare(
            "SELECT node.child_task_id
             FROM child_task_graph_nodes node
             WHERE node.parent_task_id = ?1
               AND node.readiness IN ('blocked_dependency', 'blocked_capacity')
               AND NOT EXISTS (
                   SELECT 1 FROM child_task_graph_edges edge
                   JOIN child_task_graph_nodes predecessor
                     ON predecessor.child_task_id = edge.predecessor_task_id
                   WHERE edge.parent_task_id = node.parent_task_id
                     AND edge.successor_task_id = node.child_task_id
                     AND predecessor.readiness NOT IN
                         ('succeeded', 'failed', 'timeout', 'canceled')
               )
             ORDER BY node.created_at, node.child_task_id",
        )?;
        let candidates = stmt
            .query_map(params![parent_task_id], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        for child_task_id in candidates {
            if available == 0 {
                break;
            }
            db.execute(
                "UPDATE child_task_graph_nodes
                 SET readiness = 'ready', updated_at = ?3
                 WHERE parent_task_id = ?1 AND child_task_id = ?2
                   AND readiness IN ('blocked_dependency', 'blocked_capacity')",
                params![parent_task_id, child_task_id, now],
            )?;
            available -= 1;
        }
    }
    let non_terminal = db.query_row(
        "SELECT COUNT(*) FROM child_task_graph_nodes
         WHERE parent_task_id = ?1
           AND readiness NOT IN ('succeeded', 'failed', 'timeout', 'canceled')",
        params![parent_task_id],
        |row| row.get::<_, i64>(0),
    )?;
    if non_terminal == 0 {
        db.execute(
            "UPDATE child_task_graphs
             SET status = 'terminal', updated_at = ?2
             WHERE parent_task_id = ?1 AND status = 'active'",
            params![parent_task_id, now],
        )?;
    }
    Ok(())
}

fn cancel_nodes_with_failed_required_dependencies(
    db: &Connection,
    parent_task_id: &str,
    now: &str,
) -> anyhow::Result<()> {
    loop {
        let candidate = db
            .query_row(
                "SELECT node.child_task_id, node.role, node.required
                 FROM child_task_graph_nodes node
                 WHERE node.parent_task_id = ?1
                   AND node.readiness IN ('blocked_dependency', 'blocked_capacity', 'ready')
                   AND EXISTS (
                       SELECT 1 FROM child_task_graph_edges edge
                       JOIN child_task_graph_nodes predecessor
                         ON predecessor.child_task_id = edge.predecessor_task_id
                       WHERE edge.parent_task_id = node.parent_task_id
                         AND edge.successor_task_id = node.child_task_id
                         AND edge.required = 1
                         AND predecessor.readiness IN ('failed', 'timeout', 'canceled')
                   )
                 ORDER BY node.created_at, node.child_task_id
                 LIMIT 1",
                params![parent_task_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, bool>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((child_task_id, role, required)) = candidate else {
            break;
        };
        db.execute(
            "UPDATE child_task_graph_nodes
             SET readiness = 'canceled', updated_at = ?3
             WHERE parent_task_id = ?1 AND child_task_id = ?2",
            params![parent_task_id, child_task_id, now],
        )?;
        let result = json!({
            "schema_version": CHILD_TASK_GRAPH_SCHEMA_VERSION,
            "source": "child_task_graph",
            "status": "canceled",
            "error_code": "required_dependency_failed",
            "child_task_result": {
                "schema_version": CHILD_TASK_GRAPH_SCHEMA_VERSION,
                "parent_task_id": parent_task_id,
                "child_task_id": child_task_id,
                "role": role,
                "required": required,
                "status": "cancelled",
                "error_code": "required_dependency_failed",
                "evidence_refs": [],
                "artifact_refs": [],
                "finding_refs": [],
            }
        });
        db.execute(
            "UPDATE tasks
             SET status = 'canceled', result_json = ?2, error_text = NULL,
                 lease_owner = NULL, lease_expires_at = 0, updated_at = ?3
             WHERE task_id = ?1 AND status = 'queued'",
            params![child_task_id, result.to_string(), now],
        )?;
    }
    Ok(())
}

fn dependencies_from_scope(scope: &Value) -> anyhow::Result<Vec<ChildTaskDependency>> {
    let Some(items) = scope
        .get("dependencies")
        .or_else(|| scope.get("depends_on"))
        .and_then(Value::as_array)
    else {
        return Ok(Vec::new());
    };
    let mut output = Vec::new();
    for item in items {
        let (child_task_id, required) = match item {
            Value::String(value) => (value.as_str(), true),
            Value::Object(object) => (
                object
                    .get("child_task_id")
                    .or_else(|| object.get("task_id"))
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("child_graph_dependency_invalid"))?,
                object
                    .get("required")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            ),
            _ => anyhow::bail!("child_graph_dependency_invalid"),
        };
        let child_task_id = machine_ref(child_task_id)
            .ok_or_else(|| anyhow::anyhow!("child_graph_dependency_invalid"))?;
        if !output
            .iter()
            .any(|dependency: &ChildTaskDependency| dependency.child_task_id == child_task_id)
        {
            output.push(ChildTaskDependency {
                child_task_id,
                required,
                edge_kind: "declared_dependency".to_string(),
            });
        }
    }
    Ok(output)
}

fn owned_paths_from_scope(spec: &ChildTaskSpec) -> anyhow::Result<Vec<String>> {
    if !is_workspace_writer(spec.permission_profile) {
        return Ok(Vec::new());
    }
    let mut paths = spec
        .scope
        .get("owned_paths")
        .or_else(|| spec.scope.get("path_ownership"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.as_str()
                        .ok_or_else(|| anyhow::anyhow!("child_graph_owned_path_invalid"))
                        .and_then(normalize_owned_path)
                })
                .collect::<anyhow::Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![".".to_string()]);
    if paths.is_empty() {
        paths.push(".".to_string());
    }
    let mut unique = BTreeSet::new();
    for path in paths {
        unique.insert(path);
    }
    Ok(unique.into_iter().collect())
}

fn normalize_owned_path(value: &str) -> anyhow::Result<String> {
    let trimmed = value.trim().replace('\\', "/");
    if trimmed.is_empty() {
        anyhow::bail!("child_graph_owned_path_invalid");
    }
    let path = Path::new(&trimmed);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        anyhow::bail!("child_graph_owned_path_outside_workspace");
    }
    let normalized = trimmed.trim_start_matches("./").trim_end_matches('/');
    Ok(if normalized.is_empty() {
        ".".to_string()
    } else {
        normalized.to_string()
    })
}

fn add_writer_serialization_edges(
    specs: &[ChildTaskSpec],
    ownership: &BTreeMap<String, Vec<String>>,
    dependencies: &mut BTreeMap<String, Vec<ChildTaskDependency>>,
) {
    for (left_index, left) in specs.iter().enumerate() {
        if !is_workspace_writer(left.permission_profile) {
            continue;
        }
        for right in specs.iter().skip(left_index + 1) {
            if !is_workspace_writer(right.permission_profile) {
                continue;
            }
            let overlaps = ownership
                .get(&left.child_task_id)
                .into_iter()
                .flatten()
                .any(|left_path| {
                    ownership
                        .get(&right.child_task_id)
                        .into_iter()
                        .flatten()
                        .any(|right_path| paths_overlap(left_path, right_path))
                });
            if !overlaps
                || has_dependency_path(dependencies, &left.child_task_id, &right.child_task_id)
                || has_dependency_path(dependencies, &right.child_task_id, &left.child_task_id)
            {
                continue;
            }
            dependencies
                .entry(right.child_task_id.clone())
                .or_default()
                .push(ChildTaskDependency {
                    child_task_id: left.child_task_id.clone(),
                    required: true,
                    edge_kind: "path_ownership_serialization".to_string(),
                });
        }
    }
}

fn is_workspace_writer(permission_profile: ChildTaskPermissionProfile) -> bool {
    matches!(
        permission_profile,
        ChildTaskPermissionProfile::LocalCurrentWorkspace
            | ChildTaskPermissionProfile::LocalWorktree
    )
}

fn paths_overlap(left: &str, right: &str) -> bool {
    left == "."
        || right == "."
        || left == right
        || left
            .strip_prefix(right)
            .is_some_and(|suffix| suffix.starts_with('/'))
        || right
            .strip_prefix(left)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn has_dependency_path(
    dependencies: &BTreeMap<String, Vec<ChildTaskDependency>>,
    predecessor: &str,
    successor: &str,
) -> bool {
    let mut queue = VecDeque::from([successor.to_string()]);
    let mut seen = BTreeSet::new();
    while let Some(node) = queue.pop_front() {
        if !seen.insert(node.clone()) {
            continue;
        }
        for dependency in dependencies.get(&node).into_iter().flatten() {
            if dependency.child_task_id == predecessor {
                return true;
            }
            queue.push_back(dependency.child_task_id.clone());
        }
    }
    false
}

fn ensure_acyclic(
    known: &BTreeSet<String>,
    dependencies: &BTreeMap<String, Vec<ChildTaskDependency>>,
) -> anyhow::Result<()> {
    let mut incoming = known
        .iter()
        .map(|node| (node.clone(), dependencies.get(node).map_or(0, Vec::len)))
        .collect::<BTreeMap<_, _>>();
    let mut queue = incoming
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(node, _)| node.clone())
        .collect::<VecDeque<_>>();
    let mut visited = 0;
    while let Some(predecessor) = queue.pop_front() {
        visited += 1;
        for (successor, predecessors) in dependencies {
            if predecessors
                .iter()
                .any(|dependency| dependency.child_task_id == predecessor)
            {
                let count = incoming
                    .get_mut(successor)
                    .expect("known dependency successor");
                *count -= 1;
                if *count == 0 {
                    queue.push_back(successor.clone());
                }
            }
        }
    }
    if visited != known.len() {
        anyhow::bail!("child_graph_cycle");
    }
    Ok(())
}

fn machine_policy(scope: &Value, key: &str) -> anyhow::Result<Value> {
    let value = scope.get(key).cloned().unwrap_or_else(|| json!({}));
    if !value.is_object() {
        anyhow::bail!("child_graph_policy_invalid");
    }
    Ok(value)
}

fn machine_ref(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 160
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | ':' | '.' | '/'))
    {
        return None;
    }
    Some(value.to_string())
}

fn parse_json_column(raw: String, fallback: Value) -> Value {
    serde_json::from_str(&raw).unwrap_or(fallback)
}

fn child_runtime_projection(raw_result: Option<&str>) -> Value {
    let Some(result) = raw_result.and_then(|raw| serde_json::from_str::<Value>(raw).ok()) else {
        return json!({});
    };
    json!({
        "child_task_result": result.get("child_task_result"),
        "execution_scope": result.get("child_task_execution_scope"),
        "task_lifecycle": result.get("task_lifecycle"),
        "evidence": result
            .get("child_task_result")
            .and_then(|value| value.get("evidence_refs")),
        "artifacts": result
            .get("child_task_result")
            .and_then(|value| value.get("artifact_refs")),
        "findings": result
            .get("child_task_result")
            .and_then(|value| value.get("finding_refs")),
        "patch": result
            .get("child_patch")
            .or_else(|| result.get("patch"))
            .or_else(|| result.pointer("/child_task_execution_scope/patch")),
        "usage": result
            .get("provider_usage")
            .or_else(|| result.get("usage"))
            .or_else(|| result.pointer("/task_lifecycle/provider_usage")),
        "cost": result
            .get("cost")
            .or_else(|| result.pointer("/task_lifecycle/cost")),
    })
}

#[cfg(test)]
#[path = "child_task_graph_tests.rs"]
mod tests;
