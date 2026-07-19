use anyhow::{Context, Result};
use serde_json::Value;

use crate::{client, output};

const MODEL_READINESS_SCALAR_FIELDS: &[&str] = &[
    "schema_version",
    "selected_provider",
    "selected_model",
    "selected_entry_status",
    "entry_count",
    "matched_entry_count",
    "credential_state",
];

const MODEL_READINESS_BOOL_FIELDS: &[&str] = &[
    "ready",
    "text_generation",
    "image_input",
    "image_understanding",
    "image_generation",
    "audio_input",
    "audio_transcription",
    "audio_generation",
    "video_input",
    "video_generation",
    "music_generation",
    "async_required",
    "dry_run",
];

const MODEL_READINESS_LINE_TOKEN: &str = "llm_trace_model_readiness:";

pub(crate) fn run_llm_trace(
    base_url: &str,
    key: &str,
    task_id: &str,
    json_output: bool,
    raw_output: bool,
    limit: Option<usize>,
) -> Result<()> {
    let debug = fetch_task_llm_trace(base_url, key, task_id)?;
    if json_output {
        output::print_json_pretty(&debug);
        return Ok(());
    }
    for line in llm_trace_text_lines(&debug, raw_output, limit) {
        println!("{line}");
    }
    Ok(())
}

fn fetch_task_llm_trace(base_url: &str, key: &str, task_id: &str) -> Result<Value> {
    let task_id = task_id.trim();
    if task_id.is_empty() {
        anyhow::bail!("llm_trace_task_id_required");
    }
    let url = task_llm_trace_url(base_url, task_id)?;
    let resp = client::make_client()?
        .get(url)
        .header("x-rustclaw-key", key)
        .send()
        .context("llm_trace_request_failed")?;
    let status = resp.status();
    let body: Value = resp.json().context("llm_trace_parse_response_failed")?;
    if !status.is_success() || body.get("ok").and_then(Value::as_bool) != Some(true) {
        let error = body
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("unknown_error");
        anyhow::bail!("llm_trace_fetch_failed status={} error={}", status, error);
    }
    Ok(body.get("data").cloned().unwrap_or(Value::Null))
}

pub(super) fn task_llm_trace_url(base_url: &str, task_id: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(&format!(
        "{}/debug/tasks/{}",
        client::base_v1(base_url),
        task_id
    ))
    .context("llm_trace_url_invalid")?;
    url.query_pairs_mut().append_pair("teaching", "true");
    Ok(url)
}

pub(super) fn llm_trace_text_lines(
    debug: &Value,
    raw_output: bool,
    limit: Option<usize>,
) -> Vec<String> {
    let mut lines = Vec::new();
    push_value_line(&mut lines, "llm_trace_task_id", debug.get("task_id"));
    push_first_value_line(
        &mut lines,
        "llm_trace_goal_id",
        debug,
        &[
            "/goal_id",
            "/goal/goal_id",
            "/task_goal/goal_id",
            "/result_json/task_goal/goal_id",
            "/result_json/task_journal/summary/task_goal/goal_id",
        ],
    );
    if let Some(session_id) = trace_session_id(debug) {
        lines.push(format!("llm_trace_session_id={session_id}"));
    }
    push_value_line(&mut lines, "llm_trace_call_count", debug.get("call_count"));
    if let Some(summary) = debug.get("flow_summary") {
        push_value_line(
            &mut lines,
            "llm_trace_flow_stage_count",
            summary.get("stage_count"),
        );
        push_value_line(
            &mut lines,
            "llm_trace_retry_count",
            summary.get("retry_count"),
        );
        push_value_line(
            &mut lines,
            "llm_trace_verifier_call_count",
            summary.get("verifier_call_count"),
        );
        push_value_line(
            &mut lines,
            "llm_trace_finalizer_call_count",
            summary.get("finalizer_call_count"),
        );
        push_value_line(
            &mut lines,
            "llm_trace_provider_error_count",
            summary.get("provider_error_count"),
        );
    }
    if let Some(line) = llm_trace_model_readiness_line(debug) {
        lines.push(line);
    }

    let call_limit = limit.unwrap_or(usize::MAX);
    for (fallback_index, call) in debug_calls(debug).into_iter().take(call_limit).enumerate() {
        let call_index = call
            .get("call_index")
            .and_then(Value::as_u64)
            .map(|value| value.max(1) as usize)
            .unwrap_or(fallback_index + 1);
        lines.push(llm_call_summary_line(call, call_index));
        if raw_output {
            push_raw_field_line(
                &mut lines,
                call_index,
                "request_payload",
                call.get("request_payload"),
            );
            push_raw_field_line(&mut lines, call_index, "response", call.get("response"));
            push_raw_field_line(
                &mut lines,
                call_index,
                "clean_response",
                call.get("clean_response"),
            );
            push_raw_field_line(
                &mut lines,
                call_index,
                "raw_response",
                call.get("raw_response"),
            );
            push_raw_field_line(&mut lines, call_index, "error", call.get("error"));
        }
    }
    lines
}

