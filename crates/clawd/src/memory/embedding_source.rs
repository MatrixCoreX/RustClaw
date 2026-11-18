use rusqlite::{params, Connection, OptionalExtension};

use super::EmbeddingJob;
use crate::memory::{recall_policy, vector_store::MemoryEmbeddingProfile};

/// Recheck authority on the writer connection, not on a pre-request snapshot.
pub(super) fn current(
    db: &Connection,
    job: &EmbeddingJob,
    expected: &MemoryEmbeddingProfile,
    release_default: bool,
) -> anyhow::Result<bool> {
    let Some(mut profile) = crate::memory::vector_store::load_profile(db, &job.profile_id)? else {
        return Ok(false);
    };
    profile.generation = job.profile_generation;
    if &profile != expected {
        return Ok(false);
    }
    if profile.provider_kind == "remote_http" {
        let settings = crate::memory::settings::resolve_principal_memory_settings(
            db,
            &job.principal_id,
            release_default,
        )?;
        if settings.policy_digest != job.consent_policy_digest
            || settings.external_context_policy
                != crate::memory::settings::ExternalContextPolicy::Allow
        {
            return Ok(false);
        }
    }
    if crate::repo::auth::active_principal_credential(db, &job.principal_id)?.is_none() {
        return Ok(false);
    }
    let row = db
        .query_row(
            "SELECT search_text,source_kind,source_memory_id,source_pref_key,
                user_id,chat_id,user_key,metadata_json FROM memory_retrieval_index
         WHERE id=?1 AND principal_id=?2 AND scope_kind=?3 AND scope_ref=?4",
            params![
                job.retrieval_id,
                job.principal_id,
                job.scope_kind,
                job.scope_ref
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<i64>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()?;
    let Some((text, kind, memory, preference, user_id, chat_id, key, metadata)) = row else {
        return Ok(false);
    };
    if job.projection_version != crate::memory::vector_store::PROJECTION_VERSION
        || crate::memory::vector_store::searchable_projection_digest(&text) != job.projection_digest
    {
        return Ok(false);
    }
    let metadata: serde_json::Value = serde_json::from_str(&metadata)?;
    let source = match kind.as_str() {
        crate::memory::RETRIEVAL_SOURCE_MEMORY => memory.map(recall_policy::Source::Memory),
        crate::memory::RETRIEVAL_SOURCE_MEMORY_FACT => metadata
            .get("fact_id")
            .and_then(serde_json::Value::as_i64)
            .map(recall_policy::Source::Fact),
        crate::memory::RETRIEVAL_SOURCE_PREFERENCE => {
            preference
                .as_deref()
                .map(|key| recall_policy::Source::Preference {
                    user_id,
                    chat_id,
                    key,
                    canonical_id: metadata
                        .get("canonical_memory_id")
                        .and_then(serde_json::Value::as_str),
                })
        }
        // Other typed index producers retain their exact owned projection fence.
        _ => return Ok(true),
    };
    let Some(source) = source else {
        return Ok(false);
    };
    let Some(scope) = recall_policy::resolve(db, &job.principal_id, &key, source)? else {
        return Ok(false);
    };
    Ok(scope.kind == job.scope_kind
        && scope.reference == job.scope_ref
        && scope.matches_projection(source, &text))
}
