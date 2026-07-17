use serde::{Deserialize, Serialize};

use super::AppState;
use crate::providers::{summarize_task_cost, LlmCallCostRecord, LlmTaskCostSummary};
use crate::{ClaimedTask, TaskCostBlocker};

const MAX_LLM_COST_RECORDS_PER_TASK: usize = 512;
const NANOS_PER_USD: f64 = 1_000_000_000.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LlmCostBudgetSnapshot {
    pub(crate) status: String,
    pub(crate) enforcement: String,
    pub(crate) provider: Option<String>,
    pub(crate) task_known_cost_usd_nanos: u64,
    pub(crate) user_24h_known_cost_usd_nanos: u64,
    pub(crate) provider_24h_known_cost_usd_nanos: Option<u64>,
    pub(crate) task_unknown_record_count: u64,
    pub(crate) user_24h_unknown_record_count: u64,
    pub(crate) provider_24h_unknown_record_count: Option<u64>,
    pub(crate) soft_task_limit_usd_nanos: Option<u64>,
    pub(crate) soft_user_24h_limit_usd_nanos: Option<u64>,
    pub(crate) soft_provider_24h_limit_usd_nanos: Option<u64>,
    pub(crate) hard_task_limit_usd_nanos: Option<u64>,
    pub(crate) hard_exceeded: bool,
    pub(crate) signals: Vec<String>,
}

impl AppState {
    pub(crate) fn note_task_llm_cost_record(
        &self,
        task: &ClaimedTask,
        record: LlmCallCostRecord,
    ) -> Result<(), String> {
        {
            let mut guard = self.metrics.llm_cost_records_per_task.lock().unwrap();
            let records = guard.entry(task.task_id.clone()).or_default();
            if records.len() < MAX_LLM_COST_RECORDS_PER_TASK {
                records.push(record.clone());
            }
        }
        super::llm_cost_ledger::append_record(&self.core.db, task, &record)
    }

    pub(crate) fn restore_task_llm_call_count_from_cost_ledger(&self, task_id: &str) {
        let Ok(persisted_count) =
            super::llm_cost_ledger::max_logical_call_index(&self.core.db, task_id)
        else {
            return;
        };
        if persisted_count == 0 {
            return;
        }
        let mut guard = self.metrics.llm_calls_per_task.lock().unwrap();
        guard.entry(task_id.to_string()).or_insert(persisted_count);
    }

    pub(crate) fn task_llm_cost_records(&self, task_id: &str) -> Vec<LlmCallCostRecord> {
        if let Ok(records) = super::llm_cost_ledger::task_records(&self.core.db, task_id) {
            if !records.is_empty() {
                return records;
            }
        }
        self.metrics
            .llm_cost_records_per_task
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
            .unwrap_or_default()
    }

    pub(crate) fn task_llm_cost_summary(&self, task_id: &str) -> LlmTaskCostSummary {
        summarize_task_cost(
            self.task_llm_call_count(task_id),
            &self.task_llm_cost_records(task_id),
        )
    }

