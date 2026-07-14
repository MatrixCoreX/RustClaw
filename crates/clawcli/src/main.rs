//! clawcli — terminal CLI for interacting with clawd.

mod auth;
mod chat;
mod client;
mod commands;
mod events;
mod output;
mod replay;
mod task;

use anyhow::Result;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{generate, Shell};
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
        profile: Option<String>,
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
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
        #[arg(long)]
        print_effective_config: bool,
    },

    /// Coding-agent shortcut for exec --profile coding.
    Code {
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
        #[arg(long)]
        artifact_dir: Option<PathBuf>,
        #[arg(long)]
        print_effective_config: bool,
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

    /// Print task event stream through the task API, without reading raw clawd logs.
    Logs {
        task_id: String,
        #[command(flatten)]
        event_filters: EventFilterArgs,
        #[arg(long)]
        jsonl: bool,
    },

    /// Print a stable task report summary.
    Report {
        task_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        events: bool,
    },

    /// Print coding-oriented task evidence summary.
    Review {
        task_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        events: bool,
    },

    /// Print subagent child-run summaries for a task.
    Subagents {
        task_id: String,
        #[arg(long)]
        json: bool,
    },

    /// Terminal task console.
    Tui {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        task_id: Option<String>,
        #[arg(long)]
        events: bool,
        #[arg(long)]
        once: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        interactive: bool,
        #[arg(long)]
        export_path: Option<PathBuf>,
    },

    /// Inspect structured permission and policy machine fields.
    Permission {
        #[command(subcommand)]
        command: PermissionCommand,
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

    /// POST /v1/tasks/automation-runs
    AutomationRuns {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        job_id: Option<String>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
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
    ResumeTask {
        task_id: String,
        #[arg(long = "checkpoint-id")]
        checkpoint_id: Option<String>,
        #[arg(long = "resume-reason")]
        resume_reason: Option<String>,
        #[arg(long = "message")]
        user_message: Option<String>,
        #[arg(long = "constraints-json")]
        constraints_json: Option<String>,
    },

    /// Continue a task by task id, optionally with a user message.
    Continue {
        task_id: String,
        #[arg(num_args = 0.., trailing_var_arg = true)]
        message: Vec<String>,
        #[arg(long)]
        json: bool,
    },

    /// Wait until a task reaches a selected machine lifecycle state.
    Wait {
        task_id: String,
        #[arg(long, value_enum, default_value = "terminal")]
        until: WaitUntil,
        #[arg(long)]
        timeout_seconds: Option<u64>,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        jsonl: bool,
    },

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

    /// Export or inspect recorded task replay bundles.
    Replay {
        #[command(subcommand)]
        command: ReplayCommand,
    },

    /// Generate shell completion script.
    Completions {
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum WaitUntil {
    Completed,
    Terminal,
    Background,
    NeedsUser,
}

#[derive(Subcommand)]
enum PermissionCommand {
    /// Summarize permission decisions recorded on a task.
    Inspect {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Same machine summary as inspect, intended for scripts that want JSON evidence.
    Explain {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Summarize capability availability and policy metadata.
    Capability {
        #[arg(long)]
        skill: Option<String>,
        #[arg(long)]
        capability: Option<String>,
        #[arg(long)]
        json: bool,
    },
}

impl WaitUntil {
    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Terminal => "terminal",
            Self::Background => "background",
            Self::NeedsUser => "needs_user",
        }
    }
}

#[derive(Subcommand)]
enum ReplayCommand {
    /// Export a redacted replay bundle for an existing task.
    Export {
        task_id: String,
        #[arg(short, long)]
        output: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Inspect a replay bundle without calling providers or tools.
    Run {
        bundle: PathBuf,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        coverage: bool,
    },
    /// Compare two replay bundles using stable machine fields.
    Diff {
        left: PathBuf,
        right: PathBuf,
        #[arg(long)]
        json: bool,
    },
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
            profile,
            resume_task_id,
            detach,
            json,
            jsonl,
            timeout_seconds,
            poll_interval_ms,
            continue_on_background,
            fail_on_background,
            artifact_dir,
            print_effective_config,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            let prompt = prompt.join(" ");
            run_exec_command(
                base_url,
                k,
                &prompt,
                profile.as_deref(),
                resume_task_id.as_deref(),
                *detach,
                *json,
                *jsonl,
                *timeout_seconds,
                *poll_interval_ms,
                *continue_on_background,
                *fail_on_background,
                artifact_dir.as_ref(),
                *print_effective_config,
            )
        }
        Command::Code {
            prompt,
            resume_task_id,
            detach,
            json,
            jsonl,
            timeout_seconds,
            poll_interval_ms,
            continue_on_background,
            fail_on_background,
            artifact_dir,
            print_effective_config,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            let prompt = prompt.join(" ");
            run_exec_command(
                base_url,
                k,
                &prompt,
                Some("coding"),
                resume_task_id.as_deref(),
                *detach,
                *json,
                *jsonl,
                *timeout_seconds,
                *poll_interval_ms,
                *continue_on_background,
                *fail_on_background,
                artifact_dir.as_ref(),
                *print_effective_config,
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
        Command::Logs {
            task_id,
            event_filters,
            jsonl,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_logs(
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
        Command::Report {
            task_id,
            json,
            events,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_report(base_url, k, task_id, *json, *events)
        }
        Command::Review {
            task_id,
            json,
            events,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_review(base_url, k, task_id, *json, *events)
        }
        Command::Subagents { task_id, json } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_subagents(base_url, k, task_id, *json)
        }
        Command::Tui {
            user_id,
            chat_id,
            task_id,
            events,
            once,
            interval_ms,
            json,
            interactive,
            export_path,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_tui(
                base_url,
                k,
                *user_id,
                *chat_id,
                task_id.as_deref(),
                *events,
                *once,
                *interval_ms,
                *json,
                *interactive,
                export_path.as_deref(),
            )
        }
        Command::Permission { command } => match command {
            PermissionCommand::Inspect { task_id, json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_permission_inspect(base_url, k, task_id, *json)
            }
            PermissionCommand::Explain { task_id, json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_permission_explain(base_url, k, task_id, *json)
            }
            PermissionCommand::Capability {
                skill,
                capability,
                json,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_permission_capability(
                    base_url,
                    k,
                    skill.as_deref(),
                    capability.as_deref(),
                    *json,
                )
            }
        },
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
        Command::AutomationRuns {
            user_id,
            chat_id,
            job_id,
            limit,
            json,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_automation_runs(
                base_url,
                k,
                *user_id,
                *chat_id,
                job_id.clone(),
                *limit,
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
        Command::ResumeTask {
            task_id,
            checkpoint_id,
            resume_reason,
            user_message,
            constraints_json,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_resume_task(
                base_url,
                k,
                task_id,
                checkpoint_id.as_deref(),
                resume_reason.as_deref(),
                user_message.as_deref(),
                constraints_json.as_deref(),
            )
        }
        Command::Continue {
            task_id,
            message,
            json,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            let message = message.join(" ");
            commands::run_continue_task(
                base_url,
                k,
                task_id,
                (!message.trim().is_empty()).then_some(message.as_str()),
                *json,
            )
        }
        Command::Wait {
            task_id,
            until,
            timeout_seconds,
            interval_ms,
            json,
            jsonl,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            let exit_code = commands::run_wait(
                base_url,
                k,
                task_id,
                until.as_str(),
                *timeout_seconds,
                *interval_ms,
                *json,
                *jsonl,
            )?;
            if exit_code == 0 {
                Ok(())
            } else {
                std::process::exit(i32::from(exit_code));
            }
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
        Command::Replay { command } => match command {
            ReplayCommand::Export {
                task_id,
                output: output_path,
                json,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                replay::run_export(base_url, k, task_id, output_path, *json)
            }
            ReplayCommand::Run {
                bundle,
                json,
                coverage,
            } => replay::run_run(bundle, *json, *coverage),
            ReplayCommand::Diff { left, right, json } => replay::run_diff(left, right, *json),
        },
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let bin_name = cmd.get_name().to_string();
            generate(*shell, &mut cmd, bin_name, &mut std::io::stdout());
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_exec_command(
    base_url: &str,
    key: &str,
    prompt: &str,
    profile_name: Option<&str>,
    resume_task_id: Option<&str>,
    detach: bool,
    json_output: bool,
    jsonl_output: bool,
    timeout_seconds: Option<u64>,
    interval_ms: u64,
    continue_on_background: bool,
    fail_on_background: bool,
    artifact_dir: Option<&PathBuf>,
    print_effective_config: bool,
) -> Result<()> {
    let exit_code = commands::run_exec(
        base_url,
        key,
        prompt,
        profile_name,
        resume_task_id,
        detach,
        json_output,
        jsonl_output,
        timeout_seconds,
        interval_ms,
        continue_on_background,
        fail_on_background,
        artifact_dir,
        print_effective_config,
    )?;
    if exit_code == 0 {
        Ok(())
    } else {
        std::process::exit(i32::from(exit_code));
    }
}
