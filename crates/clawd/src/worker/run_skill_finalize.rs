use anyhow::Result;
use serde_json::{json, Value};
use tracing::{error, info};

use crate::{repo, AppState};

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

fn record_run_skill_task_observation(
    journal: &mut crate::task_journal::TaskJournal,
    skill_name: &str,
    status: &str,
    text: Option<&str>,
    error_text: Option<&str>,
    validation: Option<&Value>,
    extra: Option<&Value>,
    notify: Option<bool>,
    external_skill_admission: Option<&Value>,
) {
    let mut payload = json!({
        "source": "direct_run_skill",
        "skill_name": skill_name,
        "status": status,
    });
    if let Some(obj) = payload.as_object_mut() {
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
            "source": "direct_run_skill",
            "skill": skill_name,
            "status": status,
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

async fn finalize_run_skill_success(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    skill_name: &str,
    outcome: crate::skills::SkillRunOutcome,
) -> Result<()> {
    let clean_text = crate::intercept_response_text_for_delivery(&outcome.text);
    let mut journal = crate::task_journal::TaskJournal::for_task(
        &task.task_id,
        "run_skill",
        format!("run_skill:{skill_name}"),
    );
    journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
    journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
    journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
    journal.record_used_evidence_ids_count(0);
    journal.record_context_bundle_summary(format!(
        "args={}",
        crate::truncate_for_log(
            &payload
                .get("args")
                .cloned()
                .unwrap_or(Value::Null)
                .to_string()
        )
    ));
    journal.push_step_result(&build_run_skill_step_result(
        skill_name,
        crate::executor::StepExecutionStatus::Ok,
        Some(clean_text.clone()),
        None,
    ));
    let external_skill_admission = external_skill_admission_trace(state, skill_name);
    record_run_skill_task_observation(
        &mut journal,
        skill_name,
        "ok",
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
    repo::update_task_success(state, &task.task_id, &result.to_string())?;
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
    err_text: &str,
) -> Result<()> {
    let mut journal = crate::task_journal::TaskJournal::for_task(
        &task.task_id,
        "run_skill",
        format!("run_skill:{skill_name}"),
    );
    journal.record_llm_calls_per_task(state.task_llm_call_count(&task.task_id));
    journal.record_llm_elapsed_ms_per_task(state.task_llm_elapsed_ms(&task.task_id));
    journal.record_llm_by_prompt(state.task_llm_by_prompt(&task.task_id));
    journal.record_used_evidence_ids_count(0);
    journal.record_context_bundle_summary(format!(
        "args={}",
        crate::truncate_for_log(
            &payload
                .get("args")
                .cloned()
                .unwrap_or(Value::Null)
                .to_string()
        )
    ));
    journal.push_step_result(&build_run_skill_step_result(
        skill_name,
        crate::executor::StepExecutionStatus::Error,
        None,
        Some(err_text.to_string()),
    ));
    let external_skill_admission = external_skill_admission_trace(state, skill_name);
    record_run_skill_task_observation(
        &mut journal,
        skill_name,
        "error",
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
    repo::update_task_failure_with_result(state, &task.task_id, &result.to_string(), err_text)?;
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

pub(crate) async fn finalize_run_skill_result(
    state: &AppState,
    task: &crate::ClaimedTask,
    payload: &Value,
    skill_name: &str,
    result: Result<crate::skills::SkillRunOutcome, String>,
) -> Result<()> {
    match result {
        Ok(outcome) => {
            if !repo::is_task_still_running(state, &task.task_id)? {
                state.clear_task_llm_call_count(&task.task_id);
                return finalize_run_skill_canceled(task, skill_name).await;
            }
            finalize_run_skill_success(state, task, payload, skill_name, outcome).await?;
        }
        Err(err_text) => {
            if !repo::is_task_still_running(state, &task.task_id)? {
                state.clear_task_llm_call_count(&task.task_id);
                return finalize_run_skill_canceled(task, skill_name).await;
            }
            finalize_run_skill_failure(state, task, payload, skill_name, &err_text).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "run_skill_finalize_tests.rs"]
mod tests;
