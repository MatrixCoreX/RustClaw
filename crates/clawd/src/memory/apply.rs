use anyhow::anyhow;
use rusqlite::{params, Connection};

use super::intent::{MemoryAction, MemoryActionKind, MemoryActionOp, MemoryScope, MemoryTtlPolicy};
use super::RETRIEVAL_SOURCE_PREFERENCE;
use crate::AppState;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct MemoryActionApplyStats {
    pub(crate) upserted_preferences: usize,
    pub(crate) deleted_preferences: usize,
    pub(crate) ignored_actions: usize,
}

pub(crate) fn apply_memory_actions(
    state: &AppState,
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    actions: &[MemoryAction],
    now_text: &str,
    now_ts_i64: i64,
) -> anyhow::Result<MemoryActionApplyStats> {
    let mut stats = MemoryActionApplyStats::default();
    for action in actions {
        match action.action {
            MemoryActionOp::Noop => {
                stats.ignored_actions += 1;
            }
            MemoryActionOp::Upsert => {
                let Some(pref) = preference_upsert_from_action(action) else {
                    stats.ignored_actions += 1;
                    continue;
                };
                super::upsert_user_preferences(
                    state,
                    db,
                    user_id,
                    chat_id,
                    user_key,
                    std::slice::from_ref(&pref),
                    now_text,
                    now_ts_i64,
                )?;
                stats.upserted_preferences += 1;
            }
            MemoryActionOp::Delete | MemoryActionOp::Expire => {
                let Some(pref_key) = preference_delete_key_from_action(action) else {
                    stats.ignored_actions += 1;
                    continue;
                };
                let deleted = delete_preference(db, user_id, chat_id, user_key, &pref_key, state)?;
                stats.deleted_preferences += deleted;
            }
        }
    }
    Ok(stats)
}

fn preference_upsert_from_action(action: &MemoryAction) -> Option<(String, String, f32, String)> {
    if action.kind != MemoryActionKind::Preference {
        return None;
    }
    if !matches!(action.scope, MemoryScope::Chat | MemoryScope::User) {
        return None;
    }
    if !matches!(
        action.ttl_policy,
        MemoryTtlPolicy::LongTerm | MemoryTtlPolicy::ExplicitUntil
    ) {
        return None;
    }
    let key = normalized_preference_key(&action.key)?;
    let raw_value = action
        .normalized_value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| action.value.trim());
    let value = normalized_preference_value(&key, raw_value)?;
    let source = format!(
        "memory_intent:{}:{}",
        action.source.source_kind.as_str(),
        action.scope.as_str()
    );
    Some((key, value, action.confidence.clamp(0.0, 1.0), source))
}

fn normalized_preference_key(key: &str) -> Option<String> {
    let key = key.trim();
    match key {
        "response_language" | "response_style" | "response_format" | "agent_display_name" => {
            Some(key.to_string())
        }
        _ => None,
    }
}

fn preference_delete_key_from_action(action: &MemoryAction) -> Option<String> {
    if action.kind != MemoryActionKind::Preference {
        return None;
    }
    normalized_preference_key(&action.key).or_else(|| {
        action
            .source
            .source_ref
            .as_deref()
            .and_then(normalized_preference_source_ref_key)
    })
}

fn normalized_preference_source_ref_key(source_ref: &str) -> Option<String> {
    let source_ref = source_ref.trim();
    normalized_preference_key(source_ref).or_else(|| {
        source_ref
            .strip_prefix("preference:")
            .and_then(normalized_preference_key)
    })
}

fn normalized_preference_value(key: &str, value: &str) -> Option<String> {
    match key {
        "response_language" => normalize_language_tag(value),
        "response_style" => match value.trim() {
            "concise" | "detailed" => Some(value.trim().to_string()),
            _ => None,
        },
        "response_format" => match value.trim() {
            "plain_text" | "markdown" => Some(value.trim().to_string()),
            _ => None,
        },
        "agent_display_name" => normalize_display_name(value),
        _ => None,
    }
}

fn normalize_language_tag(value: &str) -> Option<String> {
    let normalized = value
        .trim()
        .chars()
        .map(|c| if c == '_' { '-' } else { c })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>();
    if !(2..=24).contains(&normalized.len()) {
        return None;
    }
    let parts = normalized.split('-').collect::<Vec<_>>();
    if parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    let primary = parts.first()?.to_ascii_lowercase();
    if !(2..=3).contains(&primary.len()) || !primary.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    if parts.len() == 1 {
        return Some(default_language_region(&primary).unwrap_or(primary));
    }

    let mut out = Vec::with_capacity(parts.len());
    out.push(primary);
    for part in parts.iter().skip(1) {
        let normalized_part = if part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            part.to_ascii_uppercase()
        } else if part.len() == 4 && part.chars().all(|c| c.is_ascii_alphabetic()) {
            let mut chars = part.chars();
            let first = chars.next()?.to_ascii_uppercase();
            let rest = chars.as_str().to_ascii_lowercase();
            format!("{first}{rest}")
        } else if (3..=8).contains(&part.len()) && part.chars().all(|c| c.is_ascii_alphanumeric()) {
            part.to_ascii_lowercase()
        } else {
            return None;
        };
        out.push(normalized_part);
    }
    Some(out.join("-"))
}

