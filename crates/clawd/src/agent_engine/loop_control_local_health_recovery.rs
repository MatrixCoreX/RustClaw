use super::*;

#[derive(Debug, Clone, PartialEq, Eq)]
struct LocalHealthFinding {
    fields: std::collections::BTreeMap<String, String>,
}

impl LocalHealthFinding {
    fn new() -> Self {
        Self {
            fields: std::collections::BTreeMap::new(),
        }
    }

    fn insert(&mut self, key: &str, value: impl ToString) {
        let value = value.to_string();
        let value = value.trim();
        if value.is_empty() {
            return;
        }
        self.fields
            .entry(key.to_string())
            .or_insert_with(|| value.to_string());
    }

    fn has_health_signal(&self) -> bool {
        self.fields.contains_key("disk_root_use_percent")
            || self.fields.contains_key("disk_root_available")
            || self.fields.contains_key("disk_root_available_bytes")
            || self.fields.contains_key("memory_total")
            || self.fields.contains_key("memory_total_bytes")
            || self.fields.contains_key("clawd_process_count")
            || self.fields.contains_key("clawd_status")
            || self.fields.contains_key("system_warnings")
    }
}

pub(super) fn try_recover_local_health_answer_verifier_gap(
    route_result: Option<&crate::RouteResult>,
    reply: &mut AskReply,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route_allows_local_health_recovery(route) {
        return false;
    }
    let Some(verifier) = reply
        .task_journal
        .as_ref()
        .and_then(|journal| journal.answer_verifier_summary.as_ref())
    else {
        return false;
    };
    if !verifier.high_confidence_retry_gap()
        || !verifier
            .missing_evidence_fields
            .iter()
            .any(|field| matches!(field.as_str(), "command_output" | "field_value"))
    {
        return false;
    }
    let Some(finding) = observed_local_health_finding(reply) else {
        return false;
    };
    let answer = deterministic_local_health_status_line(&finding);
    if let Some(journal) = reply.task_journal.as_mut() {
        journal.answer_verifier_summary = None;
        journal.record_final_answer(&answer);
        journal.record_final_status(crate::task_journal::TaskJournalFinalStatus::Success);
        journal.record_final_stop_signal(
            crate::task_journal::ANSWER_VERIFIER_RECOVERED_TERMINAL_STOP_SIGNAL,
        );
    }
    reply.text = answer.clone();
    reply.messages = vec![answer];
    reply.should_fail_task = false;
    reply.error_text = None;
    reply.is_llm_reply = false;
    info!("answer_verifier_retry_exhausted_recovered_with_local_health_fields");
    true
}

fn route_allows_local_health_recovery(route: &crate::RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && !route.output_contract.delivery_required
        && matches!(
            route.effective_output_contract_semantic_kind(),
            crate::OutputSemanticKind::CommandOutputSummary
                | crate::OutputSemanticKind::ServiceStatus
        )
}

fn observed_local_health_finding(reply: &AskReply) -> Option<LocalHealthFinding> {
    let mut finding = LocalHealthFinding::new();
    let journal = reply.task_journal.as_ref()?;
    for step in &journal.step_results {
        if step.status != crate::executor::StepExecutionStatus::Ok {
            continue;
        }
        let Some(output) = step.output_excerpt.as_deref() else {
            continue;
        };
        collect_local_health_fields_from_output(output, &mut finding);
    }
    finding.has_health_signal().then_some(finding)
}

fn collect_local_health_fields_from_output(output: &str, finding: &mut LocalHealthFinding) {
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        collect_local_health_fields_from_json(&value, finding);
        return;
    }
    collect_local_health_fields_from_command_output(output, finding);
}

fn collect_local_health_fields_from_json(value: &Value, finding: &mut LocalHealthFinding) {
    let payload = value.get("extra").unwrap_or(value);
    collect_health_check_payload_fields(payload, finding);
    collect_process_basic_payload_fields(payload, finding);
}

