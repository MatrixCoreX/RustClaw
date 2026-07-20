use anyhow::Result;
use serde_json::{json, Value};
use tracing::{error, info};

use crate::task_lifecycle::{
    AsyncJobRef, CheckpointBudgetCounters, ResumeEntrypoint, TaskCheckpoint, TaskLifecycleState,
};
use crate::{repo, AppState};
use claw_core::skill_registry::{OutputKind, PlannerCapabilityEffect, SkillRiskLevel};

use super::run_skill_permission::DirectRunSkillVerification;

const DIRECT_RUN_SKILL_ASYNC_SOURCE: &str = "direct_run_skill_async_start_adapter";
const DIRECT_RUN_SKILL_ASYNC_ERROR_PREFIX: &str = "direct_run_skill_async_job_invalid";

async fn finalize_run_skill_canceled(task: &crate::ClaimedTask, skill_name: &str) -> Result<()> {
    info!(
        "task_call_end task_id={} kind=run_skill status=canceled skill={}",
        task.task_id, skill_name
    );
    Ok(())
}

fn build_run_skill_step_result(
    skill_name: &str,
    status: crate::executor::StepExecutionStatus,
    output: Option<String>,
    error: Option<String>,
) -> crate::executor::StepExecutionResult {
    let ts = crate::now_ts_u64();
    crate::executor::StepExecutionResult {
        step_id: "run_skill".to_string(),
        skill: skill_name.to_string(),
        status,
        output,
        error,
        started_at: ts,
        finished_at: ts,
    }
}

fn run_skill_action_from_payload(payload: &Value) -> Option<String> {
    payload
        .get("args")
        .and_then(Value::as_object)
        .and_then(|args| args.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase().replace(['-', ' ', '.'], "_"))
}

fn risk_level_token(value: Option<SkillRiskLevel>) -> &'static str {
    match value.unwrap_or(SkillRiskLevel::Unknown) {
        SkillRiskLevel::Unknown => "unknown",
        SkillRiskLevel::Low => "low",
        SkillRiskLevel::Medium => "medium",
        SkillRiskLevel::High => "high",
    }
}

fn output_kind_token(value: OutputKind) -> &'static str {
    match value {
        OutputKind::Text => "text",
        OutputKind::File => "file",
        OutputKind::Image => "image",
        OutputKind::Mixed => "mixed",
    }
}

