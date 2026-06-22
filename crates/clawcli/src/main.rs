//! clawcli — terminal CLI for interacting with clawd.

mod auth;
mod chat;
mod client;
mod commands;
mod events;
mod task;

use anyhow::Result;
use clap::{Parser, Subcommand};
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
        #[arg(long = "event-type")]
        event_types: Vec<String>,
        #[arg(long)]
        events_output: Option<PathBuf>,
    },

    /// Poll GET /v1/tasks/:task_id until interrupted or terminal.
    Watch {
        task_id: String,
        #[arg(long)]
        events: bool,
        #[arg(long = "event-type")]
        event_types: Vec<String>,
        #[arg(long)]
        until_terminal: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        json: bool,
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

    /// POST /v1/admin/reload-skills
    ReloadSkills,
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
        Command::Submit { text } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_submit(base_url, k, text)
        }
        Command::Resume { task_id, text } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_resume(base_url, k, task_id, text)
        }
        Command::Get {
            task_id,
            events,
            event_types,
            events_output,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_get(
                base_url,
                k,
                task_id,
                *events,
                event_types,
                events_output.as_ref(),
            )
        }
        Command::Watch {
            task_id,
            events,
            event_types,
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
                event_types,
                *until_terminal,
                *interval_ms,
                *json,
                *jsonl,
            )
        }
        Command::Active {
            user_id,
            chat_id,
            exclude_task_id,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_active(base_url, k, *user_id, *chat_id, exclude_task_id.clone())
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
        Command::ReloadSkills => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_reload_skills(base_url, k)
        }
    }
}
