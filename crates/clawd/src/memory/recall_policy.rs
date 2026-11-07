use rusqlite::{params, Connection, OptionalExtension};

use super::{
    retrieval_source_ref_for_memory, retrieval_source_ref_for_memory_fact,
    retrieval_source_ref_for_preference, RETRIEVAL_SOURCE_MEMORY, RETRIEVAL_SOURCE_MEMORY_FACT,
    RETRIEVAL_SOURCE_PREFERENCE,
};

#[derive(Clone, Copy)]
pub(super) enum Source<'a> {
    Memory(i64),
    Fact(i64),
    Preference {
        user_id: i64,
        chat_id: i64,
        key: &'a str,
        canonical_id: Option<&'a str>,
    },
}

impl Source<'_> {
    fn identity(self) -> (&'static str, String) {
        match self {
            Self::Memory(id) => (RETRIEVAL_SOURCE_MEMORY, retrieval_source_ref_for_memory(id)),
            Self::Fact(id) => (
                RETRIEVAL_SOURCE_MEMORY_FACT,
                retrieval_source_ref_for_memory_fact(id),
            ),
            Self::Preference { key, .. } => (
                RETRIEVAL_SOURCE_PREFERENCE,
                retrieval_source_ref_for_preference(key),
            ),
        }
    }
}

pub(super) fn require_schema(db: &Connection) -> anyhow::Result<()> {
    super::ux::require_memory_ux_schema(db)?;
    db.prepare(
        "SELECT principal_id,memory_id,feedback_kind FROM memory_retrieval_feedback LIMIT 0",
    )?;
    Ok(())
}

pub(super) struct SourceScope {
    pub(super) kind: String,
    pub(super) reference: String,
    content: String,
}

impl SourceScope {
    pub(super) fn matches_projection(&self, source: Source<'_>, text: &str) -> bool {
        match source {
            Source::Preference { key, .. } => text == format!("Preference {key}: {}", self.content),
            Source::Fact(_) => text == self.content.trim(),
            Source::Memory(_) => {
                let content = self.content.trim();
                text == content
                    .strip_prefix(super::LLM_SHORT_TERM_MEMORY_PREFIX)
                    .unwrap_or(content)
                    .trim()
            }
        }
    }
}

/// Eligibility is canonical ownership plus the durable choice, never request authentication.
pub(super) fn resolve(
    db: &Connection,
    principal: &str,
    user_key: &str,
    source: Source<'_>,
) -> anyhow::Result<Option<SourceScope>> {
    require_schema(db)?;
    let row = |row: &rusqlite::Row<'_>| {
        Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, String>(3)?,
        ))
    };
    let identity = match source {
        Source::Memory(id) => db.query_row(
            "SELECT memory_id,scope_kind,scope_ref,content FROM memories WHERE id=?1 AND principal_id=?2",
            params![id,principal],row,
        ).optional()?,
        Source::Fact(id) => db.query_row(
            "SELECT memory_id,scope_kind,scope_ref,fact_text FROM memory_facts
             WHERE id=?1 AND principal_id=?2 AND status='active'
               AND (expires_at_ts IS NULL OR expires_at_ts>?3)",
            params![id,principal,crate::now_ts_u64() as i64],row,
        ).optional()?,
        Source::Preference {user_id,chat_id,key,canonical_id} => db.query_row(
            "SELECT memory_id,scope_kind,scope_ref,pref_value FROM user_preferences WHERE principal_id=?1
             AND user_id=?3 AND chat_id=?4 AND pref_key=?5
             AND ((?6 IS NOT NULL AND memory_id=?6) OR (?6 IS NULL AND user_key=?2))",
            params![principal,user_key,user_id,chat_id,key,canonical_id],row,
        ).optional()?,
    };
    let Some((memory_id, kind, reference, content)) = identity else {
        return Ok(None);
    };
    let memory_id = memory_id
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("memory_projection_canonical_identity_unavailable"))?;
    let reference = reference
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| anyhow::anyhow!("memory_retrieval_scope_ref_required"))?;
    anyhow::ensure!(
        matches!(kind.as_str(), "principal" | "conversation" | "project"),
        "memory_projection_canonical_scope_unavailable"
    );
    let suppressed: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM memory_retrieval_feedback
         WHERE principal_id=?1 AND memory_id=?2 AND feedback_kind='do_not_use')",
        params![principal, memory_id],
        |row| row.get(0),
    )?;
    if suppressed {
        return Ok(None);
    }
    Ok(Some(SourceScope {
        kind,
        reference,
        content,
    }))
}

/// The caller owns an atomic projection boundary, including cleanup on suppression.
pub(super) fn prepare(
    db: &Connection,
    user_key: &str,
    source: Source<'_>,
    expected: &str,
) -> anyhow::Result<bool> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_retrieval_mutation_requires_transaction"
    );
    let Some(principal) = crate::repo::auth::principal_id_for_user_key(db, user_key)? else {
        return Ok(false);
    };
    let canonical = resolve(db, &principal, user_key, source)?;
    if let Some(canonical) = &canonical {
        let unchanged = if matches!(source, Source::Fact(_)) {
            canonical.content.trim() == expected.trim()
        } else {
            canonical.content == expected
        };
        anyhow::ensure!(unchanged, "memory_projection_source_changed");
    }
    let eligible = canonical.is_some();
    let (kind, reference) = source.identity();
    let (user_id, chat_id) = match source {
        Source::Preference {
            user_id, chat_id, ..
        } => (Some(user_id), Some(chat_id)),
        _ => (None, None),
    };
    let targets = "SELECT id FROM memory_retrieval_index
        WHERE principal_id=?1 AND source_kind=?2
          AND (source_ref=?3 OR source_memory_id=?7 OR source_pref_key=?8)
          AND (?4 IS NULL OR (user_id=?4 AND chat_id=?5 AND user_key=?6))";
    let (memory_id, pref_key) = match source {
        Source::Memory(id) => (Some(id), None),
        Source::Preference { key, .. } => (None, Some(key)),
        _ => (None, None),
    };
    let values =
        params![principal, kind, reference, user_id, chat_id, user_key, memory_id, pref_key];
    super::projection_retirement::remove(db, targets, values)?;
    Ok(eligible)
}
