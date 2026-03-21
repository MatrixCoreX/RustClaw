use std::hash::{Hash, Hasher};

use claw_core::types::{AuthIdentity, ChannelKind, SubmitTaskRequest};
use rusqlite::{params, OptionalExtension};
use serde_json::Value;
use uuid::Uuid;

use crate::{
    is_affirmation_click_text, main_flow_rules, normalize_affirmation_text,
    normalize_external_id_opt, now_ts, AppState,
};

pub(crate) struct SubmitTaskContext {
    pub(crate) resolved_identity: Option<AuthIdentity>,
    pub(crate) effective_user_key: Option<String>,
    pub(crate) requested_user_id: Option<i64>,
    pub(crate) effective_user_id: i64,
    pub(crate) channel: ChannelKind,
    pub(crate) effective_agent_id: String,
    pub(crate) normalized_external_user_id: Option<String>,
    pub(crate) normalized_external_chat_id: Option<String>,
    pub(crate) effective_chat_id: i64,
}

pub(crate) enum SubmitTaskContextError {
    AuthLookup(anyhow::Error),
    InvalidUserKey,
    UnknownAgentId(String),
    MissingChatId,
}

pub(crate) enum SubmitTaskAccessError {
    MissingUserId,
    Database(anyhow::Error),
    UnauthorizedUser,
}

pub(crate) enum SubmitTaskLimitError {
    RateLimiterPoisoned,
    RateLimited(String),
    QueueCount(anyhow::Error),
    QueueFull,
}

#[derive(Debug, Clone)]
pub(crate) struct RecentFailedResumeContext {
    pub(crate) resume_context: Value,
    pub(crate) failed_ts: i64,
    pub(crate) has_newer_successful_ask_after_failed_task: bool,
}

pub(crate) fn maybe_find_submit_task_dedup(
    state: &AppState,
    kind: &claw_core::types::TaskKind,
    payload: &Value,
    effective_user_id: i64,
    effective_chat_id: i64,
) -> Option<(Uuid, String)> {
    if !matches!(kind, claw_core::types::TaskKind::Ask) {
        return None;
    }
    let text = payload.get("text").and_then(|v| v.as_str())?;
    let existing_id = find_recent_duplicate_affirmation_task(
        state,
        effective_user_id,
        effective_chat_id,
        text,
        main_flow_rules(state).duplicate_affirmation_window_secs,
    )?;
    Some((existing_id, text.to_string()))
}

pub(crate) fn submit_task_audit_detail(
    call_id: &str,
    task_id: &Uuid,
    kind: &str,
    effective_chat_id: i64,
    effective_user_key: Option<&str>,
) -> String {
    serde_json::json!({
        "call_id": call_id,
        "task_id": task_id,
        "kind": kind,
        "chat_id": effective_chat_id,
        "user_key": effective_user_key,
    })
    .to_string()
}

pub(crate) fn task_count_by_status(state: &AppState, status: &str) -> anyhow::Result<usize> {
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    let count: i64 = db.query_row(
        "SELECT COUNT(1) FROM tasks WHERE status = ?1",
        params![status],
        |row| row.get(0),
    )?;

    Ok(count as usize)
}

pub(crate) fn resolve_submit_task_context(
    state: &AppState,
    req: &SubmitTaskRequest,
    default_agent_id: &str,
) -> Result<SubmitTaskContext, SubmitTaskContextError> {
    let resolved_identity = match req.user_key.as_deref() {
        Some(user_key) => crate::resolve_auth_identity_by_key(state, user_key)
            .map_err(SubmitTaskContextError::AuthLookup)?,
        None => None,
    };
    if req.user_key.is_some() && resolved_identity.is_none() {
        return Err(SubmitTaskContextError::InvalidUserKey);
    }
    let effective_user_key = resolved_identity.as_ref().map(|v| v.user_key.clone());
    let requested_user_id = req.user_id;
    let requested_chat_id = req.chat_id;
    let effective_user_id = resolved_identity
        .as_ref()
        .map(|v| v.user_id)
        .or(requested_user_id)
        .unwrap_or_default();
    let channel = req.channel.unwrap_or(ChannelKind::Telegram);
    let requested_agent_id = req
        .payload
        .get("agent_id")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let effective_agent_id = if let Some(agent_id) = requested_agent_id.as_deref() {
        if let Some(normalized) = state.normalize_known_agent_id(Some(agent_id)) {
            normalized
        } else {
            return Err(SubmitTaskContextError::UnknownAgentId(agent_id.to_string()));
        }
    } else {
        default_agent_id.to_string()
    };
    let normalized_external_user_id = normalize_external_id_opt(req.external_user_id.as_deref());
    let normalized_external_chat_id = normalize_external_id_opt(req.external_chat_id.as_deref());
    let public_conversation_seed = format!(
        "public:{}:{}",
        requested_user_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| "anon".to_string()),
        requested_chat_id
            .map(|v| v.to_string())
            .unwrap_or_else(|| "chat".to_string())
    );
    let effective_chat_id = if let Some(user_key) = effective_user_key.as_deref() {
        build_conversation_chat_id(
            channel_kind_name(channel),
            normalized_external_user_id.as_deref(),
            normalized_external_chat_id.as_deref(),
            user_key,
        )
    } else if channel_allows_public_access(channel)
        && (normalized_external_user_id.is_some() || normalized_external_chat_id.is_some())
    {
        build_conversation_chat_id(
            channel_kind_name(channel),
            normalized_external_user_id.as_deref(),
            normalized_external_chat_id.as_deref(),
            &public_conversation_seed,
        )
    } else if let Some(chat_id) = requested_chat_id {
        chat_id
    } else {
        return Err(SubmitTaskContextError::MissingChatId);
    };

    Ok(SubmitTaskContext {
        resolved_identity,
        effective_user_key,
        requested_user_id,
        effective_user_id,
        channel,
        effective_agent_id,
        normalized_external_user_id,
        normalized_external_chat_id,
        effective_chat_id,
    })
}

