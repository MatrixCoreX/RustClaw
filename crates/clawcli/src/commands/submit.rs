use anyhow::Result;
use serde_json::json;

use crate::{events::EventFilters, output, task};

use super::common::wait_for_terminal_task;

pub(crate) fn run_submit(
    base_url: &str,
    key: &str,
    text: &str,
    wait: bool,
    detach: bool,
    json_output: bool,
    interval_ms: u64,
) -> Result<()> {
    if wait && detach {
        anyhow::bail!("submit_wait_detach_conflict");
    }
    let task_id = task::submit_ask(base_url, key, text)?;
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
            "detached": true,
        }));
    } else {
        println!("task_id: {}", task_id);
    }
    Ok(())
}

pub(crate) fn run_resume(
    base_url: &str,
    key: &str,
    resume_task_id: &str,
    text: &str,
) -> Result<()> {
    let task_id = task::submit_resume_ask(base_url, key, resume_task_id, text)?;
    println!("task_id: {}", task_id);
    println!("resume_task_id: {}", resume_task_id);
    Ok(())
}
