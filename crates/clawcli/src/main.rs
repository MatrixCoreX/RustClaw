//! clawcli — terminal CLI for interacting with clawd.

mod auth;
mod chat;
mod client;
mod commands;
mod events;
mod interrupt;
mod output;
mod replay;
mod resources;
mod task;

use anyhow::Result;
use clap::{Args, CommandFactory, FromArgMatches, Parser, Subcommand, ValueEnum};
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
    Chat {
        /// Start a new persisted thread instead of continuing the latest one.
        #[arg(long = "new", conflicts_with = "thread_id")]
        new_thread: bool,
        /// Continue a specific persisted thread.
        #[arg(long)]
        thread_id: Option<String>,
        /// Print raw task events as JSONL.
        #[arg(long)]
        jsonl: bool,
    },

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

    /// Coding-agent commands.
    Code {
        #[command(subcommand)]
        command: CodeCommand,
    },

    /// Submit and inspect tasks with structured goal metadata.
    Goal {
        #[command(subcommand)]
        command: GoalCommand,
    },

    /// Navigate and resume task sessions.
    Session {
        #[command(subcommand)]
        command: SessionCommand,
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
        /// Follow the resumable live event stream until a terminal event.
        #[arg(long)]
        follow: bool,
        /// Resume after this event sequence id.
        #[arg(long, default_value_t = 0)]
        cursor: u64,
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

    /// Print numbered LLM request/response trace for a task.
    LlmTrace {
        task_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        raw: bool,
        #[arg(long)]
        limit: Option<usize>,
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

    /// Inspect provider/model capability metadata.
    Models {
        #[command(subcommand)]
        command: ModelsCommand,
    },

    /// Inspect configured MCP servers and discovered tools.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
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
        #[arg(
            long = "approval-decision",
            value_enum,
            requires = "approval_request_id"
        )]
        approval_decision: Option<task::ApprovalDecisionArg>,
        #[arg(long = "approval-request-id", requires = "approval_decision")]
        approval_request_id: Option<String>,
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
    /// List durable scoped approval grants owned by the authenticated actor.
    Grants {
        #[arg(long)]
        json: bool,
    },
    /// Revoke one durable scoped approval grant.
    Revoke {
        grant_id: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum ModelsCommand {
    /// GET /v1/models/catalog
    Catalog {
        #[arg(long)]
        provider: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Print selected provider/model readiness from /v1/models/catalog.
    Readiness {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum McpCommand {
    /// List configured MCP servers.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show MCP server lifecycle state.
    Status {
        server: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// List discovered MCP tools.
    Tools {
        server: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Run a protocol ping without invoking a tool.
    Test {
        server: String,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
enum GoalCommand {
    /// Submit an ask task with a structured goal contract.
    Start {
        #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
        prompt: Vec<String>,
        #[arg(long)]
        objective: Option<String>,
        #[arg(long = "done")]
        done_conditions: Vec<String>,
        #[arg(long = "verify")]
        verification_commands: Vec<String>,
        #[arg(long = "constraint")]
        constraints: Vec<String>,
        #[arg(long)]
        wait: bool,
        #[arg(long)]
        detach: bool,
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },
    /// Print the structured goal projection for a task.
    Status {
        task_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Pause a goal task through the existing checkpoint pause control.
    Pause {
        task_id: String,
        #[arg(long, default_value_t = 3600)]
        pause_seconds: u64,
    },
    /// Resume a goal task through the existing checkpoint resume control.
    Resume {
        task_id: String,
        #[arg(long = "checkpoint-id")]
        checkpoint_id: Option<String>,
        #[arg(long = "message")]
        user_message: Option<String>,
        #[arg(long = "constraints-json")]
        constraints_json: Option<String>,
    },
    /// Patch structured goal metadata on an existing task.
    Edit {
        task_id: String,
        #[arg(long = "goal-json")]
        goal_json: Option<String>,
        #[arg(long)]
        objective: Option<String>,
        #[arg(long = "done")]
        done_conditions: Vec<String>,
        #[arg(long = "verify")]
        verification_commands: Vec<String>,
        #[arg(long = "constraint")]
        constraints: Vec<String>,
        #[arg(long = "allowed-scope")]
        allowed_scopes: Vec<String>,
        #[arg(long = "forbidden-action")]
        forbidden_actions: Vec<String>,
        #[arg(long = "goal-status")]
        goal_status: Option<String>,
    },
    /// Remove structured goal metadata from an existing task payload.
    Clear { task_id: String },
}

#[derive(Subcommand)]
enum CodeCommand {
    /// Run a coding-agent task with exec --profile coding.
    Run {
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
    /// Alias for report focused on coding task status.
    Status {
        task_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        events: bool,
    },
    /// Alias for the coding-oriented review summary.
    Review {
        task_id: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        events: bool,
    },
    /// Continue a coding task by task id.
    Continue {
        task_id: String,
        #[arg(num_args = 0.., trailing_var_arg = true)]
        message: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Inspect the current workspace or a reversible checkpoint diff.
    Diff {
        #[arg(long = "checkpoint-id")]
        checkpoint_id: Option<String>,
        #[arg(long = "path")]
        paths: Vec<String>,
        #[arg(long)]
        detach: bool,
        #[arg(long, conflicts_with = "jsonl")]
        json: bool,
        #[arg(long)]
        jsonl: bool,
        #[arg(long)]
        timeout_seconds: Option<u64>,
        #[arg(long, default_value_t = 1000)]
        poll_interval_ms: u64,
    },
    /// Rewind a reversible workspace checkpoint after one-shot approval.
    Rewind {
        #[arg(long = "checkpoint-id")]
        checkpoint_id: String,
        #[arg(long)]
        detach: bool,
        #[arg(long, conflicts_with = "jsonl")]
        json: bool,
        #[arg(long)]
        jsonl: bool,
        #[arg(long)]
        timeout_seconds: Option<u64>,
        #[arg(long, default_value_t = 1000)]
        poll_interval_ms: u64,
    },
    /// Compatibility fallback for `clawcli code <prompt...>`.
    #[command(external_subcommand)]
    Prompt(Vec<String>),
}

#[derive(Subcommand)]
enum SessionCommand {
    /// List active task sessions for a user/chat pair.
    List {
        #[arg(long)]
        user_id: i64,
        #[arg(long)]
        chat_id: i64,
        #[arg(long)]
        json: bool,
    },
    /// Show a task session summary by task/session id.
    Show {
        session_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Resume a task session by task/session id.
    Resume {
        session_id: String,
        #[arg(num_args = 0..)]
        message: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Continue the latest locally persisted chat thread with a new turn.
    ContinueLatest {
        #[arg(required = true, num_args = 1..)]
        message: Vec<String>,
        #[arg(long)]
        json: bool,
    },
    /// Mark a locally saved task session archived.
    Archive {
        session_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Delete a locally saved task session.
    Delete {
        session_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Fork local session metadata under a new session id.
    Fork {
        session_id: String,
        new_session_id: String,
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
        #[arg(long, value_enum, default_value_t = ReplayView::Summary)]
        view: ReplayView,
    },
    /// Compare two replay bundles using stable machine fields.
    Diff {
        left: PathBuf,
        right: PathBuf,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ReplayView {
    Summary,
    Llm,
    Tools,
    Checkpoints,
}

impl ReplayView {
    fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Llm => "llm",
            Self::Tools => "tools",
            Self::Checkpoints => "checkpoints",
        }
    }
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
    let mut command = Cli::command().about(resources::text("cli.about"));
    localize_command_help(&mut command);
    let matches = command.get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|error| error.exit());
    let base_url = std::env::var("RUSTCLAW_BASE_URL")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cli.base_url.clone());
    let base_url = base_url.trim_end_matches('/');
    let key: Option<String> = cli.key.or_else(auth::default_admin_key);
    let cmd = cli.cmd.unwrap_or(Command::Chat {
        new_thread: false,
        thread_id: None,
        jsonl: false,
    });

    match &cmd {
        Command::Chat {
            new_thread,
            thread_id,
            jsonl,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            chat::run_chat(base_url, k, thread_id.as_deref(), *new_thread, *jsonl)
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
        Command::Code { command } => match command {
            CodeCommand::Run {
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
            CodeCommand::Status {
                task_id,
                json,
                events,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_report(base_url, k, task_id, *json, *events)
            }
            CodeCommand::Review {
                task_id,
                json,
                events,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_review(base_url, k, task_id, *json, *events)
            }
            CodeCommand::Continue {
                task_id,
                message,
                json,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                let message = (!message.is_empty()).then(|| message.join(" "));
                commands::run_continue_task(base_url, k, task_id, message.as_deref(), *json)
            }
            CodeCommand::Diff {
                checkpoint_id,
                paths,
                detach,
                json,
                jsonl,
                timeout_seconds,
                poll_interval_ms,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                run_code_capability_command(
                    base_url,
                    k,
                    "workspace.diff",
                    commands::workspace_diff_args(checkpoint_id.as_deref(), paths),
                    *detach,
                    *json,
                    *jsonl,
                    *timeout_seconds,
                    *poll_interval_ms,
                )
            }
            CodeCommand::Rewind {
                checkpoint_id,
                detach,
                json,
                jsonl,
                timeout_seconds,
                poll_interval_ms,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                run_code_capability_command(
                    base_url,
                    k,
                    "workspace.revert_checkpoint",
                    commands::workspace_rewind_args(checkpoint_id),
                    *detach,
                    *json,
                    *jsonl,
                    *timeout_seconds,
                    *poll_interval_ms,
                )
            }
            CodeCommand::Prompt(prompt) => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                let prompt = prompt.join(" ");
                run_exec_command(
                    base_url,
                    k,
                    &prompt,
                    Some("coding"),
                    None,
                    false,
                    false,
                    false,
                    None,
                    1000,
                    false,
                    false,
                    None,
                    false,
                )
            }
        },
        Command::Goal { command } => match command {
            GoalCommand::Start {
                prompt,
                objective,
                done_conditions,
                verification_commands,
                constraints,
                wait,
                detach,
                json,
                interval_ms,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                let prompt = prompt.join(" ");
                commands::run_goal_start(
                    base_url,
                    k,
                    &prompt,
                    objective.as_deref(),
                    done_conditions,
                    verification_commands,
                    constraints,
                    *wait,
                    *detach,
                    *json,
                    *interval_ms,
                )
            }
            GoalCommand::Status { task_id, json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_goal_status(base_url, k, task_id, *json)
            }
            GoalCommand::Pause {
                task_id,
                pause_seconds,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_goal_pause(base_url, k, task_id, *pause_seconds)
            }
            GoalCommand::Resume {
                task_id,
                checkpoint_id,
                user_message,
                constraints_json,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_goal_resume(
                    base_url,
                    k,
                    task_id,
                    checkpoint_id.as_deref(),
                    user_message.as_deref(),
                    constraints_json.as_deref(),
                )
            }
            GoalCommand::Edit {
                task_id,
                goal_json,
                objective,
                done_conditions,
                verification_commands,
                constraints,
                allowed_scopes,
                forbidden_actions,
                goal_status,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_goal_edit(
                    base_url,
                    k,
                    task_id,
                    goal_json.as_deref(),
                    objective.as_deref(),
                    done_conditions,
                    verification_commands,
                    constraints,
                    allowed_scopes,
                    forbidden_actions,
                    goal_status.as_deref(),
                )
            }
            GoalCommand::Clear { task_id } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_goal_clear(base_url, k, task_id)
            }
        },
        Command::Session { command } => match command {
            SessionCommand::List {
                user_id,
                chat_id,
                json,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_session_list(base_url, k, *user_id, *chat_id, *json)
            }
            SessionCommand::Show { session_id, json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_session_show(base_url, k, session_id, *json)
            }
            SessionCommand::Resume {
                session_id,
                message,
                json,
            } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                let message = (!message.is_empty()).then(|| message.join(" "));
                commands::run_session_resume(base_url, k, session_id, message.as_deref(), *json)
            }
            SessionCommand::ContinueLatest { message, json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_session_continue_latest(base_url, k, &message.join(" "), *json)
            }
            SessionCommand::Archive { session_id, json } => {
                commands::run_session_archive(session_id, *json)
            }
            SessionCommand::Delete { session_id, json } => {
                commands::run_session_delete(session_id, *json)
            }
            SessionCommand::Fork {
                session_id,
                new_session_id,
                json,
            } => commands::run_session_fork(session_id, new_session_id, *json),
        },
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
            follow,
            cursor,
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
                *follow,
                *cursor,
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
        Command::LlmTrace {
            task_id,
            json,
            raw,
            limit,
        } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_llm_trace(base_url, k, task_id, *json, *raw, *limit)
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
            PermissionCommand::Grants { json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_permission_grants(base_url, k, *json)
            }
            PermissionCommand::Revoke { grant_id, json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_permission_revoke(base_url, k, grant_id, *json)
            }
        },
        Command::Models { command } => match command {
            ModelsCommand::Catalog { provider, json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_models_catalog(base_url, k, provider.as_deref(), *json)
            }
            ModelsCommand::Readiness { json } => {
                let k = key.as_deref().ok_or_else(auth::key_required_error)?;
                commands::run_models_readiness(base_url, k, *json)
            }
        },
        Command::Mcp { command } => {
            let k = key.as_deref().ok_or_else(auth::key_required_error)?;
            commands::run_mcp(base_url, k, command)
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
            approval_decision,
            approval_request_id,
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
                approval_request_id.as_deref(),
                approval_decision.map(|decision| decision.as_str()),
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
                view,
            } => replay::run_run(bundle, *json, *coverage, view.as_str()),
            ReplayCommand::Diff { left, right, json } => replay::run_diff(left, right, *json),
        },
        Command::Completions { shell } => {
            let mut cmd = Cli::command().about(resources::text("cli.about"));
            localize_command_help(&mut cmd);
            let bin_name = cmd.get_name().to_string();
            generate(*shell, &mut cmd, bin_name, &mut std::io::stdout());
            Ok(())
        }
    }
}

fn localize_command_help(command: &mut clap::Command) {
    for subcommand in command.get_subcommands_mut() {
        let key = format!("command.{}", subcommand.get_name());
        if let Some(about) = resources::optional_text(&key) {
            *subcommand = subcommand.clone().about(about);
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

#[allow(clippy::too_many_arguments)]
fn run_code_capability_command(
    base_url: &str,
    key: &str,
    capability: &str,
    args: serde_json::Value,
    detach: bool,
    json_output: bool,
    jsonl_output: bool,
    timeout_seconds: Option<u64>,
    interval_ms: u64,
) -> Result<()> {
    let exit_code = commands::run_code_capability(
        base_url,
        key,
        capability,
        args,
        commands::CodeCapabilityOptions {
            detach,
            json_output,
            jsonl_output,
            timeout_seconds: timeout_seconds.or(Some(900)),
            interval_ms,
        },
    )?;
    if exit_code == 0 {
        Ok(())
    } else {
        std::process::exit(i32::from(exit_code));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clawcli_exposes_coding_workflow_command_surface() {
        let cmd = Cli::command();
        let command_names = cmd
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<std::collections::BTreeSet<_>>();

        for required in [
            "submit",
            "exec",
            "code",
            "goal",
            "session",
            "get",
            "watch",
            "events",
            "report",
            "review",
            "continue",
            "resume-task",
            "cancel-task",
            "permission",
            "subagents",
            "llm-trace",
            "replay",
            "wait",
        ] {
            assert!(command_names.contains(required), "missing {required}");
        }

        let permission = cmd
            .find_subcommand("permission")
            .expect("permission command");
        let permission_names = permission
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for required in ["inspect", "explain", "capability"] {
            assert!(permission_names.contains(required), "missing {required}");
        }

        let goal = cmd.find_subcommand("goal").expect("goal command");
        let goal_names = goal
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for required in ["start", "status", "pause", "resume", "edit", "clear"] {
            assert!(goal_names.contains(required), "missing {required}");
        }

        let code = cmd.find_subcommand("code").expect("code command");
        let code_names = code
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for required in ["run", "status", "review", "continue", "diff", "rewind"] {
            assert!(code_names.contains(required), "missing {required}");
        }

        let session = cmd.find_subcommand("session").expect("session command");
        let session_names = session
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for required in [
            "list",
            "show",
            "resume",
            "continue-latest",
            "archive",
            "delete",
            "fork",
        ] {
            assert!(session_names.contains(required), "missing {required}");
        }

        let replay = cmd.find_subcommand("replay").expect("replay command");
        let replay_names = replay
            .get_subcommands()
            .map(|subcommand| subcommand.get_name().to_string())
            .collect::<std::collections::BTreeSet<_>>();
        for required in ["export", "run", "diff"] {
            assert!(replay_names.contains(required), "missing {required}");
        }
    }

    #[test]
    fn clawcli_parses_persisted_chat_thread_options() {
        match Cli::try_parse_from(["clawcli", "chat", "--thread-id", "thread_01", "--jsonl"])
            .expect("parse chat thread options")
            .cmd
        {
            Some(Command::Chat {
                new_thread,
                thread_id,
                jsonl,
            }) => {
                assert!(!new_thread);
                assert_eq!(thread_id.as_deref(), Some("thread_01"));
                assert!(jsonl);
            }
            _ => panic!("expected chat command"),
        }

        assert!(matches!(
            Cli::try_parse_from(["clawcli", "chat", "--new"])
                .expect("parse new chat thread")
                .cmd,
            Some(Command::Chat {
                new_thread: true,
                thread_id: None,
                jsonl: false,
            })
        ));
        assert!(
            Cli::try_parse_from(["clawcli", "chat", "--new", "--thread-id", "thread_01"]).is_err()
        );
    }

    #[test]
    fn clawcli_parses_code_subcommands_and_prompt_fallback() {
        match Cli::try_parse_from(["clawcli", "code", "status", "task-1"])
            .expect("parse status")
            .cmd
        {
            Some(Command::Code {
                command: CodeCommand::Status { task_id, .. },
            }) => assert_eq!(task_id, "task-1"),
            _ => panic!("expected code status"),
        }

        match Cli::try_parse_from(["clawcli", "code", "review", "task-1", "--json"])
            .expect("parse review")
            .cmd
        {
            Some(Command::Code {
                command: CodeCommand::Review { task_id, json, .. },
            }) => {
                assert_eq!(task_id, "task-1");
                assert!(json);
            }
            _ => panic!("expected code review"),
        }

        match Cli::try_parse_from(["clawcli", "code", "continue", "task-1", "next", "step"])
            .expect("parse continue")
            .cmd
        {
            Some(Command::Code {
                command:
                    CodeCommand::Continue {
                        task_id, message, ..
                    },
            }) => {
                assert_eq!(task_id, "task-1");
                assert_eq!(message, vec!["next".to_string(), "step".to_string()]);
            }
            _ => panic!("expected code continue"),
        }

        match Cli::try_parse_from(["clawcli", "code", "run", "fix", "the", "test"])
            .expect("parse run")
            .cmd
        {
            Some(Command::Code {
                command: CodeCommand::Run { prompt, .. },
            }) => assert_eq!(
                prompt,
                vec!["fix".to_string(), "the".to_string(), "test".to_string()]
            ),
            _ => panic!("expected code run"),
        }

        match Cli::try_parse_from([
            "clawcli",
            "code",
            "diff",
            "--checkpoint-id",
            "checkpoint-1",
            "--path",
            "src/lib.rs",
            "--jsonl",
        ])
        .expect("parse workspace diff")
        .cmd
        {
            Some(Command::Code {
                command:
                    CodeCommand::Diff {
                        checkpoint_id,
                        paths,
                        jsonl,
                        ..
                    },
            }) => {
                assert_eq!(checkpoint_id.as_deref(), Some("checkpoint-1"));
                assert_eq!(paths, vec!["src/lib.rs".to_string()]);
                assert!(jsonl);
            }
            _ => panic!("expected code diff"),
        }

        match Cli::try_parse_from([
            "clawcli",
            "code",
            "rewind",
            "--checkpoint-id",
            "checkpoint-2",
            "--json",
        ])
        .expect("parse workspace rewind")
        .cmd
        {
            Some(Command::Code {
                command:
                    CodeCommand::Rewind {
                        checkpoint_id,
                        json,
                        ..
                    },
            }) => {
                assert_eq!(checkpoint_id, "checkpoint-2");
                assert!(json);
            }
            _ => panic!("expected code rewind"),
        }

        match Cli::try_parse_from(["clawcli", "code", "fix", "the", "test"])
            .expect("parse prompt fallback")
            .cmd
        {
            Some(Command::Code {
                command: CodeCommand::Prompt(prompt),
            }) => assert_eq!(
                prompt,
                vec!["fix".to_string(), "the".to_string(), "test".to_string()]
            ),
            _ => panic!("expected prompt fallback"),
        }
    }

    #[test]
    fn clawcli_parses_session_subcommands() {
        match Cli::try_parse_from([
            "clawcli",
            "session",
            "list",
            "--user-id",
            "7",
            "--chat-id",
            "9",
            "--json",
        ])
        .expect("parse session list")
        .cmd
        {
            Some(Command::Session {
                command:
                    SessionCommand::List {
                        user_id,
                        chat_id,
                        json,
                    },
            }) => {
                assert_eq!(user_id, 7);
                assert_eq!(chat_id, 9);
                assert!(json);
            }
            _ => panic!("expected session list"),
        }

        match Cli::try_parse_from(["clawcli", "session", "show", "task-1", "--json"])
            .expect("parse session show")
            .cmd
        {
            Some(Command::Session {
                command: SessionCommand::Show { session_id, json },
            }) => {
                assert_eq!(session_id, "task-1");
                assert!(json);
            }
            _ => panic!("expected session show"),
        }

        match Cli::try_parse_from([
            "clawcli", "session", "resume", "task-1", "continue", "work", "--json",
        ])
        .expect("parse session resume")
        .cmd
        {
            Some(Command::Session {
                command:
                    SessionCommand::Resume {
                        session_id,
                        message,
                        json,
                    },
            }) => {
                assert_eq!(session_id, "task-1");
                assert_eq!(message, vec!["continue".to_string(), "work".to_string()]);
                assert!(json);
            }
            _ => panic!("expected session resume"),
        }

        match Cli::try_parse_from([
            "clawcli",
            "session",
            "continue-latest",
            "continue",
            "work",
            "--json",
        ])
        .expect("parse latest session continuation")
        .cmd
        {
            Some(Command::Session {
                command: SessionCommand::ContinueLatest { message, json },
            }) => {
                assert_eq!(message, vec!["continue".to_string(), "work".to_string()]);
                assert!(json);
            }
            _ => panic!("expected latest session continuation"),
        }

        match Cli::try_parse_from(["clawcli", "session", "archive", "task-1", "--json"])
            .expect("parse session archive")
            .cmd
        {
            Some(Command::Session {
                command: SessionCommand::Archive { session_id, json },
            }) => {
                assert_eq!(session_id, "task-1");
                assert!(json);
            }
            _ => panic!("expected session archive"),
        }

        match Cli::try_parse_from(["clawcli", "session", "delete", "task-1", "--json"])
            .expect("parse session delete")
            .cmd
        {
            Some(Command::Session {
                command: SessionCommand::Delete { session_id, json },
            }) => {
                assert_eq!(session_id, "task-1");
                assert!(json);
            }
            _ => panic!("expected session delete"),
        }

        match Cli::try_parse_from(["clawcli", "session", "fork", "task-1", "task-2", "--json"])
            .expect("parse session fork")
            .cmd
        {
            Some(Command::Session {
                command:
                    SessionCommand::Fork {
                        session_id,
                        new_session_id,
                        json,
                    },
            }) => {
                assert_eq!(session_id, "task-1");
                assert_eq!(new_session_id, "task-2");
                assert!(json);
            }
            _ => panic!("expected session fork"),
        }
    }

    #[test]
    fn clawcli_parses_closed_approval_decision_protocol() {
        match Cli::try_parse_from([
            "clawcli",
            "resume-task",
            "task-1",
            "--approval-request-id",
            "approval-1",
            "--approval-decision",
            "always-for-scope",
        ])
        .expect("parse scoped approval")
        .cmd
        {
            Some(Command::ResumeTask {
                approval_request_id,
                approval_decision,
                ..
            }) => {
                assert_eq!(approval_request_id.as_deref(), Some("approval-1"));
                assert!(matches!(
                    approval_decision,
                    Some(task::ApprovalDecisionArg::AlwaysForScope)
                ));
            }
            _ => panic!("expected scoped resume-task approval decision"),
        }

        match Cli::try_parse_from([
            "clawcli",
            "resume-task",
            "task-1",
            "--approval-request-id",
            "approval-1",
            "--approval-decision",
            "deny",
        ])
        .expect("parse approval denial")
        .cmd
        {
            Some(Command::ResumeTask {
                approval_request_id,
                approval_decision,
                ..
            }) => {
                assert_eq!(approval_request_id.as_deref(), Some("approval-1"));
                assert!(matches!(
                    approval_decision,
                    Some(task::ApprovalDecisionArg::Deny)
                ));
            }
            _ => panic!("expected resume-task approval decision"),
        }

        assert!(Cli::try_parse_from([
            "clawcli",
            "resume-task",
            "task-1",
            "--approval-request-id",
            "approval-1",
            "--approval-decision",
            "approve",
        ])
        .is_err());
        assert!(Cli::try_parse_from([
            "clawcli",
            "resume-task",
            "task-1",
            "--approve",
            "--approval-request-id",
            "approval-1",
        ])
        .is_err());
    }
}
