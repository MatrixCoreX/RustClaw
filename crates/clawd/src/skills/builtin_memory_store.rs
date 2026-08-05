use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::{AppState, ClaimedTask};

const MIGRATION_ID: &str = "015_memory_capability_receipts_v1";
const MIGRATION_SQL: &str =
    include_str!("../../../../migrations/015_memory_capability_receipts.sql");
const SOURCE_SKILL: &str = "memory_store";

#[derive(Debug, Serialize)]
struct MemoryCapabilityItem {
    memory_id: String,
    revision: i64,
    kind: String,
    scope: String,
    excerpt: String,
    freshness: String,
    trust: String,
    source_available: bool,
}

pub(super) async fn execute(
    state: &AppState,
    task: Option<&ClaimedTask>,
    args: &Map<String, Value>,
) -> Result<String, String> {
    let Some(task) = task else {
        return Ok(outcome(
            "error",
            "unknown",
            Some("memory_task_context_required"),
            json!({}),
        ));
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = execute_action(state, task, action, args).await;
    Ok(match result {
        Ok(value) => outcome("ok", action, None, value),
        Err(code) => outcome("error", action, Some(code.as_str()), json!({})),
    })
}

async fn execute_action(
    state: &AppState,
    task: &ClaimedTask,
    action: &str,
    args: &Map<String, Value>,
) -> Result<Value, String> {
    let settings = crate::memory::settings::resolve_task_memory_settings(state, task)
        .map_err(|_| "memory_settings_unavailable".to_string())?
        .ok_or_else(|| "memory_settings_unavailable".to_string())?;
    let mut db = state
        .core
        .db
        .get()
        .map_err(|_| "memory_database_unavailable".to_string())?;
    ensure_schema(&db).map_err(|_| "memory_schema_unavailable".to_string())?;
    let scope = resolve_scope(&db, state, task, &settings.target_principal_id, args)?;

    match action {
        "search" => {
            if !settings.use_memory {
                return Err("memory_use_disabled".to_string());
            }
            let query = required_text(args, "query", 1, 4_000)?;
            let limit = optional_limit(args, 8, 20)?;
            let cursor = optional_text(args, "cursor", 512)?;
            let (items, continuation_token) = search_items(
                &db,
                &settings.target_principal_id,
                &scope.0,
                &scope.1,
                query,
                cursor,
                limit,
                crate::now_ts_u64() as i64,
            )
            .map_err(|error| stable_error_code(error, "memory_search_failed"))?;
            Ok(json!({
                "items": items,
                "count": items.len(),
                "continuation_token": continuation_token,
                "data_only": true,
                "instruction_authority": "none",
            }))
        }
        "list_recent" => {
            if !settings.use_memory {
                return Err("memory_use_disabled".to_string());
            }
            let limit = optional_limit(args, 10, 50)?;
            let cursor = optional_text(args, "cursor", 512)?;
            let (items, continuation_token) = search_items(
                &db,
                &settings.target_principal_id,
                &scope.0,
                &scope.1,
                "",
                cursor,
                limit,
                crate::now_ts_u64() as i64,
            )
            .map_err(|error| stable_error_code(error, "memory_list_failed"))?;
            Ok(json!({
                "items": items,
                "count": items.len(),
                "continuation_token": continuation_token,
                "data_only": true,
                "instruction_authority": "none",
            }))
        }
        "save" => {
            if !settings.generate_memory {
                return Err("memory_generate_disabled".to_string());
            }
            if is_child_task(&db, &task.task_id).unwrap_or(false) {
                return Err("memory_child_write_denied".to_string());
            }
            let content = required_text(args, "content", 1, 16_000)?;
            let (safe_content, redacted) =
                crate::skill_output_artifact::sensitivity_aware_text_model_view(content);
            if redacted || safe_content.trim() != content.trim() {
                return Err("memory_sensitive_content_blocked".to_string());
            }
            let kind = required_enum(args, "kind", &["fact", "preference", "session_note"])?;
            validate_kind_scope(kind, &scope.0)?;
            let idempotency_key = required_text(args, "idempotency_key", 8, 160)?;
            let item_key = args
                .get("key")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if kind == "preference" && item_key.is_none() {
                return Err("memory_preference_key_required".to_string());
            }
            let mut evidence_refs = evidence_refs(task, args)?;
            if scope.0 == "project" {
                evidence_refs.push(format!("current_project:{}", digest_text(&scope.1)));
                evidence_refs.sort();
                evidence_refs.dedup();
            }
            crate::memory::retention::ensure_principal_quota(
                &db,
                &state.policy.memory,
                &settings.target_principal_id,
                crate::now_ts_u64() as i64,
            )
            .map_err(|_| "memory_quota_exceeded".to_string())?;
            save_item(
                &mut db,
                task,
                &settings.target_principal_id,
                &scope.0,
                &scope.1,
                kind,
                item_key,
                content.trim(),
                idempotency_key,
                &evidence_refs,
            )
            .map_err(|error| stable_error_code(error, "memory_save_failed"))
        }
        "forget" => {
            if is_child_task(&db, &task.task_id).unwrap_or(false) {
                return Err("memory_child_write_denied".to_string());
            }
            let memory_id = required_opaque_memory_id(args)?;
            let revision = required_revision(args)?;
            ensure_item_scope(
                &db,
                &settings.target_principal_id,
                memory_id,
                &scope.0,
                &scope.1,
            )?;
            let result = crate::memory::ux::delete_memory_with_revision(
                &db,
                &settings.target_principal_id,
                memory_id,
                &crate::memory::ux::MemoryMutationRequest {
                    expected_revision: revision,
                },
                crate::now_ts_u64() as i64,
            )
            .map_err(|error| stable_error_code(error, "memory_forget_failed"))?;
            crate::memory::retrieval_async::invalidate_principal_query_cache(
                &settings.target_principal_id,
            );
            Ok(json!({
                "memory_id": result.memory_id,
                "revision": result.revision,
                "revision_id": result.revision_id,
                "undo_until_ts": result.undo_until_ts,
            }))
        }
        "correct" => {
            if !settings.generate_memory {
                return Err("memory_generate_disabled".to_string());
            }
            if is_child_task(&db, &task.task_id).unwrap_or(false) {
                return Err("memory_child_write_denied".to_string());
            }
            let memory_id = required_opaque_memory_id(args)?;
            let revision = required_revision(args)?;
            let content = required_text(args, "content", 1, 16_000)?;
            let (safe_content, redacted) =
                crate::skill_output_artifact::sensitivity_aware_text_model_view(content);
            if redacted || safe_content.trim() != content.trim() {
                return Err("memory_sensitive_content_blocked".to_string());
            }
            ensure_item_scope(
                &db,
                &settings.target_principal_id,
                memory_id,
                &scope.0,
                &scope.1,
            )?;
            let result = crate::memory::ux::correct_memory(
                &db,
                &settings.target_principal_id,
                &settings.target_principal_id,
                memory_id,
                &crate::memory::ux::MemoryCorrectionRequest {
                    expected_revision: revision,
                    content: content.to_string(),
                },
                crate::now_ts_u64() as i64,
            )
            .map_err(|error| stable_error_code(error, "memory_correct_failed"))?;
            crate::memory::retrieval_async::invalidate_principal_query_cache(
                &settings.target_principal_id,
            );
            Ok(json!({
                "memory_id": result.memory_id,
                "replacement_memory_id": result.replacement_memory_id,
                "revision": result.revision,
                "revision_id": result.revision_id,
                "undo_until_ts": result.undo_until_ts,
            }))
        }
        _ => Err("memory_action_invalid".to_string()),
    }
}

fn ensure_schema(db: &Connection) -> anyhow::Result<()> {
    crate::memory::ux::ensure_memory_ux_schema(db)?;
    crate::memory::indexing::ensure_retrieval_schema(db)?;
    let applied: Option<String> = db
        .query_row(
            "SELECT schema_digest FROM runtime_schema_migrations WHERE migration_id = ?1",
            [MIGRATION_ID],
            |row| row.get(0),
        )
        .optional()?;
    let digest = format!("sha256:{:x}", Sha256::digest(MIGRATION_SQL.as_bytes()));
    if let Some(applied) = applied {
        anyhow::ensure!(
            applied == digest,
            "memory_capability_migration_digest_mismatch"
        );
    }
    db.execute_batch(MIGRATION_SQL)?;
    db.execute(
        "INSERT INTO runtime_schema_migrations(migration_id, schema_digest, applied_at)
         VALUES (?1, ?2, ?3) ON CONFLICT(migration_id) DO NOTHING",
        params![MIGRATION_ID, digest, crate::now_ts()],
    )?;
    Ok(())
}

fn resolve_scope(
    db: &Connection,
    state: &AppState,
    task: &ClaimedTask,
    principal_id: &str,
    args: &Map<String, Value>,
) -> Result<(String, String), String> {
    match args
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("current_principal")
    {
        "current_principal" => Ok(("principal".to_string(), principal_id.to_string())),
        "current_conversation" => {
            let conversation_id = crate::conversation_state::task_conversation_id(task)
                .ok_or_else(|| "memory_current_conversation_unavailable".to_string())?;
            let scope_ref =
                crate::memory::scope::conversation_scope_ref(principal_id, &conversation_id)
                    .map_err(|_| "memory_current_conversation_unavailable".to_string())?;
            Ok(("conversation".to_string(), scope_ref))
        }
        "current_project" => {
            let identity = crate::memory::project_identity::resolve_project_identity(
                db,
                &state.skill_rt.workspace_root,
            )
            .map_err(|_| "memory_current_project_unavailable".to_string())?;
            Ok(("project".to_string(), identity.project_ref))
        }
        _ => Err("memory_scope_invalid".to_string()),
    }
}

fn search_items(
    db: &Connection,
    principal_id: &str,
    scope_kind: &str,
    scope_ref: &str,
    query: &str,
    cursor: Option<&str>,
    limit: usize,
    now_ts: i64,
) -> anyhow::Result<(Vec<MemoryCapabilityItem>, Option<String>)> {
    let stale_cutoff = now_ts.saturating_sub(30 * 86_400);
    let pattern = format!("%{}%", escape_like(query));
    let cursor = cursor
        .map(|value| parse_cursor(value, principal_id, scope_kind, scope_ref))
        .transpose()?;
    let mut statement = db.prepare(
        "SELECT memory_id, row_revision, kind, scope_kind, content, updated_at_ts,
                last_verified_at_ts, trust_tier, evidence_available
         FROM (
           SELECT memory_id, row_revision, 'fact' AS kind, scope_kind, scope_ref,
                  fact_text AS content, COALESCE(modified_at_ts, updated_at_ts) AS updated_at_ts,
                  last_verified_at_ts, trust_tier,
                  CASE WHEN evidence_refs_json != '[]' THEN 1 ELSE 0 END AS evidence_available
             FROM memory_facts WHERE principal_id = ?1 AND status = 'active'
           UNION ALL
           SELECT memory_id, row_revision, 'preference', scope_kind, scope_ref,
                  pref_key || ': ' || pref_value,
                  COALESCE(modified_at_ts, updated_at_ts), last_verified_at_ts, trust_tier,
                  CASE WHEN evidence_refs_json != '[]' THEN 1 ELSE 0 END
             FROM user_preferences WHERE principal_id = ?1 AND deleted_at_ts IS NULL
         )
         WHERE scope_kind = ?2 AND scope_ref = ?3
           AND (?4 = '%%' OR content LIKE ?4 ESCAPE '\\')
           AND (?5 IS NULL OR updated_at_ts < ?5
                OR (updated_at_ts = ?5 AND memory_id < ?6))
         ORDER BY updated_at_ts DESC, memory_id DESC LIMIT ?7",
    )?;
    let rows = statement.query_map(
        params![
            principal_id,
            scope_kind,
            scope_ref,
            pattern,
            cursor.as_ref().map(|value| value.0),
            cursor.as_ref().map(|value| value.1.as_str()),
            limit.saturating_add(1) as i64,
        ],
        |row| {
            let content = row.get::<_, String>(4)?;
            let updated_at_ts = row.get::<_, i64>(5)?;
            let verified = row.get::<_, Option<i64>>(6)?;
            Ok(MemoryCapabilityItem {
                memory_id: row.get(0)?,
                revision: row.get(1)?,
                kind: row.get(2)?,
                scope: row.get(3)?,
                excerpt: crate::truncate_text(&content, 600),
                freshness: if verified.unwrap_or(updated_at_ts) < stale_cutoff {
                    "stale".to_string()
                } else {
                    "fresh".to_string()
                },
                trust: row.get(7)?,
                source_available: row.get::<_, i64>(8)? != 0,
            })
        },
    )?;
    let mut items = rows.collect::<Result<Vec<_>, _>>()?;
    let has_more = items.len() > limit;
    if has_more {
        items.truncate(limit);
    }
    let continuation_token = if has_more {
        let last = items
            .last()
            .ok_or_else(|| anyhow::anyhow!("memory_cursor_empty_page"))?;
        let updated_at_ts: i64 = db.query_row(
            "SELECT updated_at_ts FROM (
               SELECT memory_id, COALESCE(modified_at_ts, updated_at_ts) AS updated_at_ts
                 FROM memory_facts WHERE principal_id = ?1
               UNION ALL
               SELECT memory_id, COALESCE(modified_at_ts, updated_at_ts)
                 FROM user_preferences WHERE principal_id = ?1
             ) WHERE memory_id = ?2",
            params![principal_id, last.memory_id],
            |row| row.get(0),
        )?;
        Some(make_cursor(
            principal_id,
            scope_kind,
            scope_ref,
            updated_at_ts,
            &last.memory_id,
        ))
    } else {
        None
    };
    Ok((items, continuation_token))
}

