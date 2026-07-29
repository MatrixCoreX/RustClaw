#[cfg(test)]
#[allow(dead_code)]
mod test_support;

use office_workspace::{execute, SkillRequest, SkillResponse};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let response = match line {
            Ok(line) => process_line(&line),
            Err(error) => error_response(
                "unknown",
                "stdin_read_failed",
                format!("cannot read request: {error}"),
                json!({}),
            ),
        };
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}

fn process_line(line: &str) -> SkillResponse {
    let request = match serde_json::from_str::<SkillRequest>(line) {
        Ok(request) => request,
        Err(error) => {
            return error_response(
                "unknown",
                "invalid_request",
                format!("invalid skill request JSON: {error}"),
                json!({}),
            )
        }
    };
    let request_id = request.request_id;
    match execute(&request.args) {
        Ok(mut extra) => {
            attach_model_observation(&request.args, &mut extra);
            SkillResponse {
                request_id,
                status: "ok".to_string(),
                text: compact_text(&extra),
                error_text: None,
                extra,
            }
        }
        Err(error) => SkillResponse {
            request_id,
            status: "error".to_string(),
            text: String::new(),
            error_text: Some(error.message.clone()),
            extra: error.extra(),
        },
    }
}

fn attach_model_observation(args: &Value, extra: &mut Value) {
    let Some(extra_object) = extra.as_object_mut() else {
        return;
    };
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("office.inspect");
    let mut observation = serde_json::Map::new();
    observation.insert("schema_version".to_string(), json!(1));
    observation.insert("action".to_string(), json!(action));
    for field in [
        "format",
        "source",
        "cursor",
        "truncated",
        "validation",
        "document_blocks",
        "tables",
        "workbook",
        "presentation",
        "operation_log",
        "artifacts",
        "changed_object_refs",
        "continuation",
        "preservation_report",
        "preview",
        "render",
        "status",
    ] {
        if let Some(value) = extra_object.get(field) {
            observation.insert(field.to_string(), value.clone());
        }
    }
    extra_object.insert("model_observation".to_string(), Value::Object(observation));
}

fn compact_text(extra: &Value) -> String {
    let mut summary = serde_json::Map::new();
    for field in [
        "schema_version",
        "format",
        "source",
        "cursor",
        "validation",
        "preview",
        "writes_performed",
        "normalized_operations",
    ] {
        if let Some(value) = extra.get(field).filter(|value| !value.is_null()) {
            summary.insert(field.to_string(), value.clone());
        }
    }
    Value::Object(summary).to_string()
}

fn error_response(
    request_id: &str,
    error_code: &str,
    error_text: String,
    details: Value,
) -> SkillResponse {
    SkillResponse {
        request_id: request_id.to_string(),
        status: "error".to_string(),
        text: String::new(),
        error_text: Some(error_text),
        extra: json!({
            "schema_version": 1,
            "source_skill": "office_workspace",
            "status": "error",
            "error_code": error_code,
            "message_key": format!("skill.office_workspace.{error_code}"),
            "retryable": false,
            "details": details,
        }),
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
