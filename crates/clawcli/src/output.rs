use crate::{
    events::{EventFilters, TaskEventLine},
    task,
};

pub(crate) fn print_json_pretty(body: &serde_json::Value) {
    println!("{}", serde_json::to_string_pretty(body).unwrap_or_default());
}

pub(crate) fn print_task_status(
    task: &task::TaskStatusView,
    include_events: bool,
    event_filters: &EventFilters,
) {
    println!("task_id: {}", task.task_id);
    println!("status: {}", task.status);
    if let Some(state) = task.lifecycle_state() {
        println!("lifecycle_state: {state}");
    }
    let lifecycle_tokens = task.lifecycle_summary_tokens();
    if !lifecycle_tokens.is_empty() {
        println!("lifecycle: {}", lifecycle_tokens.join(" "));
    }
    if let Some(text) = task.result_text.as_deref() {
        println!("{text}");
    }
    if let Some(error_text) = task.error_text.as_deref() {
        eprintln!("error: {error_text}");
    }
    if include_events {
        for line in filtered_event_lines(task, event_filters) {
            println!("{line}");
        }
    }
}

pub(crate) fn filtered_event_lines(
    task: &task::TaskStatusView,
    event_filters: &EventFilters,
) -> Vec<String> {
    filtered_events(task, event_filters)
        .into_iter()
        .map(|event| format!("event: {}", event.line))
        .collect()
}

pub(crate) fn filtered_events<'a>(
    task: &'a task::TaskStatusView,
    event_filters: &EventFilters,
) -> Vec<&'a TaskEventLine> {
    task.events
        .iter()
        .filter(|event| event_filters.matches(event))
        .collect()
}

pub(crate) fn print_active_task_table(body: &serde_json::Value) {
    let tasks = body
        .pointer("/data/tasks")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    println!(
        "{:<5} {:<36} {:<10} {:<12} {:<8} summary",
        "idx", "task_id", "status", "lifecycle", "age_s"
    );
    for task in tasks {
        let index = value_token(task.get("index"));
        let task_id = value_token(task.get("task_id"));
        let status = value_token(task.get("status"));
        let lifecycle = task
            .get("lifecycle")
            .and_then(|value| value.get("state"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let age_seconds = value_token(task.get("age_seconds"));
        let summary = truncate_display_token(&value_token(task.get("summary")), 80);
        println!(
            "{:<5} {:<36} {:<10} {:<12} {:<8} {}",
            index, task_id, status, lifecycle, age_seconds, summary
        );
    }
}

pub(crate) fn print_automation_run_table(body: &serde_json::Value) {
    let runs = body
        .pointer("/data/runs")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    println!(
        "{:<34} {:<16} {:<36} {:<10} {:<12} findings",
        "run_id", "job_id", "task_id", "status", "triage"
    );
    for run in runs {
        let run_id = truncate_display_token(&value_token(run.get("run_id")), 34);
        let job_id = truncate_display_token(&value_token(run.get("job_id")), 16);
        let task_id = value_token(run.get("task_id"));
        let status = value_token(run.get("task_status"));
        let triage = value_token(run.get("triage_status"));
        let findings = truncate_display_token(&join_string_array(run.get("finding_refs")), 60);
        println!(
            "{:<34} {:<16} {:<36} {:<10} {:<12} {}",
            run_id, job_id, task_id, status, triage, findings
        );
    }
}

pub(crate) fn print_skill_table(body: &serde_json::Value) {
    let items = skill_items(body);
    println!(
        "{:<30} {:<10} {:<12} {:<22} {:<5} {:<8} {:<8} {:<22} description",
        "skill", "kind", "planner", "adapter", "bg", "risk", "available", "reason"
    );
    for item in items {
        let name = value_token(item.get("name"));
        let kind = value_token(item.get("kind"));
        let planner_kind = value_token(item.get("planner_kind"));
        let adapter = value_token(item.get("adapter_category"));
        let background_job = value_token(item.get("background_job_capable"));
        let risk = value_token(item.get("risk_level"));
        let available = value_token(item.get("runtime_available"));
        let unavailable_reason = value_token(item.get("unavailable_reason"));
        let description = truncate_display_token(&value_token(item.get("description")), 80);
        println!(
            "{:<30} {:<10} {:<12} {:<22} {:<5} {:<8} {:<8} {:<22} {}",
            name,
            kind,
            planner_kind,
            adapter,
            background_job,
            risk,
            available,
            unavailable_reason,
            description
        );
    }
}

pub(crate) fn print_capability_table(body: &serde_json::Value) {
    let items = skill_items(body);
    println!(
        "{:<30} {:<40} {:<30} {:<22} {:<8} {:<8} reason",
        "skill", "planner_capabilities", "capabilities", "isolation", "risk", "available"
    );
    for item in items {
        let planner_capabilities = join_string_array(item.get("planner_capabilities"));
        let capabilities = join_string_array(item.get("capabilities"));
        if planner_capabilities.is_empty() && capabilities.is_empty() {
            continue;
        }
        let name = value_token(item.get("name"));
        let isolation_profile = capability_isolation_summary(item);
        let risk = value_token(item.get("risk_level"));
        let available = value_token(item.get("runtime_available"));
        let unavailable_reason = value_token(item.get("unavailable_reason"));
        println!(
            "{:<30} {:<40} {:<30} {:<22} {:<8} {:<8} {}",
            name,
            truncate_display_token(&planner_capabilities, 40),
            truncate_display_token(&capabilities, 30),
            truncate_display_token(&isolation_profile, 22),
            risk,
            available,
            unavailable_reason
        );
    }
}

fn capability_isolation_summary(item: &serde_json::Value) -> String {
    let Some(policies) = item
        .get("planner_capability_policies")
        .and_then(serde_json::Value::as_array)
    else {
        return value_token(item.get("isolation_profile"));
    };
    let mut profiles = policies
        .iter()
        .filter_map(|policy| policy.get("isolation_profile"))
        .filter_map(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    profiles.sort();
    profiles.dedup();
    profiles.join(",")
}

fn skill_items(body: &serde_json::Value) -> &[serde_json::Value] {
    body.pointer("/data/skill_items")
        .and_then(serde_json::Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn join_string_array(value: Option<&serde_json::Value>) -> String {
    value
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}

fn value_token(value: Option<&serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(value)) => value.trim().to_string(),
        Some(serde_json::Value::Number(value)) => value.to_string(),
        Some(serde_json::Value::Bool(value)) => value.to_string(),
        Some(
            serde_json::Value::Null | serde_json::Value::Array(_) | serde_json::Value::Object(_),
        )
        | None => String::new(),
    }
}

fn truncate_display_token(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
