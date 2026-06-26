//! clawcli — terminal CLI for interacting with clawd.

mod auth;
mod chat;
mod client;
mod commands;
mod events;
mod output;
mod task;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8787";

#[derive(Parser)]
#[command(name = "clawcli")]
#[command(about = "Terminal CLI to interact with clawd")]
#[command(subcommand_required = false)]
struct Cli {
    #[arg(long, default_value = DEFAULT_BASE_URL)]
    base_url: String,

    /// Admin key. Defaults to RUSTCLAW_ADMIN_KEY or the first enabled admin key in auth_keys.
    #[arg(short, long)]
    key: Option<String>,

    #[command(subcommand)]
    cmd: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive chat mode.
    Chat,

    /// GET /v1/health
    Health,

    /// POST /v1/tasks with kind=ask and payload {"text": "..."}.
    Submit {
        #[arg(short, long)]
        text: String,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        detach: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },

    /// Submit or resume an ask task and wait by default, suitable for scripts.
    Exec {
        #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
        prompt: Vec<String>,
        #[arg(long)]
        resume_task_id: Option<String>,
        #[arg(long)]
        detach: bool,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        jsonl: bool,
        #[arg(long)]
        timeout_seconds: Option<u64>,
        #[arg(long, default_value_t = 1000)]
        poll_interval_ms: u64,
        #[arg(long)]
        continue_on_background: bool,
        #[arg(long)]
        fail_on_background: bool,
    },

    /// POST /v1/tasks with kind=run_skill.
    RunSkill {
        skill_name: String,
        #[arg(long = "args-json")]
        args_json: Option<String>,
        #[arg(long = "args-file")]
        args_file: Option<PathBuf>,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },

    /// Continue an existing task through a user_followup payload.
    Resume {
        /// Existing task id to continue.
        task_id: String,
        /// Follow-up text for this continuation.
        #[arg(short, long)]
        text: String,
    },

    /// GET /v1/tasks/:task_id
    Get {
        task_id: String,
        #[arg(long)]
        events: bool,
        #[command(flatten)]
        event_filters: EventFilterArgs,
        #[arg(long)]
        events_output: Option<PathBuf>,
    },

    /// Poll GET /v1/tasks/:task_id until interrupted or terminal.
    Watch {
        task_id: String,
        #[arg(long)]
        events: bool,
        #[command(flatten)]
        event_filters: EventFilterArgs,
        #[arg(long)]
        until_terminal: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        jsonl: bool,
    },

    /// Print task event stream from GET /v1/tasks/:task_id.
    Events {
        task_id: String,
        #[command(flatten)]
        event_filters: EventFilterArgs,
        #[arg(long)]
        jsonl: bool,
    },

    /// POST /v1/tasks/active
    Active {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        exclude_task_id: Option<String>,
        #[arg(long)]
        json: bool,
    },

    /// POST /v1/tasks/cancel
    Cancel {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        exclude_task_id: Option<String>,
    },

    /// POST /v1/tasks/cancel-by-task-id
    CancelTask { task_id: String },

    /// POST /v1/tasks/resume-by-task-id
    ResumeTask { task_id: String },

    /// POST /v1/tasks/pause-by-task-id
    PauseTask {
        task_id: String,
        #[arg(long, default_value_t = 3600)]
        pause_seconds: u64,
    },

    /// POST /v1/tasks/cancel-one by active task index.
    CancelIndex {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        index: usize,
        #[arg(long)]
        exclude_task_id: Option<String>,
    },

    /// GET /v1/skills or /v1/skills/config.
    Skills {
        #[arg(long)]
        config: bool,
        #[arg(long)]
        json: bool,
    },

    /// Show registry capability metadata from /v1/skills/config.
    Capabilities {
        #[arg(long)]
        json: bool,
    },

    /// POST /v1/admin/reload-skills
    ReloadSkills,
}