pub(crate) fn stable_i64_from_key(input: &str) -> i64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    input.hash(&mut h);
    let v = h.finish() & (i64::MAX as u64);
    v as i64
}

pub(crate) fn channel_kind_name(channel: ChannelKind) -> &'static str {
    match channel {
        ChannelKind::Telegram => "telegram",
        ChannelKind::Whatsapp => "whatsapp",
        ChannelKind::Ui => "ui",
        ChannelKind::Feishu => "feishu",
        ChannelKind::Lark => "lark",
    }
}

pub(crate) fn build_conversation_chat_id(
    channel: &str,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
    user_key: &str,
) -> i64 {
    let scope = external_chat_id
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .or_else(|| external_user_id.map(str::trim).filter(|v| !v.is_empty()))
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("principal:{user_key}"));
    stable_i64_from_key(&format!("conv:{channel}:{scope}"))
}

pub(crate) fn is_user_allowed(state: &AppState, user_id: i64) -> bool {
    let Ok(db) = state.db.lock() else {
        return false;
    };

    let query = db
        .query_row(
            "SELECT is_allowed FROM users WHERE user_id = ?1",
            params![user_id],
            |row| row.get::<_, i64>(0),
        )
        .optional();

    matches!(query, Ok(Some(v)) if v == 1)
}

pub(crate) fn channel_allows_public_access(channel: ChannelKind) -> bool {
    matches!(
        channel,
        ChannelKind::Telegram | ChannelKind::Whatsapp | ChannelKind::Feishu | ChannelKind::Lark
    )
}

pub(crate) fn upsert_public_channel_user(state: &AppState, user_id: i64) -> anyhow::Result<()> {
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;
    db.execute(
        "INSERT INTO users (user_id, role, is_allowed, created_at, last_seen)
         VALUES (?1, 'user', 1, ?2, ?2)
         ON CONFLICT(user_id) DO UPDATE SET is_allowed=1, last_seen=excluded.last_seen",
        params![user_id, now],
    )?;
    Ok(())
}

pub(crate) fn check_submit_task_access(
    state: &AppState,
    ctx: &SubmitTaskContext,
) -> Result<(), SubmitTaskAccessError> {
    if ctx.resolved_identity.is_some() {
        return Ok(());
    }
    let Some(request_user_id) = ctx.requested_user_id else {
        return Err(SubmitTaskAccessError::MissingUserId);
    };
    if channel_allows_public_access(ctx.channel) {
        upsert_public_channel_user(state, request_user_id)
            .map_err(SubmitTaskAccessError::Database)?;
        return Ok(());
    }
    if !is_user_allowed(state, request_user_id) {
        return Err(SubmitTaskAccessError::UnauthorizedUser);
    }
    Ok(())
}

pub(crate) fn check_submit_task_limits(
    state: &AppState,
    effective_user_id: i64,
) -> Result<(), SubmitTaskLimitError> {
    let limit_result = {
        let mut limiter = state
            .rate_limiter
            .lock()
            .map_err(|_| SubmitTaskLimitError::RateLimiterPoisoned)?;
        limiter.check_and_record(effective_user_id)
    };
    if let Err(kind) = limit_result {
        return Err(SubmitTaskLimitError::RateLimited(kind.to_string()));
    }

    let queued_count = task_count_by_status(state, &main_flow_rules(state).task_status_queued)
        .map_err(SubmitTaskLimitError::QueueCount)?;
    if queued_count >= state.queue_limit {
        return Err(SubmitTaskLimitError::QueueFull);
    }
    Ok(())
}

