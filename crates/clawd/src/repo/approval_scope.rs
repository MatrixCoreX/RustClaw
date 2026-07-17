use hmac::{Hmac, Mac};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::approval_grant::{
    ApprovalBinding, ApprovalScopeBinding, APPROVAL_SCOPE_GRANT_TTL_SECONDS,
};
use crate::{AppState, ClaimedTask};

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ApprovalScopeGrantRecord {
    pub(crate) grant_id: String,
    pub(crate) scope_kind: String,
    pub(crate) scope_fingerprint: String,
    pub(crate) issued_at: i64,
    pub(crate) expires_at: i64,
}

impl ApprovalScopeGrantRecord {
    pub(crate) fn decision_json(&self, binding: &ApprovalBinding) -> Value {
        serde_json::json!({
            "schema_version": 1,
            "status": "scope_grant_matched",
            "grant_id": self.grant_id,
            "scope_kind": self.scope_kind,
            "scope_fingerprint": self.scope_fingerprint,
            "action_fingerprint": binding.action_fingerprint,
            "action_count": binding.action_count,
            "issued_at": self.issued_at,
            "expires_at": self.expires_at,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ApprovalScopeGrantView {
    pub(crate) grant_id: String,
    pub(crate) scope_kind: String,
    pub(crate) scope_fingerprint: String,
    pub(crate) scope: Value,
    pub(crate) channel: String,
    pub(crate) chat_id: i64,
    pub(crate) issued_at: i64,
    pub(crate) expires_at: i64,
    pub(crate) revoked_at: Option<i64>,
    pub(crate) use_count: u64,
    pub(crate) last_used_at: Option<i64>,
    pub(crate) source_task_id: String,
}

pub(crate) fn ensure_approval_scope_grant_schema(db: &Connection) -> anyhow::Result<()> {
    db.execute_batch(
        "CREATE TABLE IF NOT EXISTS approval_scope_grants (
            grant_id            TEXT PRIMARY KEY,
            actor_key_hash      TEXT NOT NULL,
            user_id             INTEGER NOT NULL,
            chat_id             INTEGER NOT NULL,
            channel             TEXT NOT NULL,
            scope_kind          TEXT NOT NULL,
            scope_fingerprint   TEXT NOT NULL,
            scope_json          TEXT NOT NULL,
            signature           TEXT NOT NULL,
            source_task_id      TEXT NOT NULL,
            issued_at           INTEGER NOT NULL,
            expires_at          INTEGER NOT NULL,
            revoked_at          INTEGER,
            use_count           INTEGER NOT NULL DEFAULT 0,
            last_used_at        INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_approval_scope_grants_match
        ON approval_scope_grants(
            actor_key_hash, user_id, chat_id, channel, scope_fingerprint, expires_at
        );
        CREATE INDEX IF NOT EXISTS idx_approval_scope_grants_actor
        ON approval_scope_grants(actor_key_hash, issued_at DESC);",
    )?;
    Ok(())
}

pub(crate) fn insert_approval_scope_grant(
    db: &Connection,
    source_task_id: &str,
    user_id: i64,
    chat_id: i64,
    channel: &str,
    actor_user_key: &str,
    scope: &ApprovalScopeBinding,
    now_ts: i64,
) -> anyhow::Result<ApprovalScopeGrantRecord> {
    ensure_approval_scope_grant_schema(db)?;
    let actor_user_key = crate::normalize_user_key(actor_user_key);
    if actor_user_key.is_empty()
        || source_task_id.trim().is_empty()
        || channel.trim().is_empty()
        || scope.scope_kind != "session"
        || scope.scope_fingerprint.trim().is_empty()
        || scope.entries.is_empty()
    {
        anyhow::bail!("approval_scope_grant_invalid");
    }
    let grant_id = format!("scope-grant-{}", uuid::Uuid::new_v4());
    let actor_key_hash = actor_key_hash(&actor_user_key);
    let scope_json = serde_json::to_string(scope)?;
    let expires_at = now_ts.saturating_add(APPROVAL_SCOPE_GRANT_TTL_SECONDS);
    let signature = sign_grant(
        &actor_user_key,
        &grant_id,
        &actor_key_hash,
        user_id,
        chat_id,
        channel,
        &scope.scope_kind,
        &scope.scope_fingerprint,
        &scope_json,
        source_task_id,
        now_ts,
        expires_at,
    )?;
    db.execute(
        "INSERT INTO approval_scope_grants (
            grant_id, actor_key_hash, user_id, chat_id, channel, scope_kind,
            scope_fingerprint, scope_json, signature, source_task_id, issued_at, expires_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            grant_id,
            actor_key_hash,
            user_id,
            chat_id,
            channel.trim(),
            scope.scope_kind,
            scope.scope_fingerprint,
            scope_json,
            signature,
            source_task_id.trim(),
            now_ts,
            expires_at,
        ],
    )?;
    Ok(ApprovalScopeGrantRecord {
        grant_id,
        scope_kind: scope.scope_kind.clone(),
        scope_fingerprint: scope.scope_fingerprint.clone(),
        issued_at: now_ts,
        expires_at,
    })
}

pub(crate) fn match_approval_scope_grant(
    state: &AppState,
    task: &ClaimedTask,
    binding: &ApprovalBinding,
) -> anyhow::Result<Option<ApprovalScopeGrantRecord>> {
    let Some(scope) = binding.scope.as_ref() else {
        return Ok(None);
    };
    let Some(actor_user_key) = task
        .user_key
        .as_deref()
        .map(crate::normalize_user_key)
        .filter(|value| !value.is_empty())
    else {
        return Ok(None);
    };
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db pool: {err}"))?;
    ensure_approval_scope_grant_schema(&db)?;
    let now_ts = crate::now_ts_u64() as i64;
    let key_hash = actor_key_hash(&actor_user_key);
    let row = db
        .query_row(
            "SELECT grant_id, actor_key_hash, user_id, chat_id, channel, scope_kind,
                    scope_fingerprint, scope_json, signature, source_task_id, issued_at, expires_at
             FROM approval_scope_grants
             WHERE actor_key_hash = ?1
               AND user_id = ?2
               AND chat_id = ?3
               AND channel = ?4
               AND scope_kind = 'session'
               AND scope_fingerprint = ?5
               AND revoked_at IS NULL
               AND expires_at > ?6
             ORDER BY issued_at DESC
             LIMIT 1",
            params![
                key_hash,
                task.user_id,
                task.chat_id,
                task.channel.trim(),
                scope.scope_fingerprint,
                now_ts,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                ))
            },
        )
        .optional()?;
    let Some((
        grant_id,
        stored_key_hash,
        user_id,
        chat_id,
        channel,
        scope_kind,
        scope_fingerprint,
        scope_json,
        signature,
        source_task_id,
        issued_at,
        expires_at,
    )) = row
    else {
        return Ok(None);
    };
    let signature_valid = verify_grant_signature(
        &actor_user_key,
        &signature,
        &grant_id,
        &stored_key_hash,
        user_id,
        chat_id,
        &channel,
        &scope_kind,
        &scope_fingerprint,
        &scope_json,
        &source_task_id,
        issued_at,
        expires_at,
    );
    if !signature_valid
        || stored_key_hash != key_hash
        || user_id != task.user_id
        || chat_id != task.chat_id
        || channel != task.channel.trim()
        || scope_fingerprint != scope.scope_fingerprint
    {
        return Ok(None);
    }
    let updated = db.execute(
        "UPDATE approval_scope_grants
         SET use_count = use_count + 1, last_used_at = ?2
         WHERE grant_id = ?1 AND revoked_at IS NULL AND expires_at > ?2",
        params![grant_id, now_ts],
    )?;
    if updated == 0 {
        return Ok(None);
    }
    Ok(Some(ApprovalScopeGrantRecord {
        grant_id,
        scope_kind,
        scope_fingerprint,
        issued_at,
        expires_at,
    }))
}

pub(crate) fn list_approval_scope_grants(
    state: &AppState,
    actor_user_key: &str,
) -> anyhow::Result<Vec<ApprovalScopeGrantView>> {
    let actor_user_key = crate::normalize_user_key(actor_user_key);
    if actor_user_key.is_empty() {
        return Ok(Vec::new());
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db pool: {err}"))?;
    ensure_approval_scope_grant_schema(&db)?;
    let mut statement = db.prepare(
        "SELECT grant_id, scope_kind, scope_fingerprint, scope_json, channel, chat_id,
                issued_at, expires_at, revoked_at, use_count, last_used_at, source_task_id
         FROM approval_scope_grants
         WHERE actor_key_hash = ?1
         ORDER BY issued_at DESC
         LIMIT 100",
    )?;
    let rows = statement
        .query_map(params![actor_key_hash(&actor_user_key)], |row| {
            let raw_scope = row.get::<_, String>(3)?;
            Ok(ApprovalScopeGrantView {
                grant_id: row.get(0)?,
                scope_kind: row.get(1)?,
                scope_fingerprint: row.get(2)?,
                scope: serde_json::from_str(&raw_scope).unwrap_or(Value::Null),
                channel: row.get(4)?,
                chat_id: row.get(5)?,
                issued_at: row.get(6)?,
                expires_at: row.get(7)?,
                revoked_at: row.get(8)?,
                use_count: row.get::<_, i64>(9)?.max(0) as u64,
                last_used_at: row.get(10)?,
                source_task_id: row.get(11)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

pub(crate) fn revoke_approval_scope_grant(
    state: &AppState,
    actor_user_key: &str,
    grant_id: &str,
) -> anyhow::Result<bool> {
    let actor_user_key = crate::normalize_user_key(actor_user_key);
    let grant_id = grant_id.trim();
    if actor_user_key.is_empty() || grant_id.is_empty() {
        return Ok(false);
    }
    let db = state
        .core
        .db
        .get()
        .map_err(|err| anyhow::anyhow!("db pool: {err}"))?;
    ensure_approval_scope_grant_schema(&db)?;
    let now_ts = crate::now_ts_u64() as i64;
    Ok(db.execute(
        "UPDATE approval_scope_grants
         SET revoked_at = ?3
         WHERE grant_id = ?1 AND actor_key_hash = ?2 AND revoked_at IS NULL",
        params![grant_id, actor_key_hash(&actor_user_key), now_ts],
    )? > 0)
}

#[allow(clippy::too_many_arguments)]
fn sign_grant(
    actor_user_key: &str,
    grant_id: &str,
    actor_key_hash: &str,
    user_id: i64,
    chat_id: i64,
    channel: &str,
    scope_kind: &str,
    scope_fingerprint: &str,
    scope_json: &str,
    source_task_id: &str,
    issued_at: i64,
    expires_at: i64,
) -> anyhow::Result<String> {
    let mut mac = HmacSha256::new_from_slice(actor_user_key.as_bytes())
        .map_err(|_| anyhow::anyhow!("approval_scope_signing_key_invalid"))?;
    mac.update(
        signature_payload(
            grant_id,
            actor_key_hash,
            user_id,
            chat_id,
            channel,
            scope_kind,
            scope_fingerprint,
            scope_json,
            source_task_id,
            issued_at,
            expires_at,
        )
        .as_bytes(),
    );
    Ok(format!("hmac-sha256:{:x}", mac.finalize().into_bytes()))
}

#[allow(clippy::too_many_arguments)]
fn verify_grant_signature(
    actor_user_key: &str,
    signature: &str,
    grant_id: &str,
    actor_key_hash: &str,
    user_id: i64,
    chat_id: i64,
    channel: &str,
    scope_kind: &str,
    scope_fingerprint: &str,
    scope_json: &str,
    source_task_id: &str,
    issued_at: i64,
    expires_at: i64,
) -> bool {
    let Some(raw_signature) = signature.strip_prefix("hmac-sha256:") else {
        return false;
    };
    let Some(signature_bytes) = decode_hex(raw_signature) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(actor_user_key.as_bytes()) else {
        return false;
    };
    mac.update(
        signature_payload(
            grant_id,
            actor_key_hash,
            user_id,
            chat_id,
            channel,
            scope_kind,
            scope_fingerprint,
            scope_json,
            source_task_id,
            issued_at,
            expires_at,
        )
        .as_bytes(),
    );
    mac.verify_slice(&signature_bytes).is_ok()
}

#[allow(clippy::too_many_arguments)]
fn signature_payload(
    grant_id: &str,
    actor_key_hash: &str,
    user_id: i64,
    chat_id: i64,
    channel: &str,
    scope_kind: &str,
    scope_fingerprint: &str,
    scope_json: &str,
    source_task_id: &str,
    issued_at: i64,
    expires_at: i64,
) -> String {
    serde_json::json!({
        "schema_version": 1,
        "grant_id": grant_id,
        "actor_key_hash": actor_key_hash,
        "user_id": user_id,
        "chat_id": chat_id,
        "channel": channel,
        "scope_kind": scope_kind,
        "scope_fingerprint": scope_fingerprint,
        "scope_json": scope_json,
        "source_task_id": source_task_id,
        "issued_at": issued_at,
        "expires_at": expires_at,
    })
    .to_string()
}

fn actor_key_hash(actor_user_key: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(actor_user_key.as_bytes()))
}

fn decode_hex(value: &str) -> Option<Vec<u8>> {
    if value.len() % 2 != 0 {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

#[cfg(test)]
#[path = "approval_scope_tests.rs"]
mod tests;