fn ensure_item_scope(
    db: &Connection,
    principal_id: &str,
    memory_id: &str,
    scope_kind: &str,
    scope_ref: &str,
) -> Result<(), String> {
    let owned = db
        .query_row(
            "SELECT 1 FROM (
               SELECT memory_id, principal_id, scope_kind, scope_ref
                 FROM memory_facts WHERE status = 'active'
               UNION ALL
               SELECT memory_id, principal_id, scope_kind, scope_ref
                 FROM user_preferences WHERE deleted_at_ts IS NULL
             ) WHERE memory_id = ?1 AND principal_id = ?2
                 AND scope_kind = ?3 AND scope_ref = ?4 LIMIT 1",
            params![memory_id, principal_id, scope_kind, scope_ref],
            |_| Ok(()),
        )
        .optional()
        .map_err(|_| "memory_scope_check_failed".to_string())?
        .is_some();
    owned
        .then_some(())
        .ok_or_else(|| "memory_item_not_found_in_scope".to_string())
}

fn make_cursor(
    principal_id: &str,
    scope_kind: &str,
    scope_ref: &str,
    updated_at_ts: i64,
    memory_id: &str,
) -> String {
    let material =
        format!("{principal_id}\0{scope_kind}\0{scope_ref}\0{updated_at_ts}\0{memory_id}");
    let digest = format!("{:x}", Sha256::digest(material.as_bytes()));
    format!(
        "memory_cursor_v1.{updated_at_ts}.{memory_id}.{}",
        &digest[..16]
    )
}

