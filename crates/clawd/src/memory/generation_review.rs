//! Reuse a summary only after a model reviewed this exact eligible turn.
//! Pairing is optional: unavailable, changed, or failed evidence falls back to
//! the existing consolidation path. Source rows are never removed or merged.
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::job_completion::{GenerationLease, JobPhase};
#[path = "generation_review_source.rs"]
mod source;

const MATCHING_PAIR: &str = "FROM memory_jobs c JOIN memory_jobs e
 ON e.principal_id=c.principal_id AND e.source_task_id=c.source_task_id
 AND e.scope_kind=c.scope_kind AND e.scope_ref=c.scope_ref
 AND e.source_event_start=c.source_event_start AND e.source_event_end=c.source_event_end
 AND e.source_digest=c.source_digest AND e.eligibility_json=c.eligibility_json
 AND e.settings_revision=c.settings_revision AND e.policy_digest=c.policy_digest
 AND e.provider_name=c.provider_name AND e.provider_type=c.provider_type
 AND e.model_name=c.model_name AND e.model_capability_digest=c.model_capability_digest
 WHERE c.job_kind='consolidate' AND e.job_kind='extract'";

fn digest(value: &str) -> String {
    format!("sha256:{:x}", Sha256::digest(value.as_bytes()))
}

pub(super) fn pair_turn_jobs(db: &Connection, task_id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_review_pair_requires_transaction"
    );
    let sql = format!(
        "SELECT c.job_id,e.job_id {MATCHING_PAIR}
        AND c.source_task_id=?1 AND c.status='queued' AND e.status='queued'
        AND c.cancel_requested=0 AND e.cancel_requested=0
        AND c.checkpoint_json='{{}}' AND e.checkpoint_json='{{}}'"
    );
    let pairs = db
        .prepare(&sql)?
        .query_map([task_id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    for (consolidate, extract) in pairs {
        for (job, peer_field, peer) in [
            (&consolidate, "extract_job_id", &extract),
            (&extract, "consolidate_job_id", &consolidate),
        ] {
            let mut review = json!({"schema_version":1});
            review[peer_field] = json!(peer);
            db.execute(
                "UPDATE memory_jobs SET checkpoint_json=?2 WHERE job_id=?1",
                params![job, json!({"generation_review":review}).to_string()],
            )?;
        }
    }
    Ok(())
}

struct PairedExtraction {
    job_id: String,
    status: String,
    cancelled: bool,
    checkpoint: Value,
    source_digest: String,
}

fn paired_extraction(
    db: &Connection,
    consolidate: &str,
) -> anyhow::Result<Option<PairedExtraction>> {
    let checkpoint: Option<String> = db
        .query_row(
            "SELECT checkpoint_json FROM memory_jobs WHERE job_id=?1",
            [consolidate],
            |r| r.get(0),
        )
        .optional()?;
    let Some(checkpoint) = checkpoint.and_then(|s| serde_json::from_str::<Value>(&s).ok()) else {
        return Ok(None);
    };
    let review = &checkpoint["generation_review"];
    let Some(extract) = review["extract_job_id"]
        .as_str()
        .filter(|_| review["schema_version"] == 1)
    else {
        return Ok(None);
    };
    let sql = format!(
        "SELECT e.job_id,e.status,e.cancel_requested,e.checkpoint_json,e.source_digest
        {MATCHING_PAIR} AND c.job_id=?1 AND e.job_id=?2"
    );
    let pair = db
        .query_row(&sql, params![consolidate, extract], |r| {
            let raw: String = r.get(3)?;
            Ok(PairedExtraction {
                job_id: r.get(0)?,
                status: r.get(1)?,
                cancelled: r.get(2)?,
                checkpoint: serde_json::from_str(&raw).unwrap_or(Value::Null),
                source_digest: r.get(4)?,
            })
        })
        .optional()?;
    Ok(pair.filter(|pair| {
        pair.checkpoint["generation_review"]["consolidate_job_id"] == consolidate
            && pair.checkpoint["generation_review"]["schema_version"] == 1
    }))
}

pub(super) fn waiting_for_extraction(db: &Connection, consolidate: &str) -> anyhow::Result<bool> {
    Ok(paired_extraction(db, consolidate)?.is_some_and(|pair| {
        !pair.cancelled && matches!(pair.status.as_str(), "queued" | "running" | "retry_wait")
    }))
}

fn summary(db: &Connection, principal: &str) -> anyhow::Result<String> {
    Ok(db
        .query_row(
            "SELECT summary FROM long_term_memories
        WHERE principal_id=?1 AND scope_kind='principal' AND scope_ref=?1
        ORDER BY COALESCE(updated_at_ts,created_at_ts,0) DESC,id DESC LIMIT 1",
            [principal],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or_default())
}

pub(super) struct ReviewContext {
    summary_digest: String,
    source_digest: String,
    pub(super) prompt: String,
    pub(super) prompt_source: String,
}

pub(super) fn prepare(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    generation: &super::generation_response::GenerationCall<'_>,
) -> anyhow::Result<Option<ReviewContext>> {
    let db = state.core.db.get()?;
    let checkpoint: String = db.query_row(
        "SELECT checkpoint_json FROM memory_jobs WHERE job_id=?1",
        [generation.lease.job_id],
        |r| r.get(0),
    )?;
    let Ok(checkpoint) = serde_json::from_str::<Value>(&checkpoint) else {
        return Ok(None);
    };
    let Some(consolidate) = checkpoint["generation_review"]["consolidate_job_id"].as_str() else {
        return Ok(None);
    };
    let Some(pair) = paired_extraction(&db, consolidate)? else {
        return Ok(None);
    };
    if pair.job_id != generation.lease.job_id {
        return Ok(None);
    }
    let active: bool = db.query_row(
        "SELECT status='queued' AND cancel_requested=0 FROM memory_jobs WHERE job_id=?1",
        [consolidate],
        |r| r.get(0),
    )?;
    if !active
        && super::job_completion::phase(&checkpoint.to_string()) != JobPhase::ResponsePersisted
    {
        return Ok(None);
    }
    let Some(rows) = source::verified_turn(&db, task, generation.lease.job_id)? else {
        return Ok(None);
    };
    // Never decide from an omitted, redacted or truncated part of the turn.
    if rows
        .iter()
        .any(|s| s.chars().count() > state.policy.memory.llm_preference_max_chars.max(128))
    {
        return Ok(None);
    }
    let previous = summary(&db, generation.lease.principal_id)?;
    drop(db);
    // The model decides whether this turn contains durable information. Do
    // not interpolate the mutable summary: retries must reconstruct the exact
    // original provider prompt and reuse its durably staged response.
    let context = json!({"assistant_outcome":rows[1]});
    let (safe, redacted) =
        crate::skill_output_artifact::sensitivity_aware_text_model_view(&context.to_string());
    if redacted {
        return Ok(None);
    }
    let (template, prompt_source) = crate::bootstrap::load_required_prompt_template_for_state(
        state,
        "prompts/memory_generation_review.md",
    )
    .map_err(anyhow::Error::msg)?;
    Ok(Some(ReviewContext {
        summary_digest: digest(&previous),
        source_digest: pair.source_digest,
        prompt: crate::render_prompt_template(&template, &[("__SUMMARY_REVIEW_CONTEXT__", &safe)]),
        prompt_source: format!("memory_intent_extract+{prompt_source}"),
    }))
}

pub(super) fn record(
    db: &Connection,
    lease: GenerationLease<'_>,
    review: &ReviewContext,
    needed: bool,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !db.is_autocommit(),
        "memory_review_commit_requires_transaction"
    );
    let changed=db.execute("UPDATE memory_jobs SET checkpoint_json=json_patch(checkpoint_json,?1)
        WHERE job_id=?2 AND principal_id=?3 AND status='running' AND lease_owner=?4 AND cancel_requested=0",
        params![json!({"generation_review":{"schema_version":1,"summary_update_needed":needed,
            "summary_digest":review.summary_digest,"source_digest":review.source_digest}}).to_string(),
            lease.job_id,lease.principal_id,lease.worker_id])?;
    anyhow::ensure!(changed == 1, "memory_review_commit_lease_lost");
    Ok(())
}

pub(super) fn try_reuse_summary(
    state: &crate::AppState,
    task: &crate::ClaimedTask,
    job: &super::jobs::MemoryJobSnapshot,
    lease: GenerationLease<'_>,
) -> anyhow::Result<bool> {
    let mut db = state.core.db.get()?;
    let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let Some(pair) = paired_extraction(&tx, &job.job_id)? else {
        return Ok(false);
    };
    if source::verified_turn(&tx, task, &job.job_id)?.is_none() {
        return Ok(false);
    }
    let review = &pair.checkpoint["generation_review"];
    if pair.status != "completed"
        || pair.cancelled
        || super::job_completion::phase(&pair.checkpoint.to_string()) != JobPhase::Committed
        || review["summary_update_needed"] != false
        || review["source_digest"] != job.source_digest
    {
        return Ok(false);
    }
    if review["summary_digest"] != digest(&summary(&tx, &job.principal_id)?) {
        return Ok(false);
    }
    let changed=tx.execute("UPDATE memory_jobs SET checkpoint_json=json_patch(checkpoint_json,?1)
        WHERE job_id=?2 AND principal_id=?3 AND status='running' AND lease_owner=?4 AND cancel_requested=0",
        params![json!({"generation_review":{"summary_reused":true}}).to_string(),
            lease.job_id,lease.principal_id,lease.worker_id])?;
    anyhow::ensure!(changed == 1, "memory_review_reuse_lease_lost");
    super::job_completion::mark_committed(&tx, lease)?;
    tx.commit()?;
    Ok(true)
}

#[cfg(test)]
#[path = "generation_review_tests.rs"]
mod tests;