#[derive(Args, Debug, Clone, Default)]
struct EventFilterArgs {
    #[arg(long = "event-type")]
    event_types: Vec<String>,
    #[arg(long = "checkpoint-id")]
    checkpoint_id: Option<String>,
    #[arg(long = "policy-decision")]
    policy_decision: Option<String>,
    #[arg(long = "subagent-id")]
    subagent_id: Option<String>,
    #[arg(long = "async-job-id")]
    async_job_id: Option<String>,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let base_url = std::env::var("RUSTCLAW_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cli.base_url.clone());
    let base_url = base_url.trim_end_matches('/');
    let key: Option<String> = cli.key.or_else(auth::default_admin_key);
    let cmd = cli.cmd.unwrap_or(Command::Chat);

    match &cmd {
        Command::Chat => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            chat::run_chat(base_url, k)
        }
        Command::Health => commands::run_health(base_url, key.as_deref()),
        Command::Submit {
            text,
            wait,
            detach,
            json,
            interval_ms,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_submit(base_url, k, text, *wait, *detach, *json, *interval_ms)
        }
        Command::Exec {
            prompt,
            resume_task_id,
            detach,
            json,
            jsonl,
            timeout_seconds,
            poll_interval_ms,
            continue_on_background,
            fail_on_background,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            let prompt = prompt.join(" ");
            commands::run_exec(
                base_url,
                k,
                &prompt,
                resume_task_id.as_deref(),
                *detach,
                *json,
                *jsonl,
                *timeout_seconds,
                *poll_interval_ms,
                *continue_on_background,
                *fail_on_background,
            )
        }
        Command::RunSkill {
            skill_name,
            args_json,
            args_file,
            wait,
            json,
            interval_ms,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_skill(
                base_url,
                k,
                skill_name,
                args_json.as_deref(),
                args_file.as_ref(),
                *wait,
                *json,
                *interval_ms,
            )
        }
        Command::Resume { task_id, text } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_resume(base_url, k, task_id, text)
        }
        Command::Get {
            task_id,
            events,
            event_filters,
            events_output,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_get(
                base_url,
                k,
                task_id,
                *events,
                &event_filters.event_types,
                event_filters.checkpoint_id.as_deref(),
                event_filters.policy_decision.as_deref(),
                event_filters.subagent_id.as_deref(),
                event_filters.async_job_id.as_deref(),
                events_output.as_ref(),
            )
        }
        Command::Watch {
            task_id,
            events,
            event_filters,
            until_terminal,
            interval_ms,
            json,
            jsonl,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_watch(
                base_url,
                k,
                task_id,
                *events,
                &event_filters.event_types,
                event_filters.checkpoint_id.as_deref(),
                event_filters.policy_decision.as_deref(),
                event_filters.subagent_id.as_deref(),
                event_filters.async_job_id.as_deref(),
                *until_terminal,
                *interval_ms,
                *json,
                *jsonl,
            )
        }
        Command::Events {
            task_id,
            event_filters,
            jsonl,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_events(
                base_url,
                k,
                task_id,
                &event_filters.event_types,
                event_filters.checkpoint_id.as_deref(),
                event_filters.policy_decision.as_deref(),
                event_filters.subagent_id.as_deref(),
                event_filters.async_job_id.as_deref(),
                *jsonl,
            )
        }
        Command::Active {
            user_id,
            chat_id,
            exclude_task_id,
            json,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_active(
                base_url,
                k,
                *user_id,
                *chat_id,
                exclude_task_id.clone(),
                *json,
            )
        }
        Command::Cancel {
            user_id,
            chat_id,
            exclude_task_id,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_cancel(base_url, k, *user_id, *chat_id, exclude_task_id.clone())
        }
        Command::CancelTask { task_id } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_cancel_task(base_url, k, task_id)
        }
        Command::ResumeTask { task_id } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_resume_task(base_url, k, task_id)
        }
        Command::PauseTask {
            task_id,
            pause_seconds,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_pause_task(base_url, k, task_id, *pause_seconds)
        }
        Command::CancelIndex {
            user_id,
            chat_id,
            index,
            exclude_task_id,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_cancel_index(
                base_url,
                k,
                *user_id,
                *chat_id,
                *index,
                exclude_task_id.clone(),
            )
        }
        Command::Skills { config, json } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_skills(base_url, k, *config, *json)
        }
        Command::Capabilities { json } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_capabilities(base_url, k, *json)
        }
        Command::ReloadSkills => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_reload_skills(base_url, k)
        }
    }
}
