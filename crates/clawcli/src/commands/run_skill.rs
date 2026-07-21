use anyhow::{Context, Result};
use serde_json::json;
use std::path::PathBuf;

use crate::{events::EventFilters, output, task};

use super::common::wait_for_terminal_task;

pub(crate) fn run_skill(
    base_url: &str,
    key: &str,
    skill_name: &str,
    args_json: Option<&str>,
    args_file: Option<&PathBuf>,
    wait: bool,
    json_output: bool,
    interval_ms: u64,
    submission_options: task::TaskSubmissionOptions,
) -> Result<()> {
    let args = parse_run_skill_args(args_json, args_file)?;
    let task_id = task::submit_run_skill(base_url, key, skill_name, args, submission_options)?;
    if wait {
        let task = wait_for_terminal_task(base_url, key, &task_id, interval_ms)?;
        if json_output {
            output::print_json_pretty(&task.raw_data);
        } else {
            output::print_task_status(&task, false, &EventFilters::default());
        }
    } else if json_output {
        output::print_json_pretty(&json!({
            "task_id": task_id,
            "kind": "run_skill",
            "skill_name": skill_name,
            "detached": true,
        }));
    } else {
        println!("task_id: {}", task_id);
    }
    Ok(())
}

fn parse_run_skill_args(
    args_json: Option<&str>,
    args_file: Option<&PathBuf>,
) -> Result<serde_json::Value> {
    if args_json.is_some() && args_file.is_some() {
        anyhow::bail!("run_skill_args_source_conflict");
    }
    let raw = if let Some(raw) = args_json {
        Some(raw.to_string())
    } else if let Some(path) = args_file {
        Some(
            std::fs::read_to_string(path)
                .with_context(|| format!("read run-skill args file failed: {}", path.display()))?,
        )
    } else {
        None
    };
    let Some(raw) = raw else {
        return Ok(json!({}));
    };
    let value = serde_json::from_str::<serde_json::Value>(&raw).context("parse run-skill args")?;
    if !value.is_object() {
        anyhow::bail!("run_skill_args_must_be_json_object");
    }
    Ok(value)
}