pub(crate) fn find_recent_duplicate_affirmation_task(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
    ask_text: &str,
    window_secs: i64,
) -> Option<Uuid> {
    let rules = main_flow_rules(state);
    if !is_affirmation_click_text(state, ask_text) {
        return None;
    }
    let normalized = normalize_affirmation_text(ask_text);
    let now = now_ts().parse::<i64>().unwrap_or_default();
    let db = state.db.lock().ok()?;
    let mut stmt = db
        .prepare(
            "SELECT task_id, payload_json, status, CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) AS ts
             FROM tasks
             WHERE user_id = ?1 AND chat_id = ?2 AND kind = 'ask'
             ORDER BY ts DESC
             LIMIT ?3",
        )
        .ok()?;
    let rows = stmt
        .query_map(
            params![
                user_id,
                chat_id,
                rules.duplicate_affirmation_scan_limit as i64
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .ok()?;
    for row in rows.flatten() {
        let (task_id, payload_json, status, ts) = row;
        let status_lc = status.to_ascii_lowercase();
        if !rules
            .duplicate_affirmation_statuses
            .iter()
            .any(|s| s == &status_lc)
        {
            continue;
        }
        if now.saturating_sub(ts) > window_secs {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<Value>(&payload_json) else {
            continue;
        };
        let text = payload
            .get("text")
            .and_then(|v| v.as_str())
            .map(normalize_affirmation_text)
            .unwrap_or_default();
        if text == normalized {
            if let Ok(id) = Uuid::parse_str(&task_id) {
                return Some(id);
            }
        }
    }
    None
}

pub(crate) fn find_recent_failed_resume_context(
    state: &AppState,
    user_id: i64,
    chat_id: i64,
) -> Option<RecentFailedResumeContext> {
    let db = state.db.lock().ok()?;
    let mut stmt = db
        .prepare(
            "SELECT result_json,
                    CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER)
             FROM tasks
             WHERE user_id = ?1 AND chat_id = ?2 AND kind = 'ask' AND status = 'failed'
             ORDER BY CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) DESC
             LIMIT 24",
        )
        .ok()?;
    let rows = stmt
        .query_map(params![user_id, chat_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<i64>>(1)?.unwrap_or_default(),
            ))
        })
        .ok()?;
    for row in rows.flatten() {
        let (result_json, ts) = row;
        let Some(result_json) = result_json else {
            continue;
        };
        let Ok(result) = serde_json::from_str::<Value>(&result_json) else {
            continue;
        };
        let Some(resume_context) = result.get("resume_context").cloned() else {
            continue;
        };
        if !resume_context.is_null() {
            let has_newer_successful_ask_after_failed_task = db
                .query_row(
                    "SELECT 1
                     FROM tasks
                     WHERE user_id = ?1
                       AND chat_id = ?2
                       AND kind = 'ask'
                       AND status = 'succeeded'
                       AND CAST(COALESCE(NULLIF(updated_at, ''), created_at) AS INTEGER) > ?3
                     LIMIT 1",
                    params![user_id, chat_id, ts],
                    |_row| Ok(()),
                )
                .optional()
                .ok()
                .flatten()
                .is_some();
            return Some(RecentFailedResumeContext {
                resume_context,
                failed_ts: ts,
                has_newer_successful_ask_after_failed_task,
            });
        }
    }
    None
}

pub(crate) fn task_kind_name(kind: &claw_core::types::TaskKind) -> &'static str {
    match kind {
        claw_core::types::TaskKind::Ask => "ask",
        claw_core::types::TaskKind::RunSkill => "run_skill",
        claw_core::types::TaskKind::Admin => "admin",
    }
}

pub(crate) fn build_submit_task_payload(
    mut payload: Value,
    channel: ChannelKind,
    normalized_external_user_id: Option<&str>,
    normalized_external_chat_id: Option<&str>,
    effective_user_key: Option<&str>,
    effective_agent_id: &str,
    call_id: &str,
) -> Value {
    if let Some(obj) = payload.as_object_mut() {
        let channel_str = channel_kind_name(channel);
        obj.insert(
            "channel".to_string(),
            Value::String(channel_str.to_string()),
        );
        if let Some(v) = normalized_external_user_id {
            obj.insert("external_user_id".to_string(), Value::String(v.to_string()));
        }
        if let Some(v) = normalized_external_chat_id {
            obj.insert("external_chat_id".to_string(), Value::String(v.to_string()));
        }
        if let Some(user_key) = effective_user_key {
            obj.insert("user_key".to_string(), Value::String(user_key.to_string()));
        }
        obj.insert(
            "agent_id".to_string(),
            Value::String(effective_agent_id.to_string()),
        );
        obj.insert("call_id".to_string(), Value::String(call_id.to_string()));
    }
    payload
}

pub(crate) fn insert_submitted_task(
    state: &AppState,
    task_id: &Uuid,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    channel: ChannelKind,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
    kind: &str,
    payload_text: &str,
) -> anyhow::Result<()> {
    let now = now_ts();
    let db = state
        .db
        .lock()
        .map_err(|_| anyhow::anyhow!("db lock poisoned"))?;

    db.execute(
        "INSERT INTO tasks (task_id, user_id, chat_id, user_key, channel, external_user_id, external_chat_id, message_id, kind, payload_json, status, result_json, error_text, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, 'queued', NULL, NULL, ?10, ?10)",
        params![
            task_id.to_string(),
            user_id,
            chat_id,
            user_key,
            channel_kind_name(channel),
            external_user_id,
            external_chat_id,
            kind,
            payload_text,
            now
        ],
    )?;
    Ok(())
}
