use anyhow::Result;
use serde_json::{json, Value};

use crate::{events::EventFilters, output, task};

use super::exec::{
    exec_exit_class, exec_summary_json, wait_for_exec_task, ExecExitClass, ExecWaitOptions,
};

const MAX_WORKSPACE_DIFF_PATCH_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy)]
pub(crate) struct CodeCapabilityOptions {
    pub(crate) detach: bool,
    pub(crate) json_output: bool,
    pub(crate) jsonl_output: bool,
    pub(crate) timeout_seconds: Option<u64>,
    pub(crate) interval_ms: u64,
    pub(crate) submission_options: task::TaskSubmissionOptions,
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
    if !options.detach {
        crate::interrupt::install()?;
    }
    let task_id =
        task::submit_capability(base_url, key, capability, args, options.submission_options)?;
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
        if capability == "workspace.diff" {
            map.insert(
                "workspace_diff".to_string(),
                workspace_diff_artifact_json(&task.raw_data).unwrap_or(Value::Null),
            );
        }
    }
    if options.json_output || options.jsonl_output {
        print_capability_output(&summary, options.json_output, options.jsonl_output)?;
    } else {
        output::print_task_status(&task, false, &EventFilters::default());
        if capability == "workspace.diff" {
            if let Some(patch) = workspace_diff_artifact_json(&task.raw_data)
                .and_then(|artifact| artifact.get("patch").cloned())
                .and_then(|patch| patch.as_str().map(str::to_string))
            {
                print!("{patch}");
                if !patch.ends_with('\n') {
                    println!();
                }
            }
        }
        println!("capability: {capability}");
        println!("capability_outcome: {}", outcome.as_str());
        println!("capability_exit_class: {}", exit_class.as_str());
        println!("capability_exit_code: {}", exit_class.code());
    }
    Ok(exit_class.code())
}

fn workspace_diff_artifact_json(data: &Value) -> Option<Value> {
    let steps = data
        .pointer("/result_json/task_journal/trace/step_results")
        .or_else(|| data.pointer("/task_journal/trace/step_results"))?
        .as_array()?;
    for step in steps.iter().rev() {
        let output = step.get("output_excerpt").and_then(Value::as_str)?;
        let output = serde_json::from_str::<Value>(output.trim()).ok()?;
        let artifact = output
            .get("extra")
            .filter(|value| value.is_object())
            .unwrap_or(&output);
        if artifact.get("source").and_then(Value::as_str) != Some("workspace_patch")
            || artifact.get("action").and_then(Value::as_str) != Some("diff")
        {
            continue;
        }
        let patch = artifact.get("patch").and_then(Value::as_str)?;
        if patch.len() > MAX_WORKSPACE_DIFF_PATCH_BYTES {
            return None;
        }
        return Some(json!({
            "schema_version": 1,
            "source": "workspace_patch",
            "action": "diff",
            "checkpoint_id": artifact.get("checkpoint_id").cloned().unwrap_or(Value::Null),
            "patch_id": artifact.get("patch_id").cloned().unwrap_or(Value::Null),
            "changed_files": artifact.get("changed_files").cloned().unwrap_or_else(|| json!([])),
            "patch_bytes": artifact.get("patch_bytes").cloned().unwrap_or_else(|| json!(patch.len())),
            "patch_truncated": artifact.get("patch_truncated").cloned().unwrap_or_else(|| json!(false)),
            "patch": patch,
        }));
    }
    None
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
