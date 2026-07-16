use anyhow::Result;
use serde_json::{json, Value};

use crate::{events::EventFilters, output, task};

use super::exec::{
    exec_exit_class, exec_summary_json, wait_for_exec_task, ExecExitClass, ExecWaitOptions,
};

#[derive(Debug, Clone, Copy)]
pub(crate) struct CodeCapabilityOptions {
    pub(crate) detach: bool,
    pub(crate) json_output: bool,
    pub(crate) jsonl_output: bool,
    pub(crate) timeout_seconds: Option<u64>,
    pub(crate) interval_ms: u64,
}

pub(crate) fn workspace_diff_args(checkpoint_id: Option<&str>, paths: &[String]) -> Value {
    let mut args = serde_json::Map::new();
    if let Some(checkpoint_id) = checkpoint_id
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        args.insert("checkpoint_id".to_string(), json!(checkpoint_id));
    }
    if !paths.is_empty() {
        args.insert("paths".to_string(), json!(paths));
    }
    Value::Object(args)
}

pub(crate) fn workspace_rewind_args(checkpoint_id: &str) -> Value {
    json!({"checkpoint_id": checkpoint_id.trim()})
}

pub(crate) fn run_code_capability(
    base_url: &str,
    key: &str,
    capability: &str,
    args: Value,
    options: CodeCapabilityOptions,
) -> Result<u8> {
    let task_id = task::submit_capability(base_url, key, capability, args)?;
    if options.detach {
        let summary = json!({
            "task_id": task_id,
            "capability": capability,
            "detached": true,
            "exit_class": ExecExitClass::Success.as_str(),
            "exit_code": ExecExitClass::Success.code(),
        });
        print_capability_output(&summary, options.json_output, options.jsonl_output)?;
        return Ok(ExecExitClass::Success.code());
    }

    let (task, outcome) = wait_for_exec_task(
        base_url,
        key,
        &task_id,
        ExecWaitOptions {
            interval_ms: options.interval_ms.max(100),
            timeout_seconds: options.timeout_seconds,
            continue_on_background: true,
            fail_on_background: false,
            json_output: options.json_output,
            jsonl_output: options.jsonl_output,
        },
    )?;
    let exit_class = exec_exit_class(&task, outcome, false);
    let mut summary = exec_summary_json(&task, outcome, exit_class, None);
    if let Some(map) = summary.as_object_mut() {
        map.insert("capability".to_string(), json!(capability));
    }
    if options.json_output || options.jsonl_output {
        print_capability_output(&summary, options.json_output, options.jsonl_output)?;
    } else {
        output::print_task_status(&task, false, &EventFilters::default());
        println!("capability: {capability}");
        println!("capability_outcome: {}", outcome.as_str());
        println!("capability_exit_class: {}", exit_class.as_str());
        println!("capability_exit_code: {}", exit_class.code());
    }
    Ok(exit_class.code())
}

fn print_capability_output(value: &Value, json_output: bool, jsonl_output: bool) -> Result<()> {
    if jsonl_output {
        println!("{}", serde_json::to_string(value)?);
    } else if json_output {
        output::print_json_pretty(value);
    } else {
        println!("task_id: {}", value["task_id"].as_str().unwrap_or_default());
    }
    Ok(())
}

#[cfg(test)]
#[path = "code_control_tests.rs"]
mod tests;