fn json_required_fields(schema: Option<&Value>) -> Vec<String> {
    schema
        .and_then(|schema| schema.get("required"))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn run_skill_sensitive_field_name(field: &str) -> bool {
    let normalized = field.trim().to_ascii_lowercase().replace(['-', '.'], "_");
    normalized == "key"
        || normalized == "auth"
        || normalized.ends_with("_key")
        || normalized.contains("token")
        || normalized.contains("secret")
        || normalized.contains("password")
        || normalized.contains("passwd")
        || normalized.contains("cookie")
        || normalized.contains("credential")
        || normalized.contains("ticket")
        || normalized.contains("signature")
        || normalized.contains("authorization")
}

fn run_skill_trace_safe_json(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, child) in map {
                let value = if run_skill_sensitive_field_name(key) {
                    json!("[REDACTED]")
                } else {
                    run_skill_trace_safe_json(child)
                };
                out.insert(key.clone(), value);
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(run_skill_trace_safe_json).collect()),
        Value::String(text) => Value::String(crate::visible_text::sanitize_user_visible_text(text)),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

fn run_skill_trace_safe_args_summary(payload: &Value) -> String {
    let args = payload
        .get("args")
        .map(run_skill_trace_safe_json)
        .unwrap_or(Value::Null);
    format!("args={}", crate::truncate_for_log(&args.to_string()))
}

fn record_direct_run_skill_verification(
    journal: &mut crate::task_journal::TaskJournal,
    verification: &DirectRunSkillVerification,
) {
    journal.record_plan_result(&verification.plan);
    journal.record_verify_result(&verification.verify);
}

fn run_skill_capability_contract(state: &AppState, payload: &Value, skill_name: &str) -> Value {
    let canonical = state.resolve_canonical_skill_name(skill_name);
    let action = run_skill_action_from_payload(payload);
    let args = payload
        .get("args")
        .map(run_skill_trace_safe_json)
        .unwrap_or(Value::Null);
    let registry = state.get_skills_registry();
    let manifest = registry
        .as_ref()
        .and_then(|registry| registry.manifest(&canonical));
    let planner_mapping = registry.as_ref().and_then(|registry| {
        claw_core::skill_registry::select_planner_capability_mapping(
            registry.planner_capabilities(&canonical),
            action.as_deref(),
        )
    });
    let effect = planner_mapping
        .and_then(|mapping| mapping.effect)
        .map(PlannerCapabilityEffect::as_token)
        .or_else(|| {
            manifest
                .as_ref()
                .and_then(|manifest| manifest.side_effect)
                .map(|side_effect| if side_effect { "mutate" } else { "observe" })
        })
        .unwrap_or("unknown");
    let risk_level = planner_mapping
        .and_then(|mapping| mapping.risk_level)
        .or_else(|| manifest.as_ref().and_then(|manifest| manifest.risk_level));
    let required_args = planner_mapping
        .map(|mapping| mapping.required.clone())
        .filter(|items| !items.is_empty())
        .unwrap_or_else(|| {
            json_required_fields(
                manifest
                    .as_ref()
                    .and_then(|manifest| manifest.input_schema.as_ref()),
            )
        });
    let optional_args = planner_mapping
        .map(|mapping| mapping.optional.clone())
        .unwrap_or_default();
    let expected_evidence = json_required_fields(
        manifest
            .as_ref()
            .and_then(|manifest| manifest.output_schema.as_ref()),
    );
    json!({
        "schema_version": 1,
        "source": "run_skill",
        "skill_name": skill_name,
        "canonical_skill_name": canonical,
        "action": action.as_deref().unwrap_or("_default"),
        "effect": effect,
        "risk_level": risk_level_token(risk_level),
        "required_args": required_args,
        "optional_args": optional_args,
        "expected_evidence": if expected_evidence.is_empty() { vec!["text".to_string()] } else { expected_evidence },
        "delivery_shape": manifest
            .as_ref()
            .map(|manifest| output_kind_token(manifest.output_kind))
            .unwrap_or("text"),
        "capability_ref": planner_mapping.map(|mapping| mapping.name.as_str()),
        "planner_kind": manifest
            .as_ref()
            .map(|manifest| manifest.planner_kind.as_token()),
        "idempotent": registry
            .as_ref()
            .map(|registry| registry.resolved_idempotent(&canonical, action.as_deref())),
        "dedup_scope": registry
            .as_ref()
            .map(|registry| registry.resolved_dedup_scope(&canonical, action.as_deref()).as_token()),
        "args_shape": args,
    })
}

fn run_skill_success_machine_payload() -> Value {
    json!({
        "status_code": "ok",
        "message_key": "clawd.run_skill.ok",
        "failure_attribution": null,
        "retryable": false,
    })
}

fn run_skill_failure_machine_payload(err_text: &str) -> Value {
    let structured = crate::skills::parse_structured_skill_error(err_text);
    let error_code = structured
        .as_ref()
        .map(|error| error.error_kind.as_str())
        .unwrap_or("skill_execution_failed");
    let message_key = structured
        .as_ref()
        .and_then(|error| error.extra.as_ref())
        .and_then(|extra| extra.get("message_key"))
        .and_then(Value::as_str)
        .unwrap_or("clawd.run_skill.execution_failed");
    let failure_attribution = crate::task_journal::failure_attribution_for_error_text(err_text)
        .map(|value| value.as_str())
        .unwrap_or("tool_gap");
    json!({
        "error_code": error_code,
        "status_code": error_code,
        "message_key": message_key,
        "failure_attribution": failure_attribution,
        "retryable": false,
    })
}

fn record_run_skill_task_observation(
    journal: &mut crate::task_journal::TaskJournal,
    skill_name: &str,
    status: &str,
    capability_contract: &Value,
    machine_payload: &Value,
    text: Option<&str>,
    error_text: Option<&str>,
    validation: Option<&Value>,
    extra: Option<&Value>,
    notify: Option<bool>,
    external_skill_admission: Option<&Value>,
) {
    let mut payload = json!({
        "source": "run_skill",
        "legacy_source": "direct_run_skill",
        "execution_surface": "worker/run_skill_finalize",
        "execution_surface_owner": "single_step_skill_compat",
        "skill_name": skill_name,
        "status": status,
        "status_code": machine_payload
            .get("status_code")
            .and_then(Value::as_str)
            .unwrap_or(status),
        "message_key": machine_payload.get("message_key").and_then(Value::as_str),
        "capability_contract": capability_contract,
    });
    if let Some(obj) = payload.as_object_mut() {
        if let Some(error_code) = machine_payload.get("error_code") {
            obj.insert("error_code".to_string(), error_code.clone());
        }
        if let Some(failure_attribution) = machine_payload.get("failure_attribution") {
            obj.insert(
                "failure_attribution".to_string(),
                failure_attribution.clone(),
            );
        }
        if let Some(retryable) = machine_payload.get("retryable") {
            obj.insert("retryable".to_string(), retryable.clone());
        }
        if let Some(text) = text {
            obj.insert("text".to_string(), json!(text));
        }
        if let Some(error_text) = error_text {
            obj.insert("error_text".to_string(), json!(error_text));
        }
        if let Some(validation) = validation {
            obj.insert("validation".to_string(), validation.clone());
        }
        if let Some(extra) = extra {
            obj.insert("extra".to_string(), extra.clone());
        }
        if let Some(notify) = notify {
            obj.insert("notify".to_string(), json!(notify));
        }
    }
    let payload_text = payload.to_string();
    if let Some(observed_evidence) =
        crate::task_journal::observed_evidence_from_output(Some(&payload_text))
    {
        journal.push_task_observation(json!({
            "source": "run_skill",
            "legacy_source": "direct_run_skill",
            "execution_surface": "worker/run_skill_finalize",
            "execution_surface_owner": "single_step_skill_compat",
            "skill": skill_name,
            "status": status,
            "status_code": machine_payload
                .get("status_code")
                .and_then(Value::as_str)
                .unwrap_or(status),
            "message_key": machine_payload.get("message_key").and_then(Value::as_str),
            "error_code": machine_payload.get("error_code").cloned(),
            "failure_attribution": machine_payload.get("failure_attribution").cloned(),
            "retryable": machine_payload.get("retryable").cloned(),
            "capability_contract": capability_contract,
            "capability_ref": capability_contract.get("capability_ref").cloned(),
            "external_skill_admission": external_skill_admission,
            "observed_evidence": observed_evidence,
        }));
    }
}

fn external_skill_admission_trace(state: &AppState, skill_name: &str) -> Option<Value> {
    let registry = state.get_skills_registry()?;
    let canonical = state.resolve_canonical_skill_name(skill_name);
    let entry = registry.get(&canonical)?;
    let is_external = entry.kind == claw_core::skill_registry::SkillKind::External
        || entry
            .external_bundle_dir
            .as_deref()
            .is_some_and(|value| !value.trim().is_empty())
        || entry.matrix_admission.is_some();
    if !is_external {
        return None;
    }
    let admission = entry.matrix_admission.as_ref();
    Some(json!({
        "schema_version": 1,
        "source": "skills_registry",
        "skill": canonical,
        "eligible": admission.map(|value| value.eligible).unwrap_or(false),
        "declared_actions": admission
            .map(|value| value.declared_actions.clone())
            .unwrap_or_default(),
        "evidence_sources": admission
            .map(|value| value.evidence_sources.clone())
            .unwrap_or_default(),
        "required_extra_fields": admission
            .map(|value| value.required_extra_fields.clone())
            .unwrap_or_default(),
        "extractor_kind": admission.and_then(|value| value.extractor_kind.clone()),
        "admission_version": admission.and_then(|value| value.admission_version.clone()),
    }))
}

fn pending_async_job_ref_from_extra(extra: Option<&Value>) -> Result<Option<AsyncJobRef>, String> {
    crate::async_job_contract::parse_pending_async_job_ref_from_extra(
        extra,
        DIRECT_RUN_SKILL_ASYNC_ERROR_PREFIX,
    )
}

fn pending_async_job_poll_adapter_from_extra(
    extra: Option<&Value>,
) -> Result<Option<Value>, String> {
    crate::async_job_contract::parse_pending_async_job_poll_adapter_from_extra(
        extra,
        DIRECT_RUN_SKILL_ASYNC_ERROR_PREFIX,
    )
}

fn direct_run_skill_async_checkpoint_payload(
    task: &crate::ClaimedTask,
    skill_name: &str,
    clean_text: &str,
    capability_results: &[claw_core::capability_result::CapabilityResultEnvelope],
    job: &AsyncJobRef,
    poll_adapter: Option<&Value>,
    now_ts: i64,
    budget: CheckpointBudgetCounters,
) -> Value {
    let timeout_policy =
        crate::async_job_contract::pending_async_job_timeout_policy(poll_adapter, job, now_ts);
    let checkpoint_id = format!("run-skill:{}:async-job:{}", task.task_id, job.job_id);
    let mut boundary_context = json!({
        "schema_version": 1,
        "source": DIRECT_RUN_SKILL_ASYNC_SOURCE,
        "task_id": task.task_id,
        "skill": skill_name,
        "execution_surface": "worker/run_skill_finalize",
    });
    if let (Some(obj), Some(adapter)) = (
        boundary_context.as_object_mut(),
        poll_adapter.filter(|value| value.is_object()),
    ) {
        obj.insert("async_poll_adapter".to_string(), adapter.clone());
    }
    let budget_json = serde_json::to_value(&budget).unwrap_or_else(|_| json!({}));
    let checkpoint = TaskCheckpoint {
        schema_version: 1,
        checkpoint_id: checkpoint_id.clone(),
        boundary_context,
        last_successful_round: None,
        last_successful_step: Some("run_skill".to_string()),
        pending_action: None,
        observations: vec![json!({
            "step_id": "run_skill",
            "skill": skill_name,
            "status": "ok",
            "has_output": !clean_text.trim().is_empty(),
            "has_error": false,
            "async_job_id": job.job_id,
        })],
        capability_results: capability_results.to_vec(),
        evidence_refs: vec!["run_skill".to_string()],
        artifact_refs: Vec::new(),
        completed_side_effect_refs: vec![format!(
            "run_skill:{skill_name}:async_job:{}",
            job.job_id
        )],
        budget,
        attempt_ledger: None,
        pending_async_job: Some(job.clone()),
        repair_signal: None,
        resume_entrypoint: ResumeEntrypoint::PollAsyncJob,
    };
    json!({
        "text": clean_text,
        "delivery_meta": {
            "mode": "single_step_skill_async_start",
            "label": "step",
            "skill_name": skill_name,
        },
        "task_lifecycle": {
            "schema_version": 1,
            "state": TaskLifecycleState::Waiting,
            "source": DIRECT_RUN_SKILL_ASYNC_SOURCE,
            "resume_reason": "pending_async_job",
            "next_check_after": now_ts.saturating_add(job.poll_after_seconds as i64).max(now_ts + 1),
            "checkpoint_id": checkpoint_id,
            "poll_ref": job.job_id,
            "cancel_ref": job.cancel_ref,
            "poll_after_seconds": job.poll_after_seconds,
            "async_job_expires_at": job.expires_at,
            "async_job_message_key": job.message_key,
            "async_timeout_policy": timeout_policy,
            "budget": budget_json,
            "can_poll": true,
            "can_cancel": true,
            "last_heartbeat_ts": now_ts,
        },
        "task_checkpoint": checkpoint.to_machine_json(),
    })
}

fn direct_run_skill_pending_async_checkpoint_result(
    task: &crate::ClaimedTask,
    skill_name: &str,
    clean_text: &str,
    journal: &mut crate::task_journal::TaskJournal,
    extra: Option<&Value>,
    budget: CheckpointBudgetCounters,
) -> Result<Option<Value>, String> {
    let Some(job) = pending_async_job_ref_from_extra(extra)? else {
        return Ok(None);
    };
    let poll_adapter = pending_async_job_poll_adapter_from_extra(extra)?;
    let payload = direct_run_skill_async_checkpoint_payload(
        task,
        skill_name,
        clean_text,
        &journal.capability_results,
        &job,
        poll_adapter.as_ref(),
        crate::now_ts_u64() as i64,
        budget,
    );
    if let Some(lifecycle) = payload.get("task_lifecycle").cloned() {
        journal.record_task_lifecycle(lifecycle);
    }
    if let Some(checkpoint) = payload.get("task_checkpoint").cloned() {
        journal.record_task_checkpoint(checkpoint);
    }
    journal.record_final_stop_signal("async_job_checkpoint_waiting");
    Ok(Some(journal.attach_to_result(payload)))
}

async fn finalize_run_skill_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    skill_name: &str,
    verification: &DirectRunSkillVerification,
    outcome: crate::skills::SkillRunOutcome,
) -> Result<()> {
    let clean_text = crate::intercept_response_text_for_delivery(&outcome.text);
    let mut journal = crate::task_journal::TaskJournal::for_task(
        &task.task_id,
        "run_skill",
        format!("run_skill:{skill_name}"),
    );
    journal.record_task_goal_spec_from_payload_json(&task.payload_json);
    journal.record_runtime_llm_metrics(state, &task.task_id);
    journal.record_used_evidence_ids_count(0);
    journal.record_context_bundle_summary(run_skill_trace_safe_args_summary(payload));
    record_direct_run_skill_verification(&mut journal, verification);
    let step_result = build_run_skill_step_result(
        skill_name,
        crate::executor::StepExecutionStatus::Ok,
        Some(clean_text.clone()),
        None,
    );
    let args = payload.get("args").cloned().unwrap_or_else(|| json!({}));
    journal
        .capability_results
        .push(crate::capability_result::envelope_for_step_execution(
            skill_name,
            &args,
            &step_result,
            outcome.extra.as_ref(),
        ));
    journal.push_step_result(&step_result);
    let capability_contract = run_skill_capability_contract(state, payload, skill_name);
    let machine_payload = run_skill_success_machine_payload();
    let external_skill_admission = external_skill_admission_trace(state, skill_name);
    record_run_skill_task_observation(
        &mut journal,
        skill_name,
        "ok",
        &capability_contract,
        &machine_payload,
        Some(&clean_text),
        None,
        outcome.validation.as_ref(),
        outcome.extra.as_ref(),
        outcome.notify,
        external_skill_admission.as_ref(),
    );
    journal.record_delivery_consistent(crate::task_journal::delivery_payload_consistent(
        &clean_text,
        &[],
    ));
    let pending_checkpoint_result = match direct_run_skill_pending_async_checkpoint_result(
        task,
        skill_name,
        &clean_text,
        &mut journal,
        outcome.extra.as_ref(),
        CheckpointBudgetCounters {
            round: 0,
            step: 1,
            llm_calls: u32::try_from(state.task_llm_call_count(&task.task_id)).unwrap_or(u32::MAX),
            tool_calls: 1,
            elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
            llm_elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
            tool_elapsed_ms: 0,
        },
    ) {
        Ok(result) => result,
        Err(err) => {
            finalize_run_skill_failure(state, task, payload, skill_name, verification, &err)
                .await?;
            return Ok(());
        }
    };
    if let Some(result) = pending_checkpoint_result {
        repo::update_task_checkpointed_result(
            state,
            &task.task_id,
            task.claim_attempt,
            &result.to_string(),
        )?;
        let _ = repo::insert_audit_log(
            state,
            Some(task.user_id),
            "run_skill",
            Some(
                &json!({
                    "task_id": task.task_id,
                    "chat_id": task.chat_id,
                    "skill_name": skill_name,
                    "status": "waiting",
                    "resume_reason": "pending_async_job"
                })
                .to_string(),
            ),
            None,
        );
        info!("{}", crate::LOG_CALL_WRAP);
        info!(
            "task_call_end task_id={} kind=run_skill status=waiting skill={} resume_reason=pending_async_job",
            task.task_id,
            skill_name
        );
        info!(
            "task_journal_summary task_id={} kind=run_skill phase=async_wait {}",
            task.task_id,
            journal.to_log_json()
        );
        info!("{}", crate::LOG_CALL_WRAP);
        state.clear_task_llm_call_count(&task.task_id);
        return Ok(());
    }
    journal.record_final_answer(&clean_text);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
    if outcome.notify.unwrap_or(true) {
        let notify_outcome =
            super::maybe_notify_schedule_result(state, task, payload, true, &clean_text).await;
        super::record_schedule_notify_outcome(&mut journal, notify_outcome);
    }
    let result = journal.attach_to_result(json!({
        "text": clean_text,
        "delivery_meta": {
            "mode": "single_step_skill",
            "label": "step",
            "skill_name": skill_name
        }
    }));
    repo::update_task_success(
        state,
        &task.task_id,
        task.claim_attempt,
        &result.to_string(),
    )?;
    let _ = crate::memory::service::insert_memory_with_kind(
        state,
        task.user_id,
        task.chat_id,
        task.user_key.as_deref(),
        &task.channel,
        task.external_chat_id.as_deref(),
        crate::memory::MEMORY_ROLE_ASSISTANT,
        &clean_text,
        state.policy.memory.item_max_chars.max(256),
        crate::memory::MemoryWriteKind::AssistantOutcome,
    );
    let _ = repo::insert_audit_log(
        state,
        Some(task.user_id),
        "run_skill",
        Some(
            &json!({
                "task_id": task.task_id,
                "chat_id": task.chat_id,
                "skill_name": skill_name,
                "status": "ok"
            })
            .to_string(),
        ),
        None,
    );
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=run_skill status=success skill={} result={}",
        task.task_id,
        skill_name,
        crate::truncate_for_log(&clean_text)
    );
    info!(
        "task_journal_summary task_id={} kind=run_skill phase=finalize {}",
        task.task_id,
        journal.to_log_json()
    );
    info!("{}", crate::LOG_CALL_WRAP);
    state.clear_task_llm_call_count(&task.task_id);
    Ok(())
}

