use serde_json::{json, Map, Value};

use crate::{repo, AppState, ClaimedTask};

pub(super) enum DirectRunSkillMutationGuard {
    NotRequired,
    Acquired(repo::TaskMutationLease),
    ReplaySuppressed(repo::TaskMutationRecord),
    ReconciliationRequired(repo::TaskMutationRecord),
}

impl DirectRunSkillMutationGuard {
    pub(super) fn execution_context(&self) -> Option<crate::skills::SkillExecutionContext> {
        let Self::Acquired(lease) = self else {
            return None;
        };
        Some(crate::skills::SkillExecutionContext {
            action_ref: lease.record.action_ref.clone(),
            idempotency_key: lease.record.idempotency_key.clone(),
            attempt_no: lease.record.attempt_no,
        })
    }
}

pub(super) fn prepare_direct_run_skill_mutation(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: &Value,
) -> Result<DirectRunSkillMutationGuard, String> {
    let canonical_skill = state.resolve_canonical_skill_name(skill_name);
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, &canonical_skill, args);
    if !effect.mutates || registry_action_is_idempotent(state, &canonical_skill, args) {
        return Ok(DirectRunSkillMutationGuard::NotRequired);
    }
    let action_ref = direct_run_skill_action_ref(&canonical_skill, args);
    let fingerprint = format!(
        "direct_run_skill:{}:{}",
        canonical_skill,
        canonical_json_string(args)
    );
    match repo::begin_task_mutation(
        &state.core.db,
        &state.worker.worker_id,
        task.claim_attempt,
        &task.task_id,
        &fingerprint,
        &action_ref,
    )
    .map_err(|error| error.to_string())?
    {
        repo::BeginTaskMutationOutcome::Acquired(mut lease) => {
            repo::start_task_mutation_attempt(&state.core.db, &mut lease)
                .map_err(|error| error.to_string())?;
            Ok(DirectRunSkillMutationGuard::Acquired(lease))
        }
        repo::BeginTaskMutationOutcome::ReplaySuppressed(record) => {
            Ok(DirectRunSkillMutationGuard::ReplaySuppressed(record))
        }
        repo::BeginTaskMutationOutcome::ReconciliationRequired(record) => {
            reconcile_direct_run_skill_mutation(state, task, &fingerprint, &action_ref, record)
        }
    }
}

fn reconcile_direct_run_skill_mutation(
    state: &AppState,
    task: &ClaimedTask,
    fingerprint: &str,
    action_ref: &str,
    record: repo::TaskMutationRecord,
) -> Result<DirectRunSkillMutationGuard, String> {
    let Some((resolution, projection)) =
        crate::agent_engine::load_task_mutation_reconciliation_directive(
            state,
            task,
            &record.fingerprint_hash,
        )?
    else {
        return Ok(DirectRunSkillMutationGuard::ReconciliationRequired(record));
    };
    match repo::reconcile_task_mutation(
        &state.core.db,
        &state.worker.worker_id,
        task.claim_attempt,
        &task.task_id,
        &record.fingerprint_hash,
        resolution,
        &projection,
    )
    .map_err(|error| error.to_string())?
    {
        repo::ReconcileTaskMutationOutcome::RetryReady(mut lease) => {
            repo::start_task_mutation_attempt(&state.core.db, &mut lease)
                .map_err(|error| error.to_string())?;
            Ok(DirectRunSkillMutationGuard::Acquired(lease))
        }
        repo::ReconcileTaskMutationOutcome::Reconciled(lease) => {
            repo::commit_task_mutation(&state.core.db, &lease)
                .map_err(|error| error.to_string())?;
            match repo::begin_task_mutation(
                &state.core.db,
                &state.worker.worker_id,
                task.claim_attempt,
                &task.task_id,
                fingerprint,
                action_ref,
            )
            .map_err(|error| error.to_string())?
            {
                repo::BeginTaskMutationOutcome::ReplaySuppressed(record) => {
                    Ok(DirectRunSkillMutationGuard::ReplaySuppressed(record))
                }
                _ => Err("direct_run_skill_reconciliation_commit_not_observable".to_string()),
            }
        }
        repo::ReconcileTaskMutationOutcome::ReplaySuppressed(record) => {
            Ok(DirectRunSkillMutationGuard::ReplaySuppressed(record))
        }
        repo::ReconcileTaskMutationOutcome::Waiting(record) => {
            Ok(DirectRunSkillMutationGuard::ReconciliationRequired(record))
        }
    }
}

pub(super) fn replay_suppressed_run_skill_outcome(
    record: &repo::TaskMutationRecord,
) -> crate::skills::SkillRunOutcome {
    let text = json!({
        "schema_version": 1,
        "source": "task_mutation_ledger",
        "status": record.phase.as_token(),
        "execution": "suppressed",
        "reason_code": "mutation_replay_suppressed",
        "action_ref": record.action_ref,
        "fingerprint_hash": record.fingerprint_hash,
        "idempotency_key": record.idempotency_key,
        "attempt_no": record.attempt_no,
    })
    .to_string();
    let extra = record
        .receipt
        .as_ref()
        .and_then(|value| value.get("structured_extra"))
        .cloned();
    crate::skills::SkillRunOutcome {
        text,
        notify: None,
        validation: record.verification.clone(),
        extra,
    }
}