fn llm_trace_model_readiness_line(debug: &Value) -> Option<String> {
    let readiness = debug.pointer("/model_catalog_trace/readiness")?;
    if !readiness.is_object() {
        return None;
    }
    let mut tokens = vec!["trace_ref=model_catalog_trace.readiness".to_string()];
    for key in MODEL_READINESS_SCALAR_FIELDS
        .iter()
        .chain(MODEL_READINESS_BOOL_FIELDS.iter())
    {
        push_token(&mut tokens, key, readiness.get(*key));
    }
    if tokens.len() <= 1 {
        return None;
    }
    let mut line = String::from(MODEL_READINESS_LINE_TOKEN);
    line.push(' ');
    line.push_str(&tokens.join(" "));
    Some(line)
}

fn debug_calls(debug: &Value) -> Vec<&Value> {
    debug
        .get("calls")
        .and_then(Value::as_array)
        .filter(|calls| !calls.is_empty())
        .or_else(|| debug.get("entries").and_then(Value::as_array))
        .map(|calls| calls.iter().collect())
        .unwrap_or_default()
}

fn llm_call_summary_line(call: &Value, call_index: usize) -> String {
    let flow = call.get("flow").unwrap_or(&Value::Null);
    let mut tokens = vec![
        format!("llm_call_ref=LLM#{call_index}"),
        format!("index={call_index}"),
    ];
    push_token(&mut tokens, "status", call.get("status"));
    push_token(&mut tokens, "vendor", call.get("vendor"));
    push_token(&mut tokens, "provider", call.get("provider"));
    push_token(&mut tokens, "provider_type", call.get("provider_type"));
    push_token(&mut tokens, "model", call.get("model"));
    push_token(&mut tokens, "model_kind", call.get("model_kind"));
    push_token(&mut tokens, "prompt_label", flow.get("prompt_label"));
    push_token(&mut tokens, "flow_stage", flow.get("flow_stage"));
    push_token(&mut tokens, "flow_node", flow.get("flow_node"));
    push_token(&mut tokens, "code_module", flow.get("code_module"));
    push_token(&mut tokens, "code_entrypoint", flow.get("code_entrypoint"));
    push_token(&mut tokens, "trigger_kind", flow.get("trigger_kind"));
    if let Some(usage) = call.get("usage") {
        push_token(&mut tokens, "prompt_tokens", usage.get("prompt_tokens"));
        push_token(
            &mut tokens,
            "completion_tokens",
            usage.get("completion_tokens"),
        );
        push_token(&mut tokens, "total_tokens", usage.get("total_tokens"));
    }
    format!("llm_trace_call: {}", tokens.join(" "))
}

fn push_raw_field_line(
    lines: &mut Vec<String>,
    call_index: usize,
    field: &str,
    value: Option<&Value>,
) {
    let Some(value) = value else {
        return;
    };
    if value.is_null() {
        return;
    }
    let rendered = match value {
        Value::String(value) => value.clone(),
        _ => serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()),
    };
    if rendered.trim().is_empty() {
        return;
    }
    lines.push(format!("llm_{field}_{call_index}:"));
    lines.push(rendered);
}

fn push_value_line(lines: &mut Vec<String>, key: &str, value: Option<&Value>) {
    let Some(value) = compact_value(value) else {
        return;
    };
    lines.push(format!("{key}: {value}"));
}

fn push_first_value_line(lines: &mut Vec<String>, key: &str, value: &Value, pointers: &[&str]) {
    for pointer in pointers {
        let before = lines.len();
        push_value_line(lines, key, value.pointer(pointer));
        if lines.len() != before {
            return;
        }
    }
}

fn trace_session_id(debug: &Value) -> Option<String> {
    compact_value(
        debug
            .get("session_id")
            .or_else(|| debug.pointer("/session/session_id")),
    )
    .or_else(|| {
        let user_id = compact_value(debug.get("user_id"))?;
        let chat_id = compact_value(debug.get("chat_id"))?;
        Some(format!("user_chat:{user_id}:{chat_id}"))
    })
}

fn push_token(tokens: &mut Vec<String>, key: &str, value: Option<&Value>) {
    let Some(value) = compact_value(value) else {
        return;
    };
    tokens.push(format!("{key}={value}"));
}

fn compact_value(value: Option<&Value>) -> Option<String> {
    let value = value?;
    let text = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => String::new(),
    };
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}