fn collect_health_check_payload_fields(payload: &Value, finding: &mut LocalHealthFinding) {
    if let Some(count) = payload.get("clawd_process_count").and_then(Value::as_u64) {
        finding.insert("clawd_process_count", count);
    }
    if let Some(count) = payload
        .get("telegramd_process_count")
        .and_then(Value::as_u64)
    {
        finding.insert("telegramd_process_count", count);
    }
    if let Some(open) = payload
        .get("clawd_health_port_open")
        .and_then(Value::as_bool)
    {
        finding.insert("clawd_health_port_open", open);
    }
    if let Some(log) = payload.get("clawd_log") {
        collect_log_fields("clawd_log", log, finding);
    }
    if let Some(log) = payload.get("nni_log") {
        collect_log_fields("nni_log", log, finding);
    }
    if let Some(log) = payload.get("nni_server_log") {
        collect_log_fields("nni_server_log", log, finding);
    }
    if let Some(log) = payload.get("telegramd_log") {
        collect_log_fields("telegramd_log", log, finding);
    }
    let Some(system_health) = payload.get("system_health") else {
        return;
    };
    for key in [
        "os_family",
        "service_manager",
        "cpu_count",
        "uptime_seconds",
        "load_avg_1m",
        "load_avg_5m",
        "load_avg_15m",
        "memory_total_bytes",
        "memory_available_bytes",
        "disk_root_total_bytes",
        "disk_root_available_bytes",
    ] {
        if let Some(value) = scalar_value_as_token(system_health.get(key)) {
            finding.insert(key, value);
        }
    }
    if let (Some(total), Some(available)) = (
        system_health
            .get("disk_root_total_bytes")
            .and_then(Value::as_u64),
        system_health
            .get("disk_root_available_bytes")
            .and_then(Value::as_u64),
    ) {
        insert_used_and_percent(
            finding,
            "disk_root_used_bytes",
            "disk_root_use_percent",
            total,
            available,
        );
    }
    if let (Some(total), Some(available)) = (
        system_health
            .get("memory_total_bytes")
            .and_then(Value::as_u64),
        system_health
            .get("memory_available_bytes")
            .and_then(Value::as_u64),
    ) {
        insert_used_and_percent(
            finding,
            "memory_used_bytes",
            "memory_used_percent",
            total,
            available,
        );
    }
    if let Some(warnings) = system_health.get("warnings").and_then(Value::as_array) {
        let joined = warnings
            .iter()
            .filter_map(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(",");
        finding.insert("system_warnings", joined);
    }
}

fn collect_process_basic_payload_fields(payload: &Value, finding: &mut LocalHealthFinding) {
    if payload.get("action").and_then(Value::as_str) != Some("ps") {
        return;
    }
    let filter = payload
        .get("filter")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("process");
    if let Some(status) = payload.get("status").and_then(Value::as_str) {
        finding.insert(&format!("{filter}_status"), status);
    }
    if let Some(count) = payload.get("process_count").and_then(Value::as_u64) {
        finding.insert(&format!("{filter}_process_count"), count);
    }
    if let Some(running) = payload.get("running").and_then(Value::as_bool) {
        finding.insert(&format!("{filter}_running"), running);
    }
}

fn collect_log_fields(prefix: &str, value: &Value, finding: &mut LocalHealthFinding) {
    for key in ["exists", "keyword_error_count", "size_bytes", "modified_ts"] {
        if let Some(token) = scalar_value_as_token(value.get(key)) {
            finding.insert(&format!("{prefix}_{key}"), token);
        }
    }
}

fn collect_local_health_fields_from_command_output(
    command_output: &str,
    finding: &mut LocalHealthFinding,
) {
    for line in command_output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        collect_df_root_fields(line, finding);
        collect_free_fields(line, finding);
        collect_uptime_load_fields(line, finding);
        collect_lscpu_fields(line, finding);
    }
}