async fn finalize_run_skill_failure(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    skill_name: &str,
    verification: &DirectRunSkillVerification,
    err_text: &str,
) -> Result<()> {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        &task.task_id,
        "run_skill",
        format!("run_skill:{skill_name}"),
    );
    journal.record_task_goal_spec_from_payload_json(&task.payload_json);
    journal.record_runtime_llm_metrics(state, &task.task_id);
    journal.record_used_evidence_ids_count(0);
    journal.record_context_bundle_summary(run_skill_trace_safe_args_summary(payload));
    record_direct_run_skill_verification(&mut journal, verification);
    let step_result = build_run_skill_step_result(
        skill_name,
        crate::executor::StepExecutionStatus::Error,
        None,
        Some(err_text.to_string()),
    );
    let args = payload.get("args").cloned().unwrap_or_else(|| json!({}));
    journal
        .capability_results
        .push(crate::capability_result::envelope_for_step_execution(
            skill_name,
            &args,
            &step_result,
            None,
        ));
    journal.push_step_result(&step_result);
    let capability_contract = run_skill_capability_contract(state, payload, skill_name);
    let machine_payload = run_skill_failure_machine_payload(err_text);
    let external_skill_admission = external_skill_admission_trace(state, skill_name);
    record_run_skill_task_observation(
        &mut journal,
        skill_name,
        "error",
        &capability_contract,
        &machine_payload,
        None,
        Some(err_text),
        None,
        None,
        None,
        external_skill_admission.as_ref(),
    );
    journal.record_delivery_consistent(crate::task_journal::delivery_payload_consistent(
        err_text,
        &[],
    ));
    journal.record_final_answer(err_text);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.record_final_failure_attribution_from_error(err_text);
    let notify_outcome =
        super::maybe_notify_schedule_result(state, task, payload, false, err_text).await;
    super::record_schedule_notify_outcome(&mut journal, notify_outcome);
    error!(
        "worker_once: run_skill task_id={} skill={} failed: {}",
        task.task_id, skill_name, err_text
    );
    let result = journal.attach_to_result(json!({
        "text": err_text,
    }));
    repo::update_task_failure_with_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &result.to_string(),
        err_text,
    )?;
    let _ = repo::insert_audit_log(
        state,
        Some(task.user_id),
        "run_skill",
        Some(
            &json!({
                "task_id": task.task_id,
                "chat_id": task.chat_id,
                "skill_name": skill_name,
                "status": "failed"
            })
            .to_string(),
        ),
        Some(err_text),
    );
    info!("{}", crate::LOG_CALL_WRAP);
    info!(
        "task_call_end task_id={} kind=run_skill status=failed skill={} error={}",
        task.task_id,
        skill_name,
        crate::truncate_for_log(err_text)
    );
    info!(
        "task_journal_summary task_id={} kind=run_skill phase=failure error={} {}",
        task.task_id,
        crate::truncate_for_log(err_text),
        journal.to_log_json()
    );
    info!("{}", crate::LOG_CALL_WRAP);
    state.clear_task_llm_call_count(&task.task_id);
    Ok(())
}