pub(super) fn persist_direct_run_skill_mutation_result(
    state: &AppState,
    guard: &DirectRunSkillMutationGuard,
    result: &Result<crate::skills::SkillRunOutcome, String>,
) -> bool {
    let DirectRunSkillMutationGuard::Acquired(lease) = guard else {
        return true;
    };
    let Ok(outcome) = result else {
        let _ = repo::mark_task_mutation_uncertain(&state.core.db, lease);
        return false;
    };
    let projection = crate::agent_engine::safe_mutation_outcome_projection(outcome.extra.as_ref());
    if repo::record_task_mutation_receipt(&state.core.db, lease, &outcome.text, projection.as_ref())
        .is_err()
    {
        return false;
    }
    let verification = json!({
        "schema_version": 1,
        "status": "not_required",
        "required": false,
    });
    if repo::record_task_mutation_verification(&state.core.db, lease, &verification, true).is_err()
    {
        return false;
    }
    repo::commit_task_mutation(&state.core.db, lease).is_ok()
}

pub(super) fn finalize_direct_run_skill_reconciliation(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    action_ref: &str,
    fingerprint_hash: &str,
) -> anyhow::Result<()> {
    let checkpoint_id = format!("run-skill:{}:mutation-reconciliation", task.task_id);
    let budget = crate::task_lifecycle::CheckpointBudgetCounters {
        round: 0,
        step: 0,
        llm_calls: u32::try_from(state.task_llm_call_count(&task.task_id)).unwrap_or(u32::MAX),
        tool_calls: 1,
        elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
        llm_elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
        tool_elapsed_ms: 0,
    };
    let checkpoint = crate::task_lifecycle::TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context: json!({
            "schema_version": 1,
            "source": "direct_run_skill_mutation_ledger",
            "task_id": task.task_id,
            "skill": skill_name,
            "reason_code": "mutation_outcome_unknown",
            "message_key": "clawd.task.mutation_outcome_unknown",
            "action_ref": action_ref,
            "fingerprint_hash": fingerprint_hash,
            "requires_reconciliation": true,
        }),
        last_successful_round: None,
        last_successful_step: None,
        pending_action: Some(json!({
            "schema_version": 1,
            "kind": "mutation_reconciliation",
            "action_ref": action_ref,
            "fingerprint_hash": fingerprint_hash,
            "resume_expected": "verified_effect_outcome",
        })),
        observations: Vec::new(),
        capability_results: Vec::new(),
        evidence_refs: Vec::new(),
        artifact_refs: Vec::new(),
        completed_side_effect_refs: Vec::new(),
        budget: budget.clone(),
        attempt_ledger: None,
        pending_async_job: None,
        repair_signal: None,
        resume_entrypoint: crate::task_lifecycle::ResumeEntrypoint::AwaitUserInput,
    };
    let lifecycle = json!({
        "schema_version": 1,
        "state": crate::task_lifecycle::TaskLifecycleState::NeedsUser,
        "source": "direct_run_skill_mutation_ledger",
        "resume_reason": "mutation_reconciliation_required",
        "checkpoint_id": checkpoint_id,
        "can_poll": false,
        "can_cancel": true,
        "last_heartbeat_ts": crate::now_ts_u64() as i64,
        "message_key": "clawd.task.mutation_outcome_unknown",
        "tool_or_skill": skill_name,
        "action_ref": action_ref,
        "fingerprint_hash": fingerprint_hash,
        "requires_reconciliation": true,
        "budget": serde_json::to_value(&budget).unwrap_or_else(|_| json!({})),
    });
    let result = json!({
        "task_lifecycle": lifecycle,
        "task_checkpoint": checkpoint.to_machine_json(),
    });
    repo::update_task_checkpointed_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &result.to_string(),
    )?;
    state.clear_task_llm_call_count(&task.task_id);
    Ok(())
}

fn registry_action_is_idempotent(state: &AppState, skill_name: &str, args: &Value) -> bool {
    let action = normalized_action_token(args);
    state
        .get_skills_registry()
        .is_some_and(|registry| registry.resolved_idempotent(skill_name, action.as_deref()))
}

fn direct_run_skill_action_ref(skill_name: &str, args: &Value) -> String {
    format!(
        "skill:{}:action:{}",
        skill_name,
        normalized_action_token(args).unwrap_or_else(|| "_default".to_string())
    )
}

fn normalized_action_token(args: &Value) -> Option<String> {
    args.get("action")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            value
                .to_ascii_lowercase()
                .chars()
                .map(|ch| {
                    if matches!(ch, '-' | ' ' | '.') {
                        '_'
                    } else {
                        ch
                    }
                })
                .collect()
        })
}

fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(&canonicalize_json_value(value)).unwrap_or_else(|_| value.to_string())
}

fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().collect::<Vec<_>>();
            keys.sort();
            let mut canonical = Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonicalize_json_value(&map[key]));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonicalize_json_value).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
#[path = "run_skill_mutation_tests.rs"]
mod tests;
