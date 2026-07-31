use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::AppState;

const TASK_PLAN_SCHEMA_VERSION: u64 = 1;
pub(crate) const TASK_PLAN_SOURCE: &str = "task_plan";
const MAX_PLAN_STEPS: usize = 64;
const MAX_STEP_ID_CHARS: usize = 80;
const MAX_STEP_TITLE_CHARS: usize = 512;

const INIT_TASK_PLAN_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS task_plans (
    task_id        TEXT PRIMARY KEY,
    plan_revision  INTEGER NOT NULL,
    plan_json      TEXT NOT NULL,
    updated_at_ms  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_task_plans_updated
    ON task_plans(updated_at_ms);
"#;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskPlanStep {
    pub(crate) step_id: String,
    pub(crate) title: String,
    pub(crate) status: TaskPlanStepStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TaskPlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct TaskPlanStepUpdate {
    pub(crate) step_id: String,
    #[serde(default)]
    pub(crate) title: Option<String>,
    #[serde(default)]
    pub(crate) status: Option<TaskPlanStepStatus>,
}

#[derive(Debug, Clone)]
pub(crate) struct TaskPlanError {
    pub(crate) error_code: &'static str,
    pub(crate) retryable: bool,
    pub(crate) expected_revision: Option<u64>,
    pub(crate) current_revision: Option<u64>,
    pub(crate) detail: Option<String>,
}

impl TaskPlanError {
    fn new(error_code: &'static str) -> Self {
        Self {
            error_code,
            retryable: false,
            expected_revision: None,
            current_revision: None,
            detail: None,
        }
    }

    fn invalid(detail: impl Into<String>) -> Self {
        let mut error = Self::new("task_plan_invalid");
        error.detail = Some(detail.into());
        error
    }

    fn conflict(expected_revision: u64, current_revision: u64) -> Self {
        Self {
            error_code: "task_plan_revision_conflict",
            retryable: true,
            expected_revision: Some(expected_revision),
            current_revision: Some(current_revision),
            detail: None,
        }
    }

    pub(crate) fn machine_extra(&self) -> Value {
        json!({
            "schema_version": TASK_PLAN_SCHEMA_VERSION,
            "source": TASK_PLAN_SOURCE,
            "status": "error",
            "error_code": self.error_code,
            "message_key": format!("clawd.task_plan.{}", self.error_code),
            "retryable": self.retryable,
            "expected_plan_revision": self.expected_revision,
            "current_plan_revision": self.current_revision,
            "detail": self.detail,
        })
    }
}

pub(crate) fn ensure_task_plan_schema(db: &rusqlite::Connection) -> anyhow::Result<()> {
    db.execute_batch(INIT_TASK_PLAN_SQL)?;
    Ok(())
}

pub(crate) fn read_task_plan(
    state: &AppState,
    task_id: &str,
    action: &str,
) -> Result<Value, TaskPlanError> {
    let db = state
        .core
        .db
        .get()
        .map_err(|error| storage_error("task_plan_db_pool_failed", error))?;
    ensure_task_plan_schema(&db)
        .map_err(|error| storage_error("task_plan_schema_failed", error))?;
    let stored = db
        .query_row(
            "SELECT plan_revision, plan_json, updated_at_ms
             FROM task_plans
             WHERE task_id = ?1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, u64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u64>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| storage_error("task_plan_read_failed", error))?;
    match stored {
        Some((revision, plan_json, updated_at_ms)) => {
            let steps = serde_json::from_str::<Vec<TaskPlanStep>>(&plan_json)
                .map_err(|error| storage_error("task_plan_decode_failed", error))?;
            Ok(plan_response(
                task_id,
                action,
                revision,
                updated_at_ms,
                Some(&steps),
            ))
        }
        None => Ok(plan_response(task_id, action, 0, 0, None)),
    }
}

pub(crate) fn read_task_plan_query_projection(
    state: &AppState,
    task_id: &str,
) -> Result<Option<Value>, TaskPlanError> {
    let snapshot = read_task_plan(state, task_id, "read_plan")?;
    if snapshot
        .get("plan_revision")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        == 0
    {
        return Ok(None);
    }
    Ok(Some(json!({
        "schema_version": TASK_PLAN_SCHEMA_VERSION,
        "source": TASK_PLAN_SOURCE,
        "status": "ok",
        "data_only": true,
        "render_owner": "ui_cli_channel_projection",
        "task_id": task_id,
        "plan_revision": snapshot.get("plan_revision"),
        "updated_at_ms": snapshot.get("updated_at_ms"),
        "steps": snapshot.get("steps"),
        "checkpoint": snapshot.get("checkpoint"),
    })))
}

pub(crate) fn set_task_plan(
    state: &AppState,
    task_id: &str,
    expected_revision: u64,
    steps: Vec<TaskPlanStep>,
) -> Result<Value, TaskPlanError> {
    validate_steps(&steps)?;
    write_task_plan(state, task_id, "set_plan", expected_revision, move |_| {
        Ok(steps)
    })
}

pub(crate) fn update_task_plan_steps(
    state: &AppState,
    task_id: &str,
    expected_revision: u64,
    updates: Vec<TaskPlanStepUpdate>,
) -> Result<Value, TaskPlanError> {
    if updates.is_empty() {
        return Err(TaskPlanError::invalid("updates_must_not_be_empty"));
    }
    let mut update_ids = HashSet::new();
    for update in &updates {
        validate_step_id(&update.step_id)?;
        if update.title.is_none() && update.status.is_none() {
            return Err(TaskPlanError::invalid(format!(
                "update_has_no_changes:{}",
                update.step_id
            )));
        }
        if let Some(title) = update.title.as_deref() {
            validate_title(title)?;
        }
        if !update_ids.insert(update.step_id.as_str()) {
            return Err(TaskPlanError::invalid(format!(
                "duplicate_update_step_id:{}",
                update.step_id
            )));
        }
    }
    write_task_plan(
        state,
        task_id,
        "update_steps",
        expected_revision,
        move |current| {
            let Some(mut steps) = current else {
                return Err(TaskPlanError::conflict(expected_revision, 0));
            };
            let indexes = steps
                .iter()
                .enumerate()
                .map(|(index, step)| (step.step_id.clone(), index))
                .collect::<HashMap<_, _>>();
            for update in updates {
                let Some(index) = indexes.get(&update.step_id).copied() else {
                    return Err(TaskPlanError::invalid(format!(
                        "unknown_step_id:{}",
                        update.step_id
                    )));
                };
                if let Some(title) = update.title {
                    steps[index].title = title;
                }
                if let Some(status) = update.status {
                    steps[index].status = status;
                }
            }
            validate_steps(&steps)?;
            Ok(steps)
        },
    )
}

fn write_task_plan<F>(
    state: &AppState,
    task_id: &str,
    action: &str,
    expected_revision: u64,
    transform: F,
) -> Result<Value, TaskPlanError>
where
    F: FnOnce(Option<Vec<TaskPlanStep>>) -> Result<Vec<TaskPlanStep>, TaskPlanError>,
{
    let mut db = state
        .core
        .db
        .get()
        .map_err(|error| storage_error("task_plan_db_pool_failed", error))?;
    ensure_task_plan_schema(&db)
        .map_err(|error| storage_error("task_plan_schema_failed", error))?;
    let tx = db
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| storage_error("task_plan_transaction_failed", error))?;
    let stored = tx
        .query_row(
            "SELECT plan_revision, plan_json
             FROM task_plans
             WHERE task_id = ?1",
            params![task_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| storage_error("task_plan_read_failed", error))?;
    let current_revision = stored.as_ref().map(|(revision, _)| *revision).unwrap_or(0);
    if current_revision != expected_revision {
        return Err(TaskPlanError::conflict(expected_revision, current_revision));
    }
    let current_steps = stored
        .map(|(_, plan_json)| {
            serde_json::from_str::<Vec<TaskPlanStep>>(&plan_json)
                .map_err(|error| storage_error("task_plan_decode_failed", error))
        })
        .transpose()?;
    let steps = transform(current_steps)?;
    validate_steps(&steps)?;
    let new_revision = current_revision.saturating_add(1);
    let updated_at_ms = now_ms();
    let plan_json = serde_json::to_string(&steps)
        .map_err(|error| storage_error("task_plan_encode_failed", error))?;
    tx.execute(
        "INSERT INTO task_plans(task_id, plan_revision, plan_json, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(task_id) DO UPDATE SET
             plan_revision = excluded.plan_revision,
             plan_json = excluded.plan_json,
             updated_at_ms = excluded.updated_at_ms",
        params![task_id, new_revision, plan_json, updated_at_ms],
    )
    .map_err(|error| storage_error("task_plan_write_failed", error))?;
    tx.commit()
        .map_err(|error| storage_error("task_plan_commit_failed", error))?;
    Ok(plan_response(
        task_id,
        action,
        new_revision,
        updated_at_ms,
        Some(&steps),
    ))
}

fn validate_steps(steps: &[TaskPlanStep]) -> Result<(), TaskPlanError> {
    if steps.is_empty() {
        return Err(TaskPlanError::invalid("steps_must_not_be_empty"));
    }
    if steps.len() > MAX_PLAN_STEPS {
        return Err(TaskPlanError::invalid("too_many_steps"));
    }
    let mut ids = HashSet::new();
    let mut in_progress_count = 0usize;
    for step in steps {
        validate_step_id(&step.step_id)?;
        validate_title(&step.title)?;
        if !ids.insert(step.step_id.as_str()) {
            return Err(TaskPlanError::invalid(format!(
                "duplicate_step_id:{}",
                step.step_id
            )));
        }
        if step.status == TaskPlanStepStatus::InProgress {
            in_progress_count += 1;
        }
    }
    if in_progress_count > 1 {
        return Err(TaskPlanError::invalid(
            "multiple_in_progress_steps_not_allowed",
        ));
    }
    Ok(())
}

fn validate_step_id(step_id: &str) -> Result<(), TaskPlanError> {
    let trimmed = step_id.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_STEP_ID_CHARS
        || !trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | ':'))
    {
        return Err(TaskPlanError::invalid(format!(
            "invalid_step_id:{}",
            step_id
        )));
    }
    Ok(())
}

