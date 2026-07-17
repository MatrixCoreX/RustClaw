use super::AppState;
use crate::providers::{summarize_task_cost, LlmCallCostRecord, LlmTaskCostSummary};

const MAX_LLM_COST_RECORDS_PER_TASK: usize = 512;

impl AppState {
    pub(crate) fn note_task_llm_cost_record(&self, task_id: &str, record: LlmCallCostRecord) {
        let mut guard = self.metrics.llm_cost_records_per_task.lock().unwrap();
        let records = guard.entry(task_id.to_string()).or_default();
        if records.len() < MAX_LLM_COST_RECORDS_PER_TASK {
            records.push(record);
        }
    }

    pub(crate) fn task_llm_cost_records(&self, task_id: &str) -> Vec<LlmCallCostRecord> {
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

    pub(super) fn clear_task_llm_cost_records(&self, task_id: &str) {
        self.metrics
            .llm_cost_records_per_task
            .lock()
            .unwrap()
            .remove(task_id);
    }
}
