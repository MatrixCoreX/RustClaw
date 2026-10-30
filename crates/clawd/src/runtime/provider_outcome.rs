use claw_core::provider_failure_policy::ProviderFailureClass;

use super::{AppState, TaskProviderBlocker};

#[derive(Debug, Clone)]
pub(crate) enum TaskProviderOutcome {
    Waiting(TaskProviderBlocker),
    QuotaExhausted { provider: String },
}

impl AppState {
    pub(crate) fn note_task_provider_blocker(&self, task_id: &str, blocker: TaskProviderBlocker) {
        if blocker.external_provider_blocked
            && ProviderFailureClass::from_str(&blocker.status_code)
                == Some(ProviderFailureClass::QuotaExhausted)
        {
            self.note_task_provider_quota_exhausted(task_id, blocker.provider);
            return;
        }
        let mut outcomes = self.metrics.provider_outcome_per_task.lock().unwrap();
        if !matches!(
            outcomes.get(task_id),
            Some(TaskProviderOutcome::QuotaExhausted { .. })
        ) {
            outcomes.insert(task_id.to_string(), TaskProviderOutcome::Waiting(blocker));
        }
    }

    pub(crate) fn task_provider_blocker(&self, task_id: &str) -> Option<TaskProviderBlocker> {
        match self
            .metrics
            .provider_outcome_per_task
            .lock()
            .unwrap()
            .get(task_id)
        {
            Some(TaskProviderOutcome::Waiting(blocker)) => Some(blocker.clone()),
            _ => None,
        }
    }

    pub(crate) fn note_task_provider_quota_exhausted(&self, task_id: &str, provider: String) {
        self.metrics
            .provider_outcome_per_task
            .lock()
            .unwrap()
            .insert(
                task_id.to_string(),
                TaskProviderOutcome::QuotaExhausted { provider },
            );
    }

    pub(crate) fn task_provider_quota_exhausted(&self, task_id: &str) -> Option<String> {
        match self
            .metrics
            .provider_outcome_per_task
            .lock()
            .unwrap()
            .get(task_id)
        {
            Some(TaskProviderOutcome::QuotaExhausted { provider }) => Some(provider.clone()),
            _ => None,
        }
    }

    pub(crate) fn clear_task_provider_blocker(&self, task_id: &str) {
        let mut outcomes = self.metrics.provider_outcome_per_task.lock().unwrap();
        if matches!(outcomes.get(task_id), Some(TaskProviderOutcome::Waiting(_))) {
            outcomes.remove(task_id);
        }
    }

    pub(crate) fn clear_task_provider_outcome(&self, task_id: &str) {
        self.metrics
            .provider_outcome_per_task
            .lock()
            .unwrap()
            .remove(task_id);
    }
}

#[cfg(test)]
#[path = "provider_outcome_tests.rs"]
mod tests;