fn validate_title(title: &str) -> Result<(), TaskPlanError> {
    let trimmed = title.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_STEP_TITLE_CHARS
        || trimmed.chars().any(char::is_control)
    {
        return Err(TaskPlanError::invalid("invalid_step_title"));
    }
    Ok(())
}

fn plan_response(
    task_id: &str,
    action: &str,
    revision: u64,
    updated_at_ms: u64,
    steps: Option<&[TaskPlanStep]>,
) -> Value {
    let checkpoint = steps.map(|_| {
        json!({
            "kind": TASK_PLAN_SOURCE,
            "ref": format!("task_plan:{task_id}:{revision}"),
            "plan_revision": revision,
        })
    });
    json!({
        "schema_version": TASK_PLAN_SCHEMA_VERSION,
        "source": TASK_PLAN_SOURCE,
        "status": "ok",
        "action": action,
        "task_id": task_id,
        "plan_revision": revision,
        "updated_at_ms": updated_at_ms,
        "steps": steps,
        "checkpoint": checkpoint,
    })
}

fn storage_error(error_code: &'static str, error: impl std::fmt::Display) -> TaskPlanError {
    let mut task_plan_error = TaskPlanError::new(error_code);
    task_plan_error.detail = Some(error.to_string());
    task_plan_error
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
#[path = "task_plan_tests.rs"]
mod tests;
