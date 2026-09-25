use anyhow::{anyhow, Context};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use crate::db_init::DbPool;
use crate::{AppState, ClaimedTask};

const MIGRATION_SQL: &str = include_str!("../../../../migrations/025_conversation_reply_items.sql");
const MACHINE_PRESENTATION_MIGRATION_SQL: &str =
    include_str!("../../../../migrations/027_conversation_reply_machine_presentations.sql");

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConversationReplyItem {
    pub(crate) reply_id: String,
    pub(crate) task_id: String,
    pub(crate) input_id: Option<String>,
    pub(crate) owner_principal_id: String,
    pub(crate) instruction_revision: u64,
    pub(crate) execution_epoch: u64,
    pub(crate) relation: String,
    pub(crate) lifecycle_stage: String,
    pub(crate) text: String,
    pub(crate) message_key: Option<String>,
    pub(crate) params: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PersistConversationReplyOutcome {
    pub(crate) item: ConversationReplyItem,
    pub(crate) inserted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClaimedConversationReplyDelivery {
    pub(crate) reply_id: String,
    pub(crate) lease_token: String,
    pub(crate) attempt_count: u32,
}

pub(crate) fn ensure_conversation_reply_item_schema(
    db: &rusqlite::Connection,
) -> anyhow::Result<()> {
    db.execute_batch(MIGRATION_SQL)?;
    db.execute_batch(MACHINE_PRESENTATION_MIGRATION_SQL)?;
    Ok(())
}

pub(crate) fn persist_nonterminal_reply_item(
    state: &AppState,
    task: &ClaimedTask,
    relation: &str,
    lifecycle_stage: &str,
    text: &str,
    instruction_revision: u64,
    execution_epoch: u64,
) -> anyhow::Result<PersistConversationReplyOutcome> {
    persist_nonterminal_reply_item_internal(
        state,
        task,
        relation,
        lifecycle_stage,
        text,
        instruction_revision,
        execution_epoch,
        None,
    )
}

pub(crate) fn persist_clarification_reply_with_checkpoint(
    state: &AppState,
    task: &ClaimedTask,
    text: &str,
    instruction_revision: u64,
    execution_epoch: u64,
    checkpoint_progress: &serde_json::Value,
) -> anyhow::Result<PersistConversationReplyOutcome> {
    persist_nonterminal_reply_item_internal(
        state,
        task,
        "clarification",
        "accepted",
        text,
        instruction_revision,
        execution_epoch,
        Some(checkpoint_progress),
    )
}

pub(crate) fn nonterminal_reply_id(
    task_id: &str,
    relation: &str,
    lifecycle_stage: &str,
    text: &str,
    instruction_revision: u64,
    execution_epoch: u64,
) -> String {
    format!(
        "reply_{}",
        reply_digest(
            task_id,
            relation,
            lifecycle_stage,
            text.trim(),
            instruction_revision,
            execution_epoch,
        )
    )
}

#[allow(clippy::too_many_arguments)]
fn persist_nonterminal_reply_item_internal(
    state: &AppState,
    task: &ClaimedTask,
    relation: &str,
    lifecycle_stage: &str,
    text: &str,
    instruction_revision: u64,
    execution_epoch: u64,
    checkpoint_progress: Option<&serde_json::Value>,
) -> anyhow::Result<PersistConversationReplyOutcome> {
    if !matches!(relation, "side_reply" | "clarification" | "control_status") {
        return Err(anyhow!("conversation_reply_relation_invalid"));
    }
    if !matches!(lifecycle_stage, "accepted" | "stop_requested" | "settled") {
        return Err(anyhow!("conversation_reply_lifecycle_stage_invalid"));
    }
    let text = text.trim();
    if text.is_empty() || text.len() > 64 * 1024 {
        return Err(anyhow!("conversation_reply_text_invalid"));
    }

    let digest = reply_digest(
        &task.task_id,
        relation,
        lifecycle_stage,
        text,
        instruction_revision,
        execution_epoch,
    );
    let reply_id = format!("reply_{digest}");
    let mut db = state
        .core
        .db
        .get()
        .context("conversation_reply_db_pool_failed")?;
    ensure_conversation_reply_item_schema(&db)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let task_context = tx
        .query_row(
            "SELECT principal_id, channel, payload_json, result_json
             FROM tasks
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?2
               AND claim_attempt = ?3
             LIMIT 1",
            params![
                task.task_id,
                state.worker.worker_id.as_str(),
                task.claim_attempt
            ],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((owner_principal_id, channel, payload_json, current_result_json)) = task_context
    else {
        return Err(crate::repo::worker_task_write_rejection(
            &tx,
            state,
            &task.task_id,
            task.claim_attempt,
            "persist_nonterminal_reply_item",
            &["running"],
        ));
    };
    let owner_principal_id = owner_principal_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("conversation_reply_owner_missing"))?;
    let input_id = tx
        .query_row(
            "SELECT input_id
             FROM conversation_inputs
             WHERE target_task_id = ?1
               AND instruction_revision <= ?2
             ORDER BY instruction_revision DESC, input_seq DESC
             LIMIT 1",
            params![task.task_id, instruction_revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let now_ts = crate::now_ts_u64();
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO conversation_reply_items (
             reply_id, task_id, input_id, owner_principal_id,
             instruction_revision, execution_epoch, relation, lifecycle_stage,
             text, content_digest, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            reply_id,
            task.task_id,
            input_id,
            owner_principal_id,
            instruction_revision,
            execution_epoch,
            relation,
            lifecycle_stage,
            text,
            digest,
            now_ts,
        ],
    )? == 1;
    if inserted && channel != "ui" && task_payload_has_delivery_ingress(&payload_json) {
        tx.execute(
            "INSERT OR IGNORE INTO conversation_reply_delivery_outbox (
                 reply_id, state, next_attempt_at_ts, created_at_ts, updated_at_ts
             ) VALUES (?1, 'pending', 0, ?2, ?2)",
            params![reply_id, now_ts],
        )?;
    }
    if let Some(checkpoint_progress) = checkpoint_progress {
        let now_ts_i64 = i64::try_from(now_ts).unwrap_or(i64::MAX);
        let merged_result_json = current_result_json
            .as_deref()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
            .and_then(|current| {
                crate::repo::task_resume_execution::merge_progress_with_active_resume_coordination(
                    &current,
                    checkpoint_progress,
                    now_ts_i64,
                )
            })
            .unwrap_or_else(|| checkpoint_progress.clone())
            .to_string();
        let changed = tx.execute(
            "UPDATE tasks
             SET result_json = ?2, updated_at = ?3
             WHERE task_id = ?1
               AND status = 'running'
               AND lease_owner = ?4
               AND claim_attempt = ?5
               AND result_json IS ?6",
            params![
                task.task_id,
                merged_result_json,
                now_ts_i64,
                state.worker.worker_id.as_str(),
                task.claim_attempt,
                current_result_json,
            ],
        )?;
        if changed != 1 {
            return Err(crate::repo::worker_task_write_rejection(
                &tx,
                state,
                &task.task_id,
                task.claim_attempt,
                "persist_clarification_reply_with_checkpoint",
                &["running"],
            ));
        }
    }
    tx.commit()?;
    drop(db);

    let item = ConversationReplyItem {
        reply_id,
        task_id: task.task_id.clone(),
        input_id,
        owner_principal_id,
        instruction_revision,
        execution_epoch,
        relation: relation.to_string(),
        lifecycle_stage: lifecycle_stage.to_string(),
        text: text.to_string(),
        message_key: None,
        params: BTreeMap::new(),
    };
    publish_conversation_reply_item_event(state, &item);
    Ok(PersistConversationReplyOutcome { item, inserted })
}

pub(crate) fn persist_control_status_reply_item_in_db(
    db: &rusqlite::Connection,
    task_id: &str,
    lifecycle_stage: &str,
    message_key: &str,
    params: &BTreeMap<String, String>,
    channel_delivery_required: bool,
) -> anyhow::Result<PersistConversationReplyOutcome> {
    if !matches!(lifecycle_stage, "accepted" | "stop_requested" | "settled") {
        return Err(anyhow!("conversation_reply_lifecycle_stage_invalid"));
    }
    let notice = claw_core::channel_notice::ChannelNotice::status(
        format!("conversation.control.{lifecycle_stage}"),
        message_key,
        claw_core::channel_notice::ChannelNoticeSeverity::Info,
    );
    let mut notice = notice;
    notice.params = params.clone();
    notice
        .validate()
        .map_err(|_| anyhow!("conversation_reply_machine_presentation_invalid"))?;
    ensure_conversation_reply_item_schema(db)?;
    crate::repo::conversation_inputs::ensure_conversation_input_schema(db)?;
    let task_context = db
        .query_row(
            "SELECT principal_id, channel, payload_json
             FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .optional()?;
    let Some((owner_principal_id, channel, payload_json)) = task_context else {
        return Err(anyhow!("conversation_reply_task_not_found"));
    };
    let owner_principal_id = owner_principal_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow!("conversation_reply_owner_missing"))?;
    let (instruction_revision, execution_epoch) = db
        .query_row(
            "SELECT instruction_revision, execution_epoch
             FROM conversation_input_scopes WHERE focus_task_id = ?1
             UNION ALL
             SELECT instruction_revision, execution_epoch
             FROM conversation_terminal_boundaries WHERE task_id = ?1
             LIMIT 1",
            params![task_id],
            |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
        )
        .optional()?
        .unwrap_or((0, 0));
    let input_id = db
        .query_row(
            "SELECT input_id FROM conversation_inputs
             WHERE target_task_id = ?1 AND instruction_revision <= ?2
             ORDER BY instruction_revision DESC, input_seq DESC LIMIT 1",
            params![task_id, instruction_revision],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let params_json = serde_json::to_string(params)?;
    let digest = machine_reply_digest(
        task_id,
        lifecycle_stage,
        message_key,
        &params_json,
        instruction_revision,
        execution_epoch,
    );
    let reply_id = format!("reply_{digest}");
    let now_ts = crate::now_ts_u64();
    let inserted = db.execute(
        "INSERT OR IGNORE INTO conversation_reply_items (
             reply_id, task_id, input_id, owner_principal_id,
             instruction_revision, execution_epoch, relation, lifecycle_stage,
             text, content_digest, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'control_status', ?7, '', ?8, ?9)",
        params![
            reply_id,
            task_id,
            input_id,
            owner_principal_id,
            instruction_revision,
            execution_epoch,
            lifecycle_stage,
            digest,
            now_ts,
        ],
    )? == 1;
    if inserted {
        db.execute(
            "INSERT INTO conversation_reply_item_presentations(reply_id, message_key, params_json)
             VALUES (?1, ?2, ?3)",
            params![reply_id, message_key, params_json],
        )?;
        if channel_delivery_required
            && channel != "ui"
            && task_payload_has_delivery_ingress(&payload_json)
        {
            db.execute(
                "INSERT OR IGNORE INTO conversation_reply_delivery_outbox (
                     reply_id, state, next_attempt_at_ts, created_at_ts, updated_at_ts
                 ) VALUES (?1, 'pending', 0, ?2, ?2)",
                params![reply_id, now_ts],
            )?;
        }
    }
    Ok(PersistConversationReplyOutcome {
        item: ConversationReplyItem {
            reply_id,
            task_id: task_id.to_string(),
            input_id,
            owner_principal_id,
            instruction_revision,
            execution_epoch,
            relation: "control_status".to_string(),
            lifecycle_stage: lifecycle_stage.to_string(),
            text: String::new(),
            message_key: Some(message_key.to_string()),
            params: params.clone(),
        },
        inserted,
    })
}

pub(crate) fn persist_control_status_reply_item_if_owned_in_db(
    db: &rusqlite::Connection,
    task_id: &str,
    lifecycle_stage: &str,
    message_key: &str,
    params: &BTreeMap<String, String>,
    channel_delivery_required: bool,
) -> anyhow::Result<Option<PersistConversationReplyOutcome>> {
    let has_principal_id = {
        let mut statement = db.prepare("PRAGMA table_info(tasks)")?;
        let columns = statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        columns.iter().any(|column| column == "principal_id")
    };
    if !has_principal_id {
        return Ok(None);
    }
    let owner_principal_id = db
        .query_row(
            "SELECT principal_id FROM tasks WHERE task_id = ?1 LIMIT 1",
            params![task_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten();
    if owner_principal_id
        .as_deref()
        .map(str::trim)
        .is_none_or(str::is_empty)
    {
        return Ok(None);
    }
    persist_control_status_reply_item_in_db(
        db,
        task_id,
        lifecycle_stage,
        message_key,
        params,
        channel_delivery_required,
    )
    .map(Some)
}

pub(crate) fn publish_conversation_reply_item_event(
    state: &AppState,
    item: &ConversationReplyItem,
) {
    let _ = crate::task_event_transport::publish_event(
        state,
        &item.task_id,
        "conversation_reply_item",
        json!({
            "schema_version": 1,
            "reply_id": item.reply_id,
            "input_id": item.input_id,
            "instruction_revision": item.instruction_revision,
            "execution_epoch": item.execution_epoch,
            "relation": item.relation,
            "lifecycle_stage": item.lifecycle_stage,
            "text": item.text,
            "message_key": item.message_key,
            "params": item.params,
            "terminal": false,
        }),
    );
}

pub(crate) fn claim_due_conversation_reply_delivery(
    pool: &DbPool,
    now_ts: u64,
    lease_seconds: u64,
) -> anyhow::Result<Option<ClaimedConversationReplyDelivery>> {
    let mut db = pool
        .get()
        .context("conversation_reply_delivery_db_pool_failed")?;
    ensure_conversation_reply_item_schema(&db)?;
    let now_ts = i64::try_from(now_ts).map_err(|_| anyhow!("reply_timestamp_out_of_range"))?;
    let lease_expires_at_ts = now_ts.saturating_add(
        i64::try_from(lease_seconds.max(1)).map_err(|_| anyhow!("reply_lease_out_of_range"))?,
    );
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let reply_id = tx
        .query_row(
            "SELECT reply_id
             FROM conversation_reply_delivery_outbox
             WHERE (state = 'pending' AND next_attempt_at_ts <= ?1)
                OR (state = 'dispatching' AND lease_expires_at_ts <= ?1)
             ORDER BY next_attempt_at_ts ASC, created_at_ts ASC
             LIMIT 1",
            params![now_ts],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    let Some(reply_id) = reply_id else {
        tx.commit()?;
        return Ok(None);
    };
    let lease_token = uuid::Uuid::new_v4().to_string();
    let changed = tx.execute(
        "UPDATE conversation_reply_delivery_outbox
         SET state = 'dispatching', lease_token = ?2, lease_expires_at_ts = ?3,
             attempt_count = attempt_count + 1, updated_at_ts = ?1
         WHERE reply_id = ?4
           AND ((state = 'pending' AND next_attempt_at_ts <= ?1)
             OR (state = 'dispatching' AND lease_expires_at_ts <= ?1))",
        params![now_ts, lease_token, lease_expires_at_ts, reply_id],
    )?;
    if changed != 1 {
        tx.commit()?;
        return Ok(None);
    }
    let attempt_count = tx.query_row(
        "SELECT attempt_count
         FROM conversation_reply_delivery_outbox
         WHERE reply_id = ?1",
        params![reply_id],
        |row| row.get::<_, u32>(0),
    )?;
    tx.commit()?;
    Ok(Some(ClaimedConversationReplyDelivery {
        reply_id,
        lease_token,
        attempt_count,
    }))
}

pub(crate) fn finish_conversation_reply_delivery(
    pool: &DbPool,
    claim: &ClaimedConversationReplyDelivery,
    completed: bool,
    retry_after_seconds: Option<u64>,
    error_code: Option<&str>,
    now_ts: u64,
) -> anyhow::Result<()> {
    let db = pool
        .get()
        .context("conversation_reply_delivery_db_pool_failed")?;
    let now_ts = i64::try_from(now_ts).map_err(|_| anyhow!("reply_timestamp_out_of_range"))?;
    let (state, next_attempt_at_ts) = if completed {
        ("completed", 0)
    } else if let Some(delay) = retry_after_seconds {
        (
            "pending",
            now_ts.saturating_add(i64::try_from(delay).unwrap_or(i64::MAX)),
        )
    } else {
        ("failed", 0)
    };
    let changed = db.execute(
        "UPDATE conversation_reply_delivery_outbox
         SET state = ?3, lease_token = NULL, lease_expires_at_ts = 0,
             next_attempt_at_ts = ?4, last_error_code = ?5, updated_at_ts = ?6
         WHERE reply_id = ?1 AND state = 'dispatching' AND lease_token = ?2",
        params![
            claim.reply_id,
            claim.lease_token,
            state,
            next_attempt_at_ts,
            error_code,
            now_ts,
        ],
    )?;
    if changed != 1 {
        return Err(anyhow!("conversation_reply_delivery_lease_mismatch"));
    }
    Ok(())
}

pub(crate) fn get_conversation_reply_item(
    pool: &DbPool,
    reply_id: &str,
) -> anyhow::Result<Option<ConversationReplyItem>> {
    let db = pool
        .get()
        .context("conversation_reply_delivery_db_pool_failed")?;
    ensure_conversation_reply_item_schema(&db)?;
    db.query_row(
        "SELECT item.reply_id, item.task_id, item.input_id, item.owner_principal_id,
                item.instruction_revision, item.execution_epoch, item.relation,
                item.lifecycle_stage, item.text, presentation.message_key,
                presentation.params_json
         FROM conversation_reply_items item
         LEFT JOIN conversation_reply_item_presentations presentation
           ON presentation.reply_id = item.reply_id
         WHERE item.reply_id = ?1
         LIMIT 1",
        params![reply_id],
        |row| {
            Ok(ConversationReplyItem {
                reply_id: row.get(0)?,
                task_id: row.get(1)?,
                input_id: row.get(2)?,
                owner_principal_id: row.get(3)?,
                instruction_revision: row.get(4)?,
                execution_epoch: row.get(5)?,
                relation: row.get(6)?,
                lifecycle_stage: row.get(7)?,
                text: row.get(8)?,
                message_key: row.get(9)?,
                params: row
                    .get::<_, Option<String>>(10)?
                    .and_then(|raw| serde_json::from_str(&raw).ok())
                    .unwrap_or_default(),
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub(crate) fn has_unsettled_conversation_reply_delivery(
    pool: &DbPool,
    task_id: &str,
) -> anyhow::Result<bool> {
    let db = pool
        .get()
        .context("conversation_reply_delivery_db_pool_failed")?;
    ensure_conversation_reply_item_schema(&db)?;
    Ok(db
        .query_row(
            "SELECT 1
             FROM conversation_reply_delivery_outbox outbox
             JOIN conversation_reply_items item ON item.reply_id = outbox.reply_id
             WHERE item.task_id = ?1
               AND outbox.state IN ('pending', 'dispatching')
             LIMIT 1",
            params![task_id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn task_payload_has_delivery_ingress(payload_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(payload_json)
        .ok()
        .and_then(|value| value.get("channel_ingress").cloned())
        .is_some_and(|value| value.is_object())
}

fn reply_digest(
    task_id: &str,
    relation: &str,
    lifecycle_stage: &str,
    text: &str,
    instruction_revision: u64,
    execution_epoch: u64,
) -> String {
    let value = json!({
        "task_id": task_id,
        "relation": relation,
        "lifecycle_stage": lifecycle_stage,
        "text": text,
        "instruction_revision": instruction_revision,
        "execution_epoch": execution_epoch,
    });
    let bytes = serde_json::to_vec(&value).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

fn machine_reply_digest(
    task_id: &str,
    lifecycle_stage: &str,
    message_key: &str,
    params_json: &str,
    instruction_revision: u64,
    execution_epoch: u64,
) -> String {
    let value = json!({
        "task_id": task_id,
        "relation": "control_status",
        "lifecycle_stage": lifecycle_stage,
        "message_key": message_key,
        "params_json": params_json,
        "instruction_revision": instruction_revision,
        "execution_epoch": execution_epoch,
    });
    let bytes = serde_json::to_vec(&value).unwrap_or_default();
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "conversation_reply_items_tests.rs"]
mod tests;