    pub(crate) fn evaluate_llm_cost_budget(
        &self,
        task: &ClaimedTask,
        provider: Option<&str>,
    ) -> Result<LlmCostBudgetSnapshot, String> {
        let config = &self.policy.llm_cost_governance;
        if !config.enabled {
            let snapshot = LlmCostBudgetSnapshot {
                status: "disabled".to_string(),
                enforcement: "disabled".to_string(),
                provider: provider.map(str::to_string),
                ..LlmCostBudgetSnapshot::default()
            };
            self.note_task_llm_cost_budget(&task.task_id, snapshot.clone());
            return Ok(snapshot);
        }

        let now = crate::now_ts_u64().min(i64::MAX as u64) as i64;
        let since = now.saturating_sub(86_400);
        let task_spend = super::llm_cost_ledger::task_spend(&self.core.db, &task.task_id)?;
        let user_spend =
            super::llm_cost_ledger::user_spend_since(&self.core.db, task.user_id, since)?;
        let provider_spend = provider
            .map(|name| super::llm_cost_ledger::provider_spend_since(&self.core.db, name, since))
            .transpose()?;

        let soft_task = usd_to_nanos(config.soft_task_usd);
        let soft_user = usd_to_nanos(config.soft_user_24h_usd);
        let soft_provider = usd_to_nanos(config.soft_provider_24h_usd);
        let hard_task = usd_to_nanos(config.hard_task_usd);
        let mut signals = Vec::new();
        if soft_task.is_some_and(|limit| task_spend.known_cost_usd_nanos >= limit) {
            signals.push("soft_task_cost_exceeded".to_string());
        }
        if soft_user.is_some_and(|limit| user_spend.known_cost_usd_nanos >= limit) {
            signals.push("soft_user_24h_cost_exceeded".to_string());
        }
        if provider_spend
            .as_ref()
            .zip(soft_provider)
            .is_some_and(|(spend, limit)| spend.known_cost_usd_nanos >= limit)
        {
            signals.push("soft_provider_24h_cost_exceeded".to_string());
        }
        if task_spend.unknown_record_count > 0
            || user_spend.unknown_record_count > 0
            || provider_spend
                .as_ref()
                .is_some_and(|spend| spend.unknown_record_count > 0)
        {
            signals.push("unknown_cost_record_observed".to_string());
        }
        let hard_exceeded = hard_task.is_some_and(|limit| task_spend.known_cost_usd_nanos >= limit);
        if hard_exceeded {
            signals.push("hard_task_cost_exceeded".to_string());
        }
        let status = if hard_exceeded {
            "hard_exceeded"
        } else if signals.iter().any(|signal| signal.starts_with("soft_")) {
            "soft_exceeded"
        } else if signals
            .iter()
            .any(|signal| signal == "unknown_cost_record_observed")
        {
            "unknown"
        } else {
            "within_budget"
        };
        let snapshot = LlmCostBudgetSnapshot {
            status: status.to_string(),
            enforcement: if hard_task.is_some() {
                "checkpoint"
            } else {
                "observe"
            }
            .to_string(),
            provider: provider.map(str::to_string),
            task_known_cost_usd_nanos: task_spend.known_cost_usd_nanos,
            user_24h_known_cost_usd_nanos: user_spend.known_cost_usd_nanos,
            provider_24h_known_cost_usd_nanos: provider_spend
                .as_ref()
                .map(|spend| spend.known_cost_usd_nanos),
            task_unknown_record_count: task_spend.unknown_record_count,
            user_24h_unknown_record_count: user_spend.unknown_record_count,
            provider_24h_unknown_record_count: provider_spend
                .as_ref()
                .map(|spend| spend.unknown_record_count),
            soft_task_limit_usd_nanos: soft_task,
            soft_user_24h_limit_usd_nanos: soft_user,
            soft_provider_24h_limit_usd_nanos: soft_provider,
            hard_task_limit_usd_nanos: hard_task,
            hard_exceeded,
            signals,
        };
        self.note_task_llm_cost_budget(&task.task_id, snapshot.clone());
        if let Some(limit) = hard_task.filter(|_| hard_exceeded) {
            self.note_task_cost_blocker(
                &task.task_id,
                TaskCostBlocker {
                    status_code: "llm_cost_hard_ceiling".to_string(),
                    scope: "task".to_string(),
                    observed_cost_usd_nanos: task_spend.known_cost_usd_nanos,
                    limit_cost_usd_nanos: limit,
                    retry_after_seconds: config.checkpoint_retry_after_seconds.max(1),
                    message_key: "llm.cost_hard_ceiling".to_string(),
                },
            );
        } else {
            self.clear_task_cost_blocker(&task.task_id);
        }
        Ok(snapshot)
    }

    pub(crate) fn task_llm_cost_budget(&self, task_id: &str) -> Option<LlmCostBudgetSnapshot> {
        self.metrics
            .llm_cost_budget_per_task
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
    }

    pub(crate) fn task_cost_blocker(&self, task_id: &str) -> Option<TaskCostBlocker> {
        self.metrics
            .cost_blocker_per_task
            .lock()
            .unwrap()
            .get(task_id)
            .cloned()
    }

    fn note_task_llm_cost_budget(&self, task_id: &str, snapshot: LlmCostBudgetSnapshot) {
        self.metrics
            .llm_cost_budget_per_task
            .lock()
            .unwrap()
            .insert(task_id.to_string(), snapshot);
    }

    fn note_task_cost_blocker(&self, task_id: &str, blocker: TaskCostBlocker) {
        self.metrics
            .cost_blocker_per_task
            .lock()
            .unwrap()
            .insert(task_id.to_string(), blocker);
    }

    pub(super) fn clear_task_llm_cost_records(&self, task_id: &str) {
        self.metrics
            .llm_cost_records_per_task
            .lock()
            .unwrap()
            .remove(task_id);
    }

    pub(super) fn clear_task_llm_cost_budget(&self, task_id: &str) {
        self.metrics
            .llm_cost_budget_per_task
            .lock()
            .unwrap()
            .remove(task_id);
    }

    pub(crate) fn clear_task_cost_blocker(&self, task_id: &str) {
        self.metrics
            .cost_blocker_per_task
            .lock()
            .unwrap()
            .remove(task_id);
    }
}

fn usd_to_nanos(value: Option<f64>) -> Option<u64> {
    let value = value?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }
    let nanos = (value * NANOS_PER_USD).round();
    (nanos <= u64::MAX as f64).then_some(nanos as u64)
}

#[cfg(test)]
#[path = "llm_cost_tests.rs"]
mod tests;
