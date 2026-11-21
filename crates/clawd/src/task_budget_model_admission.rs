//! Pre-call projection of TaskBudgetPolicy, shared by all model stages and provider attempts.
//! Unknown in-flight usage retains a durable conservative reservation across restart.
use crate::providers::client::{ChatRequestHints, LlmUsageSnapshot, ProviderError};
use crate::{AppState, ClaimedTask};
use rusqlite::{params, TransactionBehavior};
use serde_json::json;
use std::{sync::Arc, time::Instant};
const ERROR: &str = "task_budget_token_admission_exhausted";

pub(crate) struct ModelAdmission {
    state: AppState,
    task: ClaimedTask,
    ceiling: u64,
    input_bound: u64,
    output_cap: u64,
    label: String,
}
impl std::fmt::Debug for ModelAdmission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelAdmission")
            .field("ceiling", &self.ceiling)
            .field("input_bound", &self.input_bound)
            .field("output_cap", &self.output_cap)
            .finish()
    }
}
pub(crate) fn prepare(
    state: &AppState,
    task: &ClaimedTask,
    serialized_input: &str,
    label: &str,
    hints: &mut ChatRequestHints,
) -> Result<Arc<ModelAdmission>, String> {
    let policy = super::load_task_budget_policy(&state.skill_rt.workspace_root);
    let payload: serde_json::Value = serde_json::from_str(&task.payload_json).unwrap_or_default();
    let ceiling = payload
        .pointer("/child_task_contract/budget/max_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(policy.hard_ceilings.total_tokens)
        .min(policy.hard_ceilings.total_tokens);
    // UTF-8 bytes bound text-token use conservatively; reserve framing/tool wrapper overhead too.
    // This is admission evidence, never fabricated provider usage.
    let input_bound = (serialized_input.len() as u64).saturating_add(8192);
    let stage_cap = if label == "verifier" { 4096 } else { 8192 };
    let output_cap = hints.max_tokens.unwrap_or(stage_cap).min(stage_cap).max(1);
    let admission = Arc::new(ModelAdmission {
        state: state.clone(),
        task: task.clone(),
        ceiling,
        input_bound,
        output_cap,
        label: label.into(),
    });
    let db = state
        .core
        .db
        .get()
        .map_err(|_| "task_budget_ledger_unavailable")?;
    db.execute_batch("CREATE TABLE IF NOT EXISTS task_model_token_reservations (attempt_id TEXT PRIMARY KEY, task_id TEXT NOT NULL, reserved_tokens INTEGER NOT NULL, status TEXT NOT NULL);
        CREATE INDEX IF NOT EXISTS idx_task_model_token_reservations_task ON task_model_token_reservations(task_id);
        CREATE TABLE IF NOT EXISTS task_model_budget_baselines (task_id TEXT PRIMARY KEY, historical_tokens INTEGER NOT NULL);")
        .map_err(|_| "task_budget_ledger_unavailable")?;
    let cost = state.task_llm_cost_summary(&task.task_id);
    db.execute("INSERT OR IGNORE INTO task_model_budget_baselines(task_id,historical_tokens) VALUES(?1,?2)",params![task.task_id,cost.input_tokens.saturating_add(cost.output_tokens)])
        .map_err(|_| "task_budget_ledger_unavailable")?;
    admission
        .remaining(&db)?
        .checked_sub(input_bound)
        .filter(|n| *n >= 256)
        .ok_or(ERROR)?;
    hints.max_tokens = Some(output_cap);
    if label == "verifier" && hints.reasoning_effort.is_none() {
        hints.reasoning_effort = Some("low".into());
    }
    Ok(admission)
}
impl ModelAdmission {
    fn remaining(&self, db: &rusqlite::Connection) -> Result<u64, String> {
        let cost = self.state.task_llm_cost_summary(&self.task.task_id);
        let held: u64 = db.query_row("SELECT COALESCE(SUM(reserved_tokens),0) FROM task_model_token_reservations WHERE task_id=?1",[&self.task.task_id],|r|r.get(0)).map_err(|_| "task_budget_ledger_unavailable")?;
        let baseline: u64 = db
            .query_row(
                "SELECT historical_tokens FROM task_model_budget_baselines WHERE task_id=?1",
                [&self.task.task_id],
                |r| r.get(0),
            )
            .map_err(|_| "task_budget_ledger_unavailable")?;
        Ok(self.ceiling.saturating_sub(
            cost.input_tokens
                .saturating_add(cost.output_tokens)
                .max(baseline.saturating_add(held)),
        ))
    }
    pub(crate) fn start(
        self: &Arc<Self>,
        hints: &mut ChatRequestHints,
        provider: &str,
        model: &str,
    ) -> Result<Attempt, String> {
        let mut db = self
            .state
            .core
            .db
            .get()
            .map_err(|_| "task_budget_ledger_unavailable")?;
        let tx = db
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|_| "task_budget_ledger_unavailable")?;
        let remaining = self.remaining(&tx)?;
        let output = remaining
            .checked_sub(self.input_bound)
            .filter(|n| *n >= 256)
            .ok_or(ERROR)?
            .min(self.output_cap);
        hints.max_tokens = Some(output);
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute("INSERT INTO task_model_token_reservations(attempt_id,task_id,reserved_tokens,status) VALUES(?1,?2,?3,'in_flight')",params![id,self.task.task_id,self.input_bound.saturating_add(output)]).map_err(|_| "task_budget_ledger_unavailable")?;
        tx.commit().map_err(|_| "task_budget_ledger_unavailable")?;
        let attempt = Attempt {
            owner: self.clone(),
            id,
            provider: provider.into(),
            model: model.into(),
            started: Instant::now(),
            finished: false,
        };
        attempt.emit("started", None, None, 0);
        Ok(attempt)
    }
}
pub(crate) struct Attempt {
    owner: Arc<ModelAdmission>,
    id: String,
    provider: String,
    model: String,
    started: Instant,
    finished: bool,
}
impl Attempt {
    fn emit(&self, status: &str, error: Option<&str>, http_status: Option<u16>, backoff_ms: u64) {
        let record = json!({"schema_version":1,"attempt_id":self.id,"logical_call_index":self.owner.state.task_llm_call_count(&self.owner.task.task_id),
            "prompt_label":self.owner.label,"provider":self.provider,"model":self.model,"status":status,
            "elapsed_ms":self.started.elapsed().as_millis() as u64,"error_category":error,"http_status":http_status,"backoff_ms":backoff_ms});
        tracing::info!(task_id=%self.owner.task.task_id, attempt=%record, "model_provider_attempt");
        let _ = crate::task_event_transport::publish_claimed_event(
            &self.owner.state,
            &self.owner.task,
            "model_provider_attempt",
            record,
        );
    }
    pub(crate) fn finish(
        mut self,
        usage: Option<&LlmUsageSnapshot>,
        error: Option<&ProviderError>,
        backoff_ms: u64,
    ) {
        // Successful and failed calls with known usage are accounted by the existing cost ledger.
        // A missing usage record keeps its reservation, including when the request is cancelled.
        if let Ok(db) = self.owner.state.core.db.get() {
            if let Some(usage) =
                usage.filter(|u| u.input_tokens.is_some() && u.output_tokens.is_some())
            {
                let charge = usage
                    .input_tokens
                    .unwrap_or(0)
                    .saturating_add(usage.output_tokens.unwrap_or(0));
                let _ = db.execute("UPDATE task_model_token_reservations SET status='usage_known',reserved_tokens=?2 WHERE attempt_id=?1",params![self.id,charge]);
            } else {
                let _ = db.execute("UPDATE task_model_token_reservations SET status='usage_unknown' WHERE attempt_id=?1",[&self.id]);
            }
        }
        self.emit(
            if error.is_some() {
                "failed"
            } else {
                "completed"
            },
            error.map(ProviderError::observability_kind),
            error.and_then(|e| e.http_status),
            backoff_ms,
        );
        self.finished = true;
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            if let Ok(db) = self.owner.state.core.db.get() {
                let _ = db.execute("UPDATE task_model_token_reservations SET status='interrupted_usage_unknown' WHERE attempt_id=?1",[&self.id]);
            }
            self.emit("interrupted", Some("cancelled_or_dropped"), None, 0);
        }
    }
}

#[cfg(test)]
#[path = "task_budget_model_admission_tests.rs"]
mod tests;