pub(super) async fn finalize_run_skill_confirmation_required(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    skill_name: &str,
    verification: &DirectRunSkillVerification,
) -> Result<()> {
    let confirmation_step_ids =
        crate::approval_grant::confirmation_step_ids(&verification.verify.issues);
    let detail = verification
        .verify
        .blocked_reason
        .as_deref()
        .unwrap_or("explicit_approval_required");
    let (user_error, resume_context) =
        crate::agent_engine::build_confirmation_required_resume_context(
            state,
            task,
            &verification.plan.steps,
            &verification.request_envelope,
            &verification.plan.goal,
            &[],
            &[],
            detail,
            &confirmation_step_ids,
        )
        .await;
    let required_decision = resume_context
        .get("required_decision")
        .and_then(Value::as_str)
        .and_then(crate::policy_decision::PolicyDecision::parse_token)
        .unwrap_or(crate::policy_decision::PolicyDecision::Deny);
    let (failure_reason, audit_status) = match required_decision {
        crate::policy_decision::PolicyDecision::BackgroundWait => {
            ("hook_background_wait", "background_wait")
        }
        crate::policy_decision::PolicyDecision::Deny => {
            ("hook_permission_denied", "permission_denied")
        }
        _ => ("explicit_approval_required", "approval_required"),
    };
    let mut journal = crate::task_journal::TaskJournal::for_task(
        &task.task_id,
        "run_skill",
        format!("run_skill:{skill_name}"),
    );
    journal.record_task_goal_spec_from_payload_json(&task.payload_json);
    journal.record_runtime_llm_metrics(state, &task.task_id);
    journal.record_used_evidence_ids_count(0);
    journal.record_context_bundle_summary(run_skill_trace_safe_args_summary(payload));
    record_direct_run_skill_verification(&mut journal, verification);
    journal.record_delivery_consistent(crate::task_journal::delivery_payload_consistent(
        &user_error,
        &[],
    ));
    journal.record_final_answer(&user_error);
    if required_decision == crate::policy_decision::PolicyDecision::RequireConfirmation
        && resume_context
            .pointer("/approval_request/status")
            .and_then(Value::as_str)
            == Some("pending")
    {
        let now_ts = crate::now_ts_u64() as i64;
        let checkpoint_id = format!("run-skill:{}:approval", task.task_id);
        let approval_request = resume_context
            .get("approval_request")
            .cloned()
            .unwrap_or_else(|| json!({}));
        let budget = CheckpointBudgetCounters {
            round: 0,
            step: 0,
            llm_calls: u32::try_from(state.task_llm_call_count(&task.task_id)).unwrap_or(u32::MAX),
            tool_calls: 0,
            elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
            llm_elapsed_ms: state.task_llm_elapsed_ms(&task.task_id),
            tool_elapsed_ms: 0,
        };
        let checkpoint = TaskCheckpoint {
            schema_version: 1,
            checkpoint_id: checkpoint_id.clone(),
            boundary_context: json!({
                "schema_version": 1,
                "source": "direct_run_skill_plan_verifier",
                "stage": "permission_request",
                "decision": required_decision.as_token(),
                "task_id": task.task_id,
                "skill": skill_name,
                "resume_reason": "confirmation_required",
                "message_key": resume_context.get("message_key").cloned().unwrap_or(Value::Null),
            }),
            last_successful_round: None,
            last_successful_step: None,
            pending_action: Some(json!({
                "schema_version": 1,
                "kind": "approval_request",
                "request_id": approval_request.get("request_id").cloned().unwrap_or(Value::Null),
                "action_fingerprint": approval_request
                    .get("action_fingerprint")
                    .cloned()
                    .unwrap_or(Value::Null),
                "arguments_hash": approval_request
                    .get("arguments_hash")
                    .cloned()
                    .unwrap_or(Value::Null),
                "targets": approval_request.get("targets").cloned().unwrap_or_else(|| json!([])),
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
            resume_entrypoint: ResumeEntrypoint::AwaitUserInput,
        };
        let lifecycle = json!({
            "schema_version": 1,
            "state": TaskLifecycleState::NeedsUser,
            "source": "direct_run_skill_plan_verifier",
            "resume_reason": "confirmation_required",
            "checkpoint_id": checkpoint_id,
            "can_poll": true,
            "can_cancel": true,
            "last_heartbeat_ts": now_ts,
            "message_key": resume_context.get("message_key").cloned().unwrap_or(Value::Null),
            "decision": required_decision.as_token(),
            "tool_or_skill": skill_name,
            "budget": serde_json::to_value(&budget).unwrap_or_else(|_| json!({})),
        });
        journal.record_task_lifecycle(lifecycle.clone());
        journal.record_task_checkpoint(checkpoint.to_machine_json());
        journal.record_final_stop_signal("approval_checkpoint_needs_user");
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        let result = journal.attach_to_result(json!({
            "text": user_error,
            "resume_context": resume_context,
            "task_lifecycle": lifecycle,
        }));
        repo::update_task_checkpointed_result(
            state,
            &task.task_id,
            task.claim_attempt,
            &result.to_string(),
        )?;
        let _ = repo::insert_audit_log(
            state,
            Some(task.user_id),
            "run_skill",
            Some(
                &json!({
                    "task_id": task.task_id,
                    "chat_id": task.chat_id,
                    "skill_name": skill_name,
                    "status": "needs_user",
                    "resume_reason": "confirmation_required",
                })
                .to_string(),
            ),
            None,
        );
        info!(
            "task_call_checkpointed task_id={} kind=run_skill lifecycle_state=needs_user skill={} resume_reason=confirmation_required",
            task.task_id, skill_name
        );
        state.clear_task_llm_call_count(&task.task_id);
        return Ok(());
    }
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
    journal.record_final_failure_attribution_from_error(failure_reason);
    let notify_outcome =
        super::maybe_notify_schedule_result(state, task, payload, false, &user_error).await;
    super::record_schedule_notify_outcome(&mut journal, notify_outcome);
    let result = journal.attach_to_result(json!({
        "text": user_error,
        "resume_context": resume_context,
    }));
    repo::update_task_failure_with_result(
        state,
        &task.task_id,
        task.claim_attempt,
        &result.to_string(),
        failure_reason,
    )?;
    let _ = repo::insert_audit_log(
        state,
        Some(task.user_id),
        "run_skill",
        Some(
            &json!({
                "task_id": task.task_id,
                "chat_id": task.chat_id,
                "skill_name": skill_name,
                "status": audit_status,
            })
            .to_string(),
        ),
        None,
    );
    state.clear_task_llm_call_count(&task.task_id);
    Ok(())
}

pub(super) async fn finalize_run_skill_result(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    skill_name: &str,
    verification: &DirectRunSkillVerification,
    result: Result<crate::skills::SkillRunOutcome, String>,
) -> Result<()> {
    match result {
        Ok(outcome) => {
            if !repo::is_task_claim_active(state, &task.task_id, task.claim_attempt)? {
                state.clear_task_llm_call_count(&task.task_id);
                return finalize_run_skill_canceled(task, skill_name).await;
            }
            finalize_run_skill_success(state, task, payload, skill_name, verification, outcome)
                .await?;
        }
        Err(err_text) => {
            if !repo::is_task_claim_active(state, &task.task_id, task.claim_attempt)? {
                state.clear_task_llm_call_count(&task.task_id);
                return finalize_run_skill_canceled(task, skill_name).await;
            }
            finalize_run_skill_failure(state, task, payload, skill_name, verification, &err_text)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "run_skill_finalize_tests.rs"]
mod tests;
