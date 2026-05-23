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
            "observed_evidence": observed_evidence,
        }));
    }
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
    record_run_skill_task_observation(
        &mut journal,
        skill_name,
        "ok",
        Some(&clean_text),
        None,
        outcome.validation.as_ref(),
        outcome.extra.as_ref(),
        outcome.notify,
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
    record_run_skill_task_observation(
        &mut journal,
        skill_name,
        "error",
        None,
        Some(err_text),
        None,
        None,
        None,
    );
    journal.record_delivery_consistent(crate::task_journal::delivery_payload_consistent(
        err_text,
        &[],
    ));
    journal.record_final_answer(err_text);
    journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Failure);
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
mod tests {
    use serde_json::{json, Value};

    #[test]
    fn direct_run_skill_observation_records_redacted_extra_evidence() {
        let token = "sk-test_abcdefghijklmnopqrstuvwxyz1234567890";
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-1", "run_skill", "run_skill:demo");

        super::record_run_skill_task_observation(
            &mut journal,
            "demo",
            "ok",
            Some("done"),
            None,
            Some(&json!({"ok": true})),
            Some(&json!({
                "api_token": token,
                "result": {
                    "path": "/tmp/output.txt",
                    "exists": true
                }
            })),
            Some(true),
        );

        let trace = journal.to_trace_json();
        let trace_text = trace.to_string();
        assert!(!trace_text.contains(token));
        assert_eq!(
            trace
                .get("task_observations")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );

        let items = trace
            .get("task_observations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|entry| entry.get("observed_evidence"))
            .and_then(|evidence| evidence.get("items"))
            .and_then(Value::as_array)
            .expect("observed evidence items");

        let token_item = items
            .iter()
            .find(|item| item.get("field").and_then(Value::as_str) == Some("extra.api_token"))
            .expect("extra api token item");
        assert_eq!(
            token_item.get("redacted").and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn direct_run_skill_failure_records_error_observation() {
        let mut journal =
            crate::task_journal::TaskJournal::for_task("task-2", "run_skill", "run_skill:demo");

        super::record_run_skill_task_observation(
            &mut journal,
            "demo",
            "error",
            None,
            Some("missing required field: path"),
            None,
            None,
            None,
        );

        let trace = journal.to_trace_json();
        let observed = trace
            .get("task_observations")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|entry| entry.get("observed_evidence"))
            .expect("observed evidence");
        assert_eq!(
            observed.get("source").and_then(Value::as_str),
            Some("step_output")
        );
        assert_eq!(
            trace
                .get("task_observations")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|entry| entry.get("source"))
                .and_then(Value::as_str),
            Some("direct_run_skill")
        );
    }
}