fn parse_cursor(
    value: &str,
    principal_id: &str,
    scope_kind: &str,
    scope_ref: &str,
) -> anyhow::Result<(i64, String)> {
    let parts = value.split('.').collect::<Vec<_>>();
    anyhow::ensure!(
        parts.len() == 4 && parts[0] == "memory_cursor_v1",
        "memory_cursor_invalid"
    );
    let updated_at_ts = parts[1]
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("memory_cursor_invalid"))?;
    let memory_id = parts[2];
    anyhow::ensure!(
        required_opaque_memory_id(&Map::from_iter([(
            "memory_id".to_string(),
            Value::String(memory_id.to_string()),
        )]))
        .is_ok(),
        "memory_cursor_invalid"
    );
    let expected = make_cursor(
        principal_id,
        scope_kind,
        scope_ref,
        updated_at_ts,
        memory_id,
    );
    anyhow::ensure!(expected == value, "memory_cursor_invalid");
    Ok((updated_at_ts, memory_id.to_string()))
}

#[allow(clippy::too_many_arguments)]
fn save_item(
    db: &mut Connection,
    task: &ClaimedTask,
    principal_id: &str,
    scope_kind: &str,
    scope_ref: &str,
    kind: &str,
    item_key: Option<&str>,
    content: &str,
    idempotency_key: &str,
    evidence_refs: &[String],
) -> anyhow::Result<Value> {
    let content_digest = digest_text(content);
    if let Some(existing) = db
        .query_row(
            "SELECT content_digest, memory_id, memory_kind, scope_kind
             FROM memory_capability_write_receipts
             WHERE principal_id = ?1 AND idempotency_key = ?2",
            params![principal_id, idempotency_key],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .optional()?
    {
        anyhow::ensure!(
            existing.0 == content_digest && existing.2 == kind && existing.3 == scope_kind,
            "memory_idempotency_conflict"
        );
        return Ok(json!({
            "write_status": "existing",
            "memory_id": existing.1,
            "content_digest": content_digest,
        }));
    }
    let memory_id = format!("memory_{}", uuid::Uuid::new_v4().simple());
    let now = crate::now_ts_u64() as i64;
    let source_refs = serde_json::to_string(&[format!("task:{}", task.task_id)])?;
    let evidence_json = serde_json::to_string(evidence_refs)?;
    let tx = db.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let row_id = if kind == "preference" {
        let key = item_key.ok_or_else(|| anyhow::anyhow!("memory_preference_key_required"))?;
        let conflict: Option<String> = tx
            .query_row(
                "SELECT memory_id FROM user_preferences
                 WHERE principal_id = ?1 AND scope_kind = ?2 AND scope_ref = ?3
                   AND pref_key = ?4 AND deleted_at_ts IS NULL LIMIT 1",
                params![principal_id, scope_kind, scope_ref, key],
                |row| row.get(0),
            )
            .optional()?;
        anyhow::ensure!(conflict.is_none(), "memory_preference_conflict_use_correct");
        tx.execute(
            "INSERT INTO user_preferences(
                memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                pref_key, pref_value, confidence, source, updated_at, updated_at_ts,
                origin, row_revision, legacy_scope_inferred, content_digest, idempotency_key,
                source_task_id, source_refs_json, evidence_refs_json, trust_tier, sensitivity,
                last_verified_at_ts, modified_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 1.0,
                       'current_user_message', ?10, ?11, 'agent_explicit', 1, 0,
                       ?12, ?13, ?14, ?15, ?16, 'user_message_evidence', 'normal', ?11, ?11)",
            params![
                memory_id,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref().unwrap_or_default(),
                principal_id,
                scope_kind,
                scope_ref,
                key,
                content,
                now.to_string(),
                now,
                content_digest,
                idempotency_key,
                task.task_id,
                source_refs,
                evidence_json,
            ],
        )?;
        tx.last_insert_rowid()
    } else {
        let namespace = if kind == "session_note" {
            "session_notes"
        } else if scope_kind == "project" {
            "project_facts"
        } else {
            "user_profile"
        };
        let fact_key = item_key
            .map(str::to_string)
            .unwrap_or_else(|| format!("explicit_{}", &content_digest[7..23]));
        tx.execute(
            "INSERT INTO memory_facts(
                memory_id, user_id, chat_id, user_key, principal_id, scope_kind, scope_ref,
                namespace, fact_key, fact_value, fact_text, confidence, source_kind,
                source_ref, source_memory_ids_json, reason, created_at_ts, updated_at_ts,
                safety_flag, status, origin, row_revision, legacy_scope_inferred,
                content_digest, idempotency_key, source_task_id, source_refs_json,
                evidence_refs_json, trust_tier, sensitivity, observed_at_ts,
                last_verified_at_ts, modified_at_ts
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 1.0,
                       'current_user_message', ?11, '[]', 'agent_explicit_save', ?12, ?12,
                       'normal', 'active', 'agent_explicit', 1, 0, ?13, ?14, ?15,
                       ?16, ?17, 'user_message_evidence', 'normal', ?12, ?12, ?12)",
            params![
                memory_id,
                task.user_id,
                task.chat_id,
                task.user_key.as_deref().unwrap_or_default(),
                principal_id,
                scope_kind,
                scope_ref,
                namespace,
                fact_key,
                content,
                format!("task:{}", task.task_id),
                now,
                content_digest,
                idempotency_key,
                task.task_id,
                source_refs,
                evidence_json,
            ],
        )?;
        tx.last_insert_rowid()
    };
    tx.execute(
        "INSERT INTO memory_capability_write_receipts(
            principal_id, idempotency_key, content_digest, memory_id, memory_kind,
            scope_kind, scope_ref, source_task_id, created_at_ts
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            principal_id,
            idempotency_key,
            content_digest,
            memory_id,
            kind,
            scope_kind,
            scope_ref,
            task.task_id,
            now,
        ],
    )?;
    if kind == "preference" {
        crate::memory::indexing::index_preference_entries(
            &tx,
            task.user_id,
            task.chat_id,
            task.user_key.as_deref().unwrap_or_default(),
            &[(
                item_key.unwrap_or_default().to_string(),
                content.to_string(),
                1.0,
                "current_user_message".to_string(),
            )],
            now,
        )?;
    } else {
        crate::memory::indexing::upsert_memory_fact_retrieval_row(
            &tx,
            task.user_id,
            task.user_key.as_deref().unwrap_or_default(),
            if kind == "session_note" {
                "session_notes"
            } else if scope_kind == "project" {
                "project_facts"
            } else {
                "user_profile"
            },
            row_id,
            content,
            1.0,
            now,
        )?;
    }
    tx.commit()?;
    crate::memory::retrieval_async::invalidate_principal_query_cache(principal_id);
    Ok(json!({
        "write_status": "created",
        "memory_id": memory_id,
        "revision": 1,
        "content_digest": content_digest,
        "evidence_refs": evidence_refs,
    }))
}

