//! Metadata projections must not inflate provider bodies for usage summaries.
use super::*;

#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct UsageLogEntry {
    pub(super) task_id: Option<String>,
    ts: Option<u64>,
    vendor: Option<String>,
    provider: Option<String>,
    provider_type: Option<String>,
    model: Option<String>,
    model_kind: Option<String>,
    status: Option<String>,
    prompt_file: Option<String>,
    error: Option<String>,
    usage: Option<TaskDebugUsage>,
}

impl From<&TaskDebugEntry> for UsageLogEntry {
    fn from(entry: &TaskDebugEntry) -> Self {
        Self {
            task_id: entry.task_id.clone(),
            ts: entry.ts,
            vendor: entry.vendor.clone(),
            provider: entry.provider.clone(),
            provider_type: entry.provider_type.clone(),
            model: entry.model.clone(),
            model_kind: entry.model_kind.clone(),
            status: entry.status.clone(),
            prompt_file: entry.prompt_file.clone(),
            error: entry.error.clone(),
            usage: entry.usage.clone(),
        }
    }
}

#[derive(Default)]
pub(super) struct UsageTaskAccumulator {
    latest: UsageLogEntry,
    call_count: usize,
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
}

impl UsageTaskAccumulator {
    pub(super) fn add(&mut self, entry: UsageLogEntry) {
        let prompt = entry
            .usage
            .as_ref()
            .and_then(|u| u.prompt_tokens.or(u.input_tokens))
            .unwrap_or(0);
        let completion = entry
            .usage
            .as_ref()
            .and_then(|u| u.completion_tokens.or(u.output_tokens))
            .unwrap_or(0);
        let total = entry
            .usage
            .as_ref()
            .and_then(|u| u.total_tokens)
            .unwrap_or_else(|| prompt.saturating_add(completion));
        self.prompt_tokens = self.prompt_tokens.saturating_add(prompt);
        self.completion_tokens = self.completion_tokens.saturating_add(completion);
        self.total_tokens = self.total_tokens.saturating_add(total);
        if self.call_count == 0 || entry.ts.unwrap_or(0) >= self.latest.ts.unwrap_or(0) {
            self.latest = entry;
        }
        self.call_count = self.call_count.saturating_add(1);
    }

    pub(super) fn into_summary(
        self,
        task_id: String,
        meta: UsageTaskMeta,
    ) -> UsageHistoryRecordSummary {
        UsageHistoryRecordSummary {
            record_id: task_id.clone(),
            task_id,
            ts: self.latest.ts,
            channel: Some(meta.channel),
            kind: Some(meta.kind),
            task_status: Some(meta.task_status),
            telegram_bot_name: meta.telegram_bot_name,
            external_user_id: meta.external_user_id,
            external_chat_id: meta.external_chat_id,
            request_text: meta.request_text,
            vendor: self.latest.vendor,
            provider: self.latest.provider,
            provider_type: self.latest.provider_type,
            model: self.latest.model,
            model_kind: self.latest.model_kind,
            prompt_file: self.latest.prompt_file,
            prompt_tokens: Some(self.prompt_tokens),
            completion_tokens: Some(self.completion_tokens),
            total_tokens: Some(self.total_tokens),
            llm_call_count: self.call_count,
            status: self.latest.status,
            error: self.latest.error,
        }
    }
}

pub(super) fn summarize_usage_task(
    task_id: String,
    meta: UsageTaskMeta,
    entries: &[TaskDebugEntry],
) -> UsageHistoryRecordSummary {
    let mut summary = UsageTaskAccumulator::default();
    for entry in entries {
        summary.add(UsageLogEntry::from(entry));
    }
    summary.into_summary(task_id, meta)
}

pub(super) fn line_matches_task_ids(
    line: &str,
    task_ids: &std::collections::HashSet<String>,
) -> bool {
    #[derive(Deserialize)]
    struct Selector<'a> {
        #[serde(borrow)]
        task_id: Option<std::borrow::Cow<'a, str>>,
    }
    serde_json::from_str::<Selector<'_>>(line)
        .ok()
        .and_then(|entry| entry.task_id.map(|id| task_ids.contains(id.as_ref())))
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "model_log_scan_tests.rs"]
mod tests;
