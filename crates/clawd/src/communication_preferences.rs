use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use claw_core::types::{ApiResponse, AuthIdentity, ChannelKind};
use tracing::error;

use crate::{build_conversation_chat_id, now_ts, now_ts_u64, stable_i64_from_key, AppState};

pub(crate) const LOCALE_KEY: &str = "communication.locale";
pub(crate) const VOICE_REPLY_MODE_KEY: &str = "communication.voice_reply_mode";
const PREFERENCE_SOURCE: &str = "channel_preference_api";

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CommunicationPreferenceScopeRequest {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    channel: Option<ChannelKind>,
    #[serde(default)]
    external_user_id: Option<String>,
    #[serde(default)]
    external_chat_id: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateCommunicationPreferencesRequest {
    #[serde(flatten)]
    scope: CommunicationPreferenceScopeRequest,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    voice_reply_mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommunicationPreferences {
    pub(crate) schema_version: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) voice_reply_mode: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedLocale {
    pub(crate) locale: String,
    pub(crate) source: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct LegacyVoicePreferenceMigrationReport {
    pub(crate) discovered: usize,
    pub(crate) migrated: usize,
    pub(crate) already_current: usize,
    pub(crate) binding_missing: usize,
    pub(crate) invalid: usize,
}

pub(crate) fn normalize_locale(value: &str) -> Option<String> {
    let value = value.trim().replace('_', "-");
    if !(2..=35).contains(&value.len())
        || value.starts_with('-')
        || value.ends_with('-')
        || value.split('-').any(|part| {
            part.is_empty()
                || part.len() > 8
                || !part.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
    {
        return None;
    }
    let mut parts = value.split('-');
    let language = parts.next()?.to_ascii_lowercase();
    let suffix = parts
        .map(|part| {
            if part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_alphabetic()) {
                part.to_ascii_uppercase()
            } else {
                part.to_string()
            }
        })
        .collect::<Vec<_>>();
    Some(if suffix.is_empty() {
        language
    } else {
        format!("{language}-{}", suffix.join("-"))
    })
}

pub(crate) fn normalize_voice_reply_mode(value: &str) -> Option<String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "text" => Some("text".to_string()),
        "voice" => Some("voice".to_string()),
        "auto" => Some("auto".to_string()),
        _ => None,
    }
}

pub(crate) fn load(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
) -> anyhow::Result<CommunicationPreferences> {
    let mut result = CommunicationPreferences {
        schema_version: 1,
        ..CommunicationPreferences::default()
    };
    let mut stmt = db.prepare(
        "SELECT pref_key, pref_value
         FROM user_preferences
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3
           AND pref_key IN (?4, ?5)
         ORDER BY updated_at_ts ASC, id ASC",
    )?;
    let rows = stmt.query_map(
        params![user_id, chat_id, user_key, LOCALE_KEY, VOICE_REPLY_MODE_KEY],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;
    for row in rows {
        let (key, value) = row?;
        match key.as_str() {
            LOCALE_KEY => result.locale = normalize_locale(&value),
            VOICE_REPLY_MODE_KEY => result.voice_reply_mode = normalize_voice_reply_mode(&value),
            _ => {}
        }
    }
    Ok(result)
}

pub(crate) fn update(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    locale: Option<&str>,
    voice_reply_mode: Option<&str>,
) -> anyhow::Result<CommunicationPreferences> {
    let now = now_ts();
    let now_unix = now_ts_u64() as i64;
    if let Some(locale) = locale {
        let locale = normalize_locale(locale)
            .ok_or_else(|| anyhow::anyhow!("communication_locale_invalid"))?;
        upsert(
            db, user_id, chat_id, user_key, LOCALE_KEY, &locale, &now, now_unix,
        )?;
    }
    if let Some(mode) = voice_reply_mode {
        let mode = normalize_voice_reply_mode(mode)
            .ok_or_else(|| anyhow::anyhow!("communication_voice_reply_mode_invalid"))?;
        upsert(
            db,
            user_id,
            chat_id,
            user_key,
            VOICE_REPLY_MODE_KEY,
            &mode,
            &now,
            now_unix,
        )?;
    }
    load(db, user_id, chat_id, user_key)
}

fn upsert(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    key: &str,
    value: &str,
    now: &str,
    now_unix: i64,
) -> anyhow::Result<()> {
    db.execute(
        "INSERT INTO user_preferences
         (user_id, chat_id, user_key, pref_key, pref_value, confidence, source, updated_at, updated_at_ts)
         VALUES (?1, ?2, ?3, ?4, ?5, 1.0, ?6, ?7, ?8)
         ON CONFLICT(user_id, chat_id, user_key, pref_key)
         DO UPDATE SET pref_value=excluded.pref_value, confidence=excluded.confidence,
                       source=excluded.source, updated_at=excluded.updated_at,
                       updated_at_ts=excluded.updated_at_ts",
        params![
            user_id,
            chat_id,
            user_key,
            key,
            value,
            PREFERENCE_SOURCE,
            now,
            now_unix
        ],
    )?;
    Ok(())
}

pub(crate) fn resolve_locale(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
    platform_locale: Option<&str>,
    runtime_default_locale: &str,
) -> anyhow::Result<ResolvedLocale> {
    if let Some(user_key) = user_key {
        if let Some(locale) = load(db, user_id, chat_id, user_key)?.locale {
            return Ok(ResolvedLocale {
                locale,
                source: "conversation_preference",
            });
        }
        if let Some(locale) = load(db, user_id, 0, user_key)?.locale {
            return Ok(ResolvedLocale {
                locale,
                source: "user_preference",
            });
        }
    }
    if let Some(locale) = platform_locale.and_then(normalize_locale) {
        return Ok(ResolvedLocale {
            locale,
            source: "platform",
        });
    }
    if let Some(locale) = latest_conversation_locale(db, user_id, chat_id, user_key)? {
        return Ok(ResolvedLocale {
            locale,
            source: "conversation",
        });
    }
    if let Some(locale) = normalize_locale(runtime_default_locale) {
        return Ok(ResolvedLocale {
            locale,
            source: "runtime_default",
        });
    }
    Ok(ResolvedLocale {
        locale: "en-US".to_string(),
        source: "safe_default",
    })
}

fn latest_conversation_locale(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let payload = if let Some(user_key) = user_key {
        db.query_row(
            "SELECT payload_json FROM tasks
             WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![user_id, chat_id, user_key],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    } else {
        db.query_row(
            "SELECT payload_json FROM tasks
             WHERE user_id = ?1 AND chat_id = ?2
             ORDER BY created_at DESC, rowid DESC LIMIT 1",
            params![user_id, chat_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
    };
    Ok(payload
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|value| {
            value
                .get("channel_ingress")
                .and_then(|ingress| ingress.get("locale"))
                .and_then(serde_json::Value::as_str)
                .and_then(normalize_locale)
        }))
}

pub(crate) fn migrate_legacy_telegram_voice_preferences(
    db: &Connection,
    legacy: &std::collections::HashMap<String, String>,
) -> anyhow::Result<LegacyVoicePreferenceMigrationReport> {
    let mut report = LegacyVoicePreferenceMigrationReport {
        discovered: legacy.len(),
        ..LegacyVoicePreferenceMigrationReport::default()
    };
    for (external_chat_id, mode) in legacy {
        let Some(mode) = normalize_voice_reply_mode(mode) else {
            report.invalid += 1;
            continue;
        };
        let binding = db
            .query_row(
                "SELECT user_key FROM channel_bindings
                 WHERE channel = 'telegram' AND external_chat_id = ?1
                 ORDER BY id DESC LIMIT 1",
                params![external_chat_id.trim()],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let Some(user_key) = binding else {
            report.binding_missing += 1;
            continue;
        };
        let user_id = stable_i64_from_key(&user_key);
        let chat_id = build_conversation_chat_id(
            "telegram",
            Some(external_chat_id),
            Some(external_chat_id),
            &user_key,
        );
        if load(db, user_id, chat_id, &user_key)?
            .voice_reply_mode
            .as_deref()
            == Some(mode.as_str())
        {
            report.already_current += 1;
            continue;
        }
        update(db, user_id, chat_id, &user_key, None, Some(&mode))?;
        report.migrated += 1;
    }
    Ok(report)
}

fn preference_chat_id(
    state: &AppState,
    identity: &AuthIdentity,
    scope: &CommunicationPreferenceScopeRequest,
) -> Result<i64, &'static str> {
    match scope.scope.as_deref().unwrap_or("conversation") {
        "user" => Ok(0),
        "conversation" => {
            let Some(channel) = scope.channel else {
                return Ok(identity.chat_id);
            };
            let channel_name = super::channel_kind_label(channel);
            let resolved = super::resolve_channel_binding_identity(
                state,
                channel_name,
                scope.external_user_id.as_deref(),
                scope.external_chat_id.as_deref(),
            )
            .map_err(|_| "communication_preference_binding_lookup_failed")?
            .ok_or("communication_preference_binding_required")?;
            if resolved.user_key != identity.user_key {
                return Err("communication_preference_scope_forbidden");
            }
            Ok(build_conversation_chat_id(
                channel_name,
                scope.external_user_id.as_deref(),
                scope.external_chat_id.as_deref(),
                &identity.user_key,
            ))
        }
        _ => Err("communication_preference_scope_invalid"),
    }
}

pub(crate) async fn get_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(scope): Query<CommunicationPreferenceScopeRequest>,
) -> (StatusCode, Json<ApiResponse<CommunicationPreferences>>) {
    let identity =
        match super::require_auth_identity_for_api::<CommunicationPreferences>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    let chat_id = match preference_chat_id(&state, &identity, &scope) {
        Ok(chat_id) => chat_id,
        Err(error) => return super::api_err(StatusCode::BAD_REQUEST, error),
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(error) => {
            error!(error = %error, "communication_preference_database_checkout_failed");
            return super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "communication_preference_database_unavailable",
            );
        }
    };
    match load(&db, identity.user_id, chat_id, &identity.user_key) {
        Ok(preferences) => super::api_ok(preferences),
        Err(error) => {
            error!(error = %error, "communication_preference_lookup_failed");
            super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "communication_preference_lookup_failed",
            )
        }
    }
}

pub(crate) async fn update_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateCommunicationPreferencesRequest>,
) -> (StatusCode, Json<ApiResponse<CommunicationPreferences>>) {
    let identity =
        match super::require_auth_identity_for_api::<CommunicationPreferences>(&state, &headers) {
            Ok(identity) => identity,
            Err(response) => return response,
        };
    if request.locale.is_none() && request.voice_reply_mode.is_none() {
        return super::api_err(
            StatusCode::BAD_REQUEST,
            "communication_preference_update_empty",
        );
    }
    let chat_id = match preference_chat_id(&state, &identity, &request.scope) {
        Ok(chat_id) => chat_id,
        Err(error) => return super::api_err(StatusCode::BAD_REQUEST, error),
    };
    let db = match state.core.db.get() {
        Ok(db) => db,
        Err(error) => {
            error!(error = %error, "communication_preference_database_checkout_failed");
            return super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "communication_preference_database_unavailable",
            );
        }
    };
    match update(
        &db,
        identity.user_id,
        chat_id,
        &identity.user_key,
        request.locale.as_deref(),
        request.voice_reply_mode.as_deref(),
    ) {
        Ok(preferences) => super::api_ok(preferences),
        Err(error) if error.to_string().ends_with("_invalid") => {
            super::api_err(StatusCode::BAD_REQUEST, error.to_string())
        }
        Err(error) => {
            error!(error = %error, "communication_preference_update_failed");
            super::api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "communication_preference_update_failed",
            )
        }
    }
}

#[cfg(test)]
#[path = "communication_preferences_tests.rs"]
mod tests;