fn collect_df_root_fields(line: &str, finding: &mut LocalHealthFinding) {
    let cols = line.split_whitespace().collect::<Vec<_>>();
    if cols.len() < 6 || cols.last().copied() != Some("/") || !cols[4].ends_with('%') {
        return;
    }
    finding.insert("disk_root_source", cols[0]);
    finding.insert("disk_root_size", cols[1]);
    finding.insert("disk_root_used", cols[2]);
    finding.insert("disk_root_available", cols[3]);
    finding.insert("disk_root_use_percent", cols[4]);
}

fn collect_free_fields(line: &str, finding: &mut LocalHealthFinding) {
    let cols = line.split_whitespace().collect::<Vec<_>>();
    match cols.as_slice() {
        ["Mem:", total, used, free, _shared, _buff_cache, available, ..] => {
            finding.insert("memory_total", total);
            finding.insert("memory_used", used);
            finding.insert("memory_free", free);
            finding.insert("memory_available", available);
        }
        ["Swap:", total, used, free, ..] => {
            finding.insert("swap_total", total);
            finding.insert("swap_used", used);
            finding.insert("swap_free", free);
        }
        _ => {}
    }
}

fn collect_uptime_load_fields(line: &str, finding: &mut LocalHealthFinding) {
    let Some((_, load_values)) = line.split_once("load average:") else {
        return;
    };
    let loads = load_values
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    if let Some(value) = loads.first() {
        finding.insert("load_avg_1m", value);
    }
    if let Some(value) = loads.get(1) {
        finding.insert("load_avg_5m", value);
    }
    if let Some(value) = loads.get(2) {
        finding.insert("load_avg_15m", value);
    }
}

fn collect_lscpu_fields(line: &str, finding: &mut LocalHealthFinding) {
    let Some((key, value)) = line.split_once(':') else {
        return;
    };
    let key = key.trim();
    let value = value.trim();
    if key == "CPU(s)" && value.chars().all(|ch| ch.is_ascii_digit()) {
        finding.insert("cpu_count", value);
    } else if key == "Model name" {
        finding.insert("cpu_model", value);
    }
}

fn scalar_value_as_token(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        }
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

fn insert_used_and_percent(
    finding: &mut LocalHealthFinding,
    used_key: &str,
    percent_key: &str,
    total: u64,
    available: u64,
) {
    if total == 0 || available > total {
        return;
    }
    let used = total - available;
    finding.insert(used_key, used);
    let percent = (used as f64 / total as f64) * 100.0;
    finding.insert(percent_key, trim_decimal_zeros(format!("{percent:.1}")));
}

fn trim_decimal_zeros(mut value: String) -> String {
    if value.ends_with(".0") {
        value.truncate(value.len() - 2);
    }
    value
}

fn deterministic_local_health_status_line(finding: &LocalHealthFinding) -> String {
    let priority = [
        "disk_root_use_percent",
        "disk_root_available",
        "disk_root_used",
        "disk_root_size",
        "disk_root_available_bytes",
        "disk_root_used_bytes",
        "disk_root_total_bytes",
        "memory_total",
        "memory_used",
        "memory_available",
        "memory_total_bytes",
        "memory_used_bytes",
        "memory_available_bytes",
        "swap_total",
        "swap_used",
        "load_avg_1m",
        "load_avg_5m",
        "load_avg_15m",
        "clawd_status",
        "clawd_process_count",
        "clawd_health_port_open",
        "telegramd_process_count",
        "system_warnings",
    ];
    let mut parts = Vec::new();
    let mut emitted = std::collections::BTreeSet::new();
    for key in priority {
        if let Some(value) = finding.fields.get(key) {
            parts.push(format!("{key}={value}"));
            emitted.insert(key);
        }
    }
    for (key, value) in &finding.fields {
        if !emitted.contains(key.as_str()) {
            parts.push(format!("{key}={value}"));
        }
    }
    parts.join(" ")
}