fn default_language_region(primary: &str) -> Option<String> {
    let tag = match primary {
        "en" => "en-US",
        "zh" => "zh-CN",
        "ja" => "ja-JP",
        "ko" => "ko-KR",
        "fr" => "fr-FR",
        "de" => "de-DE",
        "es" => "es-ES",
        "it" => "it-IT",
        "pt" => "pt-BR",
        "ru" => "ru-RU",
        "ar" => "ar-SA",
        "hi" => "hi-IN",
        "id" => "id-ID",
        "th" => "th-TH",
        "vi" => "vi-VN",
        _ => return None,
    };
    Some(tag.to_string())
}

fn normalize_display_name(value: &str) -> Option<String> {
    let candidate = value
        .trim()
        .trim_matches(|c| c == '"' || c == '\'' || c == '“' || c == '”' || c == '‘' || c == '’')
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let char_count = candidate.chars().count();
    ((1..=24).contains(&char_count)).then_some(candidate)
}

fn delete_preference(
    db: &Connection,
    user_id: i64,
    chat_id: i64,
    user_key: &str,
    pref_key: &str,
    state: &AppState,
) -> anyhow::Result<usize> {
    let deleted = db.execute(
        "DELETE FROM user_preferences
         WHERE user_id = ?1 AND chat_id = ?2 AND user_key = ?3 AND pref_key = ?4",
        params![user_id, chat_id, user_key, pref_key],
    )?;
    if state.policy.memory.hybrid_recall_enabled {
        let _ = db.execute(
            "DELETE FROM memory_retrieval_index
             WHERE source_kind = ?1 AND user_id = ?2 AND chat_id = ?3
               AND COALESCE(user_key, '') = ?4 AND source_pref_key = ?5",
            params![
                RETRIEVAL_SOURCE_PREFERENCE,
                user_id,
                chat_id,
                user_key,
                pref_key
            ],
        );
        let _ = db.execute(
            "DELETE FROM memory_retrieval_index_fts
             WHERE rowid NOT IN (SELECT id FROM memory_retrieval_index)",
            [],
        );
    }
    Ok(deleted)
}

#[allow(dead_code)]
pub(crate) fn record_memory_action_audit(action: &MemoryAction) -> anyhow::Result<()> {
    if action.reason.trim().is_empty() {
        return Err(anyhow!("memory action audit requires a reason"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        normalize_language_tag, normalized_preference_key, normalized_preference_source_ref_key,
        normalized_preference_value,
    };

    #[test]
    fn memory_intent_language_tag_normalization_is_structural() {
        assert_eq!(normalize_language_tag("zh_CN"), Some("zh-CN".to_string()));
        assert_eq!(normalize_language_tag("ko-KR"), Some("ko-KR".to_string()));
        assert_eq!(normalize_language_tag("fr"), Some("fr-FR".to_string()));
        assert_eq!(normalize_language_tag("EN_us"), Some("en-US".to_string()));
        assert_eq!(normalize_language_tag("中文"), None);
        assert_eq!(normalize_language_tag("-en"), None);
    }

    #[test]
    fn memory_intent_preference_key_allowlist_is_schema_token_based() {
        assert_eq!(
            normalized_preference_key("response_language"),
            Some("response_language".to_string())
        );
        assert_eq!(normalized_preference_key("中文回复"), None);
    }

    #[test]
    fn memory_intent_preference_source_ref_key_is_structural() {
        assert_eq!(
            normalized_preference_source_ref_key("preference:response_language"),
            Some("response_language".to_string())
        );
        assert_eq!(
            normalized_preference_source_ref_key("response_format"),
            Some("response_format".to_string())
        );
        assert_eq!(
            normalized_preference_source_ref_key("language preference"),
            None
        );
    }

    #[test]
    fn memory_intent_preference_values_reject_unstructured_text() {
        assert_eq!(
            normalized_preference_value("response_format", "plain_text"),
            Some("plain_text".to_string())
        );
        assert_eq!(
            normalized_preference_value("response_format", "plain words"),
            None
        );
    }
}