fn evidence_refs(task: &ClaimedTask, args: &Map<String, Value>) -> Result<Vec<String>, String> {
    let mut refs = args
        .get("evidence_refs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if has_current_request_text(task) {
        refs.push(format!("task:{}:current_user_message", task.task_id));
    }
    refs.sort();
    refs.dedup();
    if refs.is_empty() || refs.len() > 16 || refs.iter().any(|value| value.len() > 512) {
        return Err("memory_evidence_required".to_string());
    }
    Ok(refs)
}

fn has_current_request_text(task: &ClaimedTask) -> bool {
    let Ok(payload) = serde_json::from_str::<Value>(&task.payload_json) else {
        return false;
    };
    ["text", "request_text", "prompt", "content"]
        .into_iter()
        .find_map(|key| payload.get(key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some()
}

fn is_child_task(db: &Connection, task_id: &str) -> anyhow::Result<bool> {
    Ok(db
        .query_row(
            "SELECT parent_task_id FROM child_task_graph_nodes WHERE child_task_id = ?1",
            [task_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .is_some())
}

fn required_text<'a>(
    args: &'a Map<String, Value>,
    key: &str,
    min: usize,
    max: usize,
) -> Result<&'a str, String> {
    let value = args
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| value.chars().count() >= min && value.chars().count() <= max)
        .ok_or_else(|| format!("memory_{key}_invalid"))?;
    Ok(value)
}

fn required_enum<'a>(
    args: &'a Map<String, Value>,
    key: &str,
    allowed: &[&str],
) -> Result<&'a str, String> {
    let value = required_text(args, key, 1, 64)?;
    allowed
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| format!("memory_{key}_invalid"))
}

