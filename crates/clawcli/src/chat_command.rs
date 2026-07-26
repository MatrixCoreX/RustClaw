use std::path::PathBuf;

use serde::Serialize;

use crate::chat_session::PermissionMode;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatCommand {
    Help,
    New,
    Resume(String),
    ResumeTask(String),
    Detach,
    Cancel,
    Status,
    Continue,
    Approve,
    ApproveScope,
    Deny,
    Model(Option<String>),
    Compact,
    Diff(Vec<String>),
    Permissions(Option<PermissionMode>),
    Files,
    Attachments,
    File(PathBuf),
    Image(PathBuf),
    Goal,
    Exit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ChatCommandError {
    pub(crate) schema_version: u32,
    pub(crate) error_code: &'static str,
    pub(crate) command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) argument: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) suggestion: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChatCommandSpec {
    pub(crate) token: &'static str,
    pub(crate) argument_shape: &'static str,
}

const COMMAND_SPECS: &[ChatCommandSpec] = &[
    spec("/help", "none"),
    spec("/new", "none"),
    spec("/resume", "conversation_id"),
    spec("/resume-task", "task_id"),
    spec("/detach", "none"),
    spec("/cancel", "none"),
    spec("/status", "none"),
    spec("/continue", "none"),
    spec("/approve", "none"),
    spec("/approve-scope", "none"),
    spec("/deny", "none"),
    spec("/model", "optional_model_id"),
    spec("/compact", "none"),
    spec("/diff", "optional_paths"),
    spec("/permissions", "optional_mode"),
    spec("/files", "none"),
    spec("/attachments", "none"),
    spec("/file", "path"),
    spec("/image", "path"),
    spec("/goal", "none"),
    spec("/exit", "none"),
];

const fn spec(token: &'static str, argument_shape: &'static str) -> ChatCommandSpec {
    ChatCommandSpec {
        token,
        argument_shape,
    }
}

pub(crate) fn command_specs() -> &'static [ChatCommandSpec] {
    COMMAND_SPECS
}

pub(crate) fn parse_chat_command(input: &str) -> Option<Result<ChatCommand, ChatCommandError>> {
    let trimmed = input.trim();
    if !trimmed.starts_with('/') {
        return None;
    }
    let words = match shell_words::split(trimmed) {
        Ok(words) if !words.is_empty() => words,
        Ok(_) => return Some(Err(error("chat_command_empty", "", None, None))),
        Err(_) => {
            let command = trimmed.split_whitespace().next().unwrap_or_default();
            return Some(Err(error("chat_command_parse_failed", command, None, None)));
        }
    };
    let command = words[0].as_str();
    let args = &words[1..];
    let parsed = match command {
        "/help" => no_args(command, args, ChatCommand::Help),
        "/new" => no_args(command, args, ChatCommand::New),
        "/resume" => one_arg(command, args, ChatCommand::Resume),
        "/resume-task" => one_arg(command, args, ChatCommand::ResumeTask),
        "/detach" => no_args(command, args, ChatCommand::Detach),
        "/cancel" => no_args(command, args, ChatCommand::Cancel),
        "/status" => no_args(command, args, ChatCommand::Status),
        "/continue" => no_args(command, args, ChatCommand::Continue),
        "/approve" => no_args(command, args, ChatCommand::Approve),
        "/approve-scope" => no_args(command, args, ChatCommand::ApproveScope),
        "/deny" => no_args(command, args, ChatCommand::Deny),
        "/model" => optional_one_arg(command, args).map(ChatCommand::Model),
        "/compact" => no_args(command, args, ChatCommand::Compact),
        "/diff" => Ok(ChatCommand::Diff(args.to_vec())),
        "/permissions" => parse_permissions(command, args),
        "/files" => no_args(command, args, ChatCommand::Files),
        "/attachments" => no_args(command, args, ChatCommand::Attachments),
        "/file" => one_arg(command, args, |path| ChatCommand::File(path.into())),
        "/image" => one_arg(command, args, |path| ChatCommand::Image(path.into())),
        "/goal" => no_args(command, args, ChatCommand::Goal),
        "/exit" => no_args(command, args, ChatCommand::Exit),
        _ => Err(error(
            "chat_command_unknown",
            command,
            None,
            command_suggestion(command),
        )),
    };
    Some(parsed)
}

fn no_args(
    command: &str,
    args: &[String],
    value: ChatCommand,
) -> Result<ChatCommand, ChatCommandError> {
    if let Some(argument) = args.first() {
        return Err(error(
            "chat_command_argument_unexpected",
            command,
            Some(argument),
            None,
        ));
    }
    Ok(value)
}

fn one_arg(
    command: &str,
    args: &[String],
    build: impl FnOnce(String) -> ChatCommand,
) -> Result<ChatCommand, ChatCommandError> {
    if args.is_empty() {
        return Err(error("chat_command_argument_missing", command, None, None));
    }
    if args.len() > 1 {
        return Err(error(
            "chat_command_argument_unexpected",
            command,
            args.get(1),
            None,
        ));
    }
    Ok(build(args[0].clone()))
}

fn optional_one_arg(command: &str, args: &[String]) -> Result<Option<String>, ChatCommandError> {
    if args.len() > 1 {
        return Err(error(
            "chat_command_argument_unexpected",
            command,
            args.get(1),
            None,
        ));
    }
    Ok(args.first().cloned())
}

fn parse_permissions(command: &str, args: &[String]) -> Result<ChatCommand, ChatCommandError> {
    let Some(raw) = optional_one_arg(command, args)? else {
        return Ok(ChatCommand::Permissions(None));
    };
    let Some(mode) = PermissionMode::parse(&raw) else {
        return Err(error(
            "chat_permission_mode_invalid",
            command,
            Some(&raw),
            None,
        ));
    };
    Ok(ChatCommand::Permissions(Some(mode)))
}

fn command_suggestion(command: &str) -> Option<String> {
    let mut matches = COMMAND_SPECS
        .iter()
        .filter(|spec| {
            spec.token.starts_with(command)
                || command.starts_with(spec.token)
                || edit_distance_at_most_one(command, spec.token)
        })
        .map(|spec| spec.token);
    let first = matches.next()?;
    matches.next().is_none().then(|| first.to_string())
}

fn edit_distance_at_most_one(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len().abs_diff(right.len()) > 1 {
        return false;
    }
    if left.len() == right.len() {
        return left
            .iter()
            .zip(right)
            .filter(|(a, b)| a != b)
            .take(2)
            .count()
            <= 1;
    }
    let (short, long) = if left.len() < right.len() {
        (left, right)
    } else {
        (right, left)
    };
    let mut short_index = 0;
    let mut long_index = 0;
    let mut skipped = false;
    while short_index < short.len() && long_index < long.len() {
        if short[short_index] == long[long_index] {
            short_index += 1;
            long_index += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            long_index += 1;
        }
    }
    true
}

fn error(
    error_code: &'static str,
    command: &str,
    argument: Option<&String>,
    suggestion: Option<String>,
) -> ChatCommandError {
    ChatCommandError {
        schema_version: 1,
        error_code,
        command: command.to_string(),
        argument: argument.cloned(),
        suggestion,
    }
}

#[cfg(test)]
#[path = "chat_command_tests.rs"]
mod tests;
