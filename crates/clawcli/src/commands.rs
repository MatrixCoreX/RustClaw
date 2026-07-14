//! Command handlers for `clawcli`.

mod common;
mod exec;
mod goal;
mod health;
mod llm_trace;
mod permission;
mod report;
mod report_budget_health;
mod report_outcome;
mod run_skill;
mod session;
mod skills;
mod submit;
mod task_control;
mod task_query;
mod tui;

pub(crate) use exec::run_exec;
pub(crate) use goal::{
    run_goal_clear, run_goal_edit, run_goal_pause, run_goal_resume, run_goal_start, run_goal_status,
};
pub(crate) use health::run_health;
pub(crate) use llm_trace::run_llm_trace;
pub(crate) use permission::{
    run_permission_capability, run_permission_explain, run_permission_inspect,
};
pub(crate) use run_skill::run_skill;
pub(crate) use session::{
    run_session_archive, run_session_delete, run_session_fork, run_session_list,
    run_session_resume, run_session_show,
};
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
    exec_artifact_index_json, exec_compact_text_lines, exec_effective_options, exec_exit_class,
    exec_failure_class_from_machine_tokens, exec_summary_json, write_exec_artifacts, ExecExitClass,
    ExecWaitOutcome,
};
#[cfg(test)]
use goal::{
    goal_control_summary_json, goal_edit_patch_json, goal_request_payload,
    goal_status_summary_json, goal_status_text_lines,
};
#[cfg(test)]
use llm_trace::llm_trace_text_lines;
#[cfg(test)]
use permission::permission_report_json;
#[cfg(test)]
use report::{coding_review_json, subagent_report_json, task_report_json, task_report_text_lines};
#[cfg(test)]
use session::{
    session_list_json, session_resume_json, session_show_json, session_store_archive_json,
    session_store_delete_json, session_store_fork_json, session_store_upsert_summary, SessionStore,
};
#[cfg(test)]
use task_control::{automation_runs_request_payload, task_resume_control_summary_json};
#[cfg(test)]
use task_query::wait_until_matches;
#[cfg(test)]
use task_query::{task_event_output_lines, watch_progress_json};
#[cfg(test)]
use tui::{
    tui_command_from_input, tui_export_json, tui_selected_task_lines, tui_snapshot_json, TuiCommand,
};

#[cfg(test)]
#[path = "commands_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "commands_session_tests.rs"]
mod session_tests;
