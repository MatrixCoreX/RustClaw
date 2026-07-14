//! Command handlers for `clawcli`.

mod common;
mod exec;
mod health;
mod permission;
mod report;
mod report_budget_health;
mod report_outcome;
mod run_skill;
mod skills;
mod submit;
mod task_control;
mod task_query;
mod tui;

pub(crate) use exec::run_exec;
pub(crate) use health::run_health;
pub(crate) use permission::{
    run_permission_capability, run_permission_explain, run_permission_inspect,
};
pub(crate) use run_skill::run_skill;
pub(crate) use skills::{run_capabilities, run_reload_skills, run_skills};
pub(crate) use submit::{run_resume, run_submit};
pub(crate) use task_control::{
    run_active, run_automation_runs, run_cancel, run_cancel_index, run_cancel_task,
    run_continue_task, run_pause_task, run_resume_task,
};
pub(crate) use task_query::{
    run_events, run_get, run_logs, run_report, run_review, run_subagents, run_wait, run_watch,
};
pub(crate) use tui::run_tui;

#[cfg(test)]
use exec::{
    exec_effective_options, exec_exit_class, exec_failure_class_from_machine_tokens,
    exec_summary_json, write_exec_artifacts, ExecExitClass, ExecWaitOutcome,
};
#[cfg(test)]
use permission::permission_report_json;
#[cfg(test)]
use report::{coding_review_json, subagent_report_json, task_report_json, task_report_text_lines};
#[cfg(test)]
use task_control::automation_runs_request_payload;
#[cfg(test)]
use task_query::wait_until_matches;
#[cfg(test)]
use task_query::{task_event_output_lines, watch_progress_json};
#[cfg(test)]
use tui::{tui_command_from_input, tui_export_json, tui_snapshot_json, TuiCommand};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;