fn optional_limit(args: &Map<String, Value>, default: usize, max: usize) -> Result<usize, String> {
    let value = args
        .get("limit")
        .map(|value| value.as_u64().and_then(|value| usize::try_from(value).ok()))
        .unwrap_or(Some(default))
        .ok_or_else(|| "memory_limit_invalid".to_string())?;
    (1..=max)
        .contains(&value)
        .then_some(value)
        .ok_or_else(|| "memory_limit_invalid".to_string())
}

fn optional_text<'a>(
    args: &'a Map<String, Value>,
    key: &str,
    max: usize,
) -> Result<Option<&'a str>, String> {
    args.get(key)
        .map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty() && value.chars().count() <= max)
                .ok_or_else(|| format!("memory_{key}_invalid"))
        })
        .transpose()
}

fn required_opaque_memory_id(args: &Map<String, Value>) -> Result<&str, String> {
    let value = required_text(args, "memory_id", 16, 128)?;
    let valid = value.starts_with("memory_")
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_');
    valid
        .then_some(value)
        .ok_or_else(|| "memory_id_invalid".to_string())
}

fn required_revision(args: &Map<String, Value>) -> Result<i64, String> {
    args.get("expected_revision")
        .and_then(Value::as_i64)
        .filter(|value| *value >= 1)
        .ok_or_else(|| "memory_expected_revision_invalid".to_string())
}

fn validate_kind_scope(kind: &str, scope_kind: &str) -> Result<(), String> {
    let valid = match kind {
        "preference" => scope_kind == "principal",
        "session_note" => scope_kind == "conversation",
        "fact" => matches!(scope_kind, "principal" | "project"),
        _ => false,
    };
    valid
        .then_some(())
        .ok_or_else(|| "memory_kind_scope_invalid".to_string())
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn digest_text(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

fn stable_error_code(error: anyhow::Error, fallback: &str) -> String {
    let code = error.to_string();
    if code.starts_with("memory_") {
        code
    } else {
        fallback.to_string()
    }
}

fn outcome(status: &str, action: &str, error_code: Option<&str>, data: Value) -> String {
    let mut payload = json!({
        "schema_version": 1,
        "source_skill": SOURCE_SKILL,
        "status": status,
        "action": action,
        "error_code": error_code,
        "message_key": error_code,
        "retryable": false,
        "data": data,
    });
    if error_code.is_none() {
        if let Some(object) = payload.as_object_mut() {
            object.remove("error_code");
            object.remove("message_key");
        }
    }
    payload.to_string()
}

#[cfg(test)]
#[path = "builtin_memory_store_tests.rs"]
mod tests;
