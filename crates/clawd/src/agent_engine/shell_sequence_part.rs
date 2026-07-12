use super::*;

pub(super) fn shell_sequence_part_can_run_independently(part: &str, is_last: bool) -> bool {
    let words = shell_like_words(part);
    let Some(first_word) = words.first().map(|word| word.trim()) else {
        return false;
    };
    if !is_last && shell_word_is_variable_assignment(first_word) {
        return false;
    }
    let first = command_basename(first_word).trim();
    let first = first.to_ascii_lowercase();
    if matches!(
        first.as_str(),
        "if" | "for"
            | "while"
            | "until"
            | "case"
            | "select"
            | "do"
            | "then"
            | "else"
            | "elif"
            | "fi"
            | "done"
            | "esac"
            | "function"
            | "{"
            | "}"
            | "("
            | ")"
    ) {
        return false;
    }
    if !is_last
        && matches!(
            first.as_str(),
            "cd" | "export" | "source" | "." | "set" | "unset" | "alias" | "unalias" | "umask"
        )
    {
        return false;
    }
    true
}

fn shell_word_is_variable_assignment(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };
    let Some(first) = name.chars().next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

pub(super) fn split_sequential_run_cmd_actions(
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let split_conditionals =
        should_split_planner_introduced_shell_conditionals(user_text, original_user_text);
    let mut changed = false;
    let mut rewritten = Vec::with_capacity(actions.len());
    for action in actions {
        match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let parts = run_cmd_command_from_args(&args).and_then(|command| {
                    if run_cmd_args_async_start(&args)
                        || should_preserve_user_supplied_shell_command(
                            command,
                            user_text,
                            original_user_text,
                        )
                    {
                        None
                    } else if let Some(first_attempt) =
                        planner_failure_fallback_first_command(command, split_conditionals)
                    {
                        Some(vec![first_attempt])
                    } else {
                        split_shell_sequence_command_with_policy(command, split_conditionals)
                    }
                });
                if let Some(parts) = parts {
                    let continue_on_error = parts.len() > 1;
                    for command in parts {
                        rewritten.push(AgentAction::CallSkill {
                            skill: skill.clone(),
                            args: run_cmd_args_for_rewritten_command(
                                &args,
                                command,
                                continue_on_error,
                            ),
                        });
                    }
                    changed = true;
                } else {
                    rewritten.push(AgentAction::CallSkill { skill, args });
                }
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let parts = run_cmd_command_from_args(&args).and_then(|command| {
                    if run_cmd_args_async_start(&args)
                        || should_preserve_user_supplied_shell_command(
                            command,
                            user_text,
                            original_user_text,
                        )
                    {
                        None
                    } else if let Some(first_attempt) =
                        planner_failure_fallback_first_command(command, split_conditionals)
                    {
                        Some(vec![first_attempt])
                    } else {
                        split_shell_sequence_command_with_policy(command, split_conditionals)
                    }
                });
                if let Some(parts) = parts {
                    let continue_on_error = parts.len() > 1;
                    for command in parts {
                        rewritten.push(AgentAction::CallTool {
                            tool: tool.clone(),
                            args: run_cmd_args_for_rewritten_command(
                                &args,
                                command,
                                continue_on_error,
                            ),
                        });
                    }
                    changed = true;
                } else {
                    rewritten.push(AgentAction::CallTool { tool, args });
                }
            }
            other => rewritten.push(other),
        }
    }
    if changed {
        info!("plan_split_sequential_run_cmd_actions");
    }
    rewritten
}

fn run_cmd_args_async_start(args: &Value) -> bool {
    args.get("async_start").and_then(Value::as_bool) == Some(true)
}

pub(super) fn run_cmd_command_from_args(args: &Value) -> Option<&str> {
    args.get("command")
        .or_else(|| args.get("cmd"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

pub(super) fn run_cmd_args_for_rewritten_command(
    args: &Value,
    command: String,
    continue_on_error: bool,
) -> Value {
    let mut next_args = args.clone();
    if let Some(obj) = next_args.as_object_mut() {
        let key = if obj.contains_key("command") {
            "command"
        } else {
            "cmd"
        };
        obj.insert(key.to_string(), Value::String(command));
        if continue_on_error {
            obj.insert(
                super::super::CLAWD_CONTINUE_ON_ERROR_ARG.to_string(),
                Value::Bool(true),
            );
        } else {
            obj.remove(super::super::CLAWD_CONTINUE_ON_ERROR_ARG);
            obj.remove(super::super::CLAWD_LITERAL_COMMAND_ARG);
            obj.remove(super::super::CLAWD_LITERAL_FAILURE_REPAIRABLE_ARG);
        }
    }
    next_args
}

pub(super) fn fs_basic_read_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty()
        || enabled_skills.contains("fs_basic")
        || enabled_skills.contains("system_basic")
}

pub(super) fn command_has_shell_control_or_expansion(command: &str) -> bool {
    if command.contains('\n') || command.contains('\r') || command.contains('$') {
        return true;
    }
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            if quote != Some('\'') {
                escaped = true;
            }
            continue;
        }
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        if matches!(ch, '\'' | '"') {
            quote = Some(ch);
            continue;
        }
        if matches!(ch, '`' | '|' | ';' | '<' | '>' | '&') {
            return true;
        }
    }
    quote.is_some()
}

pub(super) fn parse_shell_line_count(raw: &str) -> Option<u64> {
    let value = raw.trim();
    if value.is_empty() || !value.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|n| (1..=500).contains(n))
}

pub(super) fn shell_file_path_token_is_safe(path: &str) -> bool {
    let path = path.trim();
    !path.is_empty()
        && path != "-"
        && !path.starts_with('~')
        && !path.contains('\0')
        && !path.contains('$')
        && !path
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
}

pub(super) fn absolutize_readonly_file_path_from_run_cmd_args(path: &str, args: &Value) -> String {
    let trimmed = path.trim();
    let candidate = Path::new(trimmed);
    if candidate.is_absolute() {
        return trimmed.to_string();
    }
    args.get("cwd")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|cwd| !cwd.is_empty())
        .map(|cwd| Path::new(cwd).join(candidate).display().to_string())
        .unwrap_or_else(|| trimmed.to_string())
}

pub(super) fn append_text_from_shell_command(command: &str) -> Option<(String, String)> {
    echo_text_redirect_from_shell_command(command).and_then(|redirect| {
        (redirect.action == "append_text").then_some((redirect.content, redirect.path))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EchoTextRedirect {
    action: &'static str,
    content: String,
    path: String,
}

fn echo_text_redirect_from_shell_command(command: &str) -> Option<EchoTextRedirect> {
    if command.contains('\n') || command.contains('\r') || command.contains('\0') {
        return None;
    }
    let words = shell_like_words(command);
    let executable = words.first().map(|word| command_basename(word))?;
    if !executable.eq_ignore_ascii_case("echo") {
        return None;
    }
    let redirect_idx = words.iter().position(|word| word == ">" || word == ">>")?;
    if redirect_idx < 2 || redirect_idx + 2 != words.len() {
        return None;
    }
    let action = if words[redirect_idx] == ">>" {
        "append_text"
    } else {
        "write_text"
    };
    let mut content_start = 1usize;
    let mut trailing_newline = true;
    if words.get(1).is_some_and(|word| word == "-n") {
        content_start = 2;
        trailing_newline = false;
    }
    if content_start >= redirect_idx {
        return None;
    }
    let mut content = words[content_start..redirect_idx].join(" ");
    if trailing_newline {
        content.push('\n');
    }
    let path = words.get(redirect_idx + 1)?.trim();
    if !shell_file_path_token_is_safe(path) {
        return None;
    }
    Some(EchoTextRedirect {
        action,
        content,
        path: path.to_string(),
    })
}

fn mkdir_path_from_shell_command(command: &str) -> Option<String> {
    if command_has_shell_control_or_expansion(command) {
        return None;
    }
    let words = shell_like_words(command);
    let executable = words.first().map(|word| command_basename(word))?;
    if !executable.eq_ignore_ascii_case("mkdir") {
        return None;
    }
    let mut paths = Vec::new();
    for word in words.iter().skip(1) {
        let word = word.trim();
        if word.is_empty() || matches!(word, "-p" | "--parents") {
            continue;
        }
        if word.starts_with('-') {
            return None;
        }
        paths.push(word.to_string());
    }
    if paths.len() != 1 || !shell_file_path_token_is_safe(&paths[0]) {
        return None;
    }
    Some(paths.remove(0))
}

fn cat_path_from_shell_command(command: &str) -> Option<String> {
    if command_has_shell_control_or_expansion(command) {
        return None;
    }
    let words = shell_like_words(command);
    let executable = words.first().map(|word| command_basename(word))?;
    if !executable.eq_ignore_ascii_case("cat") {
        return None;
    }
    let mut words = words
        .into_iter()
        .skip(1)
        .filter(|word| !word.trim().is_empty());
    let path = words.next()?;
    if words.next().is_some() || !shell_file_path_token_is_safe(&path) {
        return None;
    }
    Some(path)
}

fn rm_path_from_shell_command(command: &str) -> Option<(String, bool)> {
    if command_has_shell_control_or_expansion(command) {
        return None;
    }
    let words = shell_like_words(command);
    let executable = words.first().map(|word| command_basename(word))?;
    if !executable.eq_ignore_ascii_case("rm") {
        return None;
    }
    let mut recursive = false;
    let mut paths = Vec::new();
    for word in words.iter().skip(1) {
        let word = word.trim();
        if word.is_empty() {
            continue;
        }
        if word == "--" {
            return None;
        }
        if word == "-r" || word == "-R" || word == "--recursive" {
            recursive = true;
            continue;
        }
        if word == "-f" || word == "--force" {
            continue;
        }
        if word.starts_with('-') {
            let flags = word.trim_start_matches('-');
            if flags.is_empty() || !flags.chars().all(|ch| matches!(ch, 'r' | 'R' | 'f')) {
                return None;
            }
            if flags.chars().any(|ch| matches!(ch, 'r' | 'R')) {
                recursive = true;
            }
            continue;
        }
        paths.push(word.to_string());
    }
    if paths.len() != 1 || !shell_file_path_token_is_safe(&paths[0]) {
        return None;
    }
    Some((paths.remove(0), recursive))
}

fn simple_filesystem_action_from_shell_command(command: &str, args: &Value) -> Option<AgentAction> {
    if let Some(path) = mkdir_path_from_shell_command(command) {
        return Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "make_dir",
                "path": absolutize_readonly_file_path_from_run_cmd_args(&path, args),
            }),
        });
    }
    if let Some(redirect) = echo_text_redirect_from_shell_command(command) {
        return Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": redirect.action,
                "path": absolutize_readonly_file_path_from_run_cmd_args(&redirect.path, args),
                "content": redirect.content,
            }),
        });
    }
    if let Some(path) = cat_path_from_shell_command(command) {
        return Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": absolutize_readonly_file_path_from_run_cmd_args(&path, args),
                "mode": "head",
                "n": 500,
            }),
        });
    }
    if let Some((path, recursive)) = rm_path_from_shell_command(command) {
        let mut action_args = serde_json::json!({
            "action": "remove_path",
            "path": absolutize_readonly_file_path_from_run_cmd_args(&path, args),
        });
        if recursive {
            if let Some(obj) = action_args.as_object_mut() {
                obj.insert(
                    "target_kind".to_string(),
                    Value::String("directory".to_string()),
                );
                obj.insert("recursive".to_string(), Value::Bool(true));
            }
        }
        return Some(AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: action_args,
        });
    }
    None
}

pub(super) fn rewrite_simple_filesystem_run_cmd_to_fs_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state) {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallSkill { skill, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallSkill { skill, args };
                }
                let Some(next_action) = simple_filesystem_action_from_shell_command(command, &args)
                else {
                    return AgentAction::CallSkill { skill, args };
                };
                changed = true;
                next_action
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallTool { tool, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallTool { tool, args };
                }
                let Some(next_action) = simple_filesystem_action_from_shell_command(command, &args)
                else {
                    return AgentAction::CallTool { tool, args };
                };
                changed = true;
                next_action
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_rewrite_simple_filesystem_run_cmd_to_fs_basic");
    }
    rewritten
}

pub(super) fn rewrite_append_run_cmd_to_fs_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state) {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallSkill { skill, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallSkill { skill, args };
                }
                let Some((content, path)) = append_text_from_shell_command(command) else {
                    return AgentAction::CallSkill { skill, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "append_text",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "content": content,
                    }),
                }
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallTool { tool, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallTool { tool, args };
                }
                let Some((content, path)) = append_text_from_shell_command(command) else {
                    return AgentAction::CallTool { tool, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "append_text",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "content": content,
                    }),
                }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_rewrite_append_run_cmd_to_fs_basic");
    }
    rewritten
}

pub(super) fn readonly_file_read_from_shell_command(
    command: &str,
) -> Option<(&'static str, u64, String)> {
    if command_has_shell_control_or_expansion(command) {
        return None;
    }
    let words = shell_like_words(command);
    let executable = words.first().map(|word| command_basename(word))?;
    let mode = match executable.to_ascii_lowercase().as_str() {
        "head" => "head",
        "tail" => "tail",
        _ => return None,
    };
    let mut n = 10;
    let mut paths = Vec::new();
    let mut idx = 1;
    while idx < words.len() {
        let word = words[idx].trim();
        if word.is_empty() {
            idx += 1;
            continue;
        }
        if word == "--" {
            paths.extend(words.iter().skip(idx + 1).cloned());
            break;
        }
        if matches!(word, "-q" | "--quiet" | "--silent") {
            idx += 1;
            continue;
        }
        if matches!(word, "-n" | "--lines") {
            let value = words.get(idx + 1)?;
            n = parse_shell_line_count(value)?;
            idx += 2;
            continue;
        }
        if let Some(value) = word.strip_prefix("-n") {
            n = parse_shell_line_count(value)?;
            idx += 1;
            continue;
        }
        if let Some(value) = word.strip_prefix("--lines=") {
            n = parse_shell_line_count(value)?;
            idx += 1;
            continue;
        }
        if let Some(value) = word.strip_prefix('-') {
            n = parse_shell_line_count(value)?;
            idx += 1;
            continue;
        }
        paths.push(word.to_string());
        idx += 1;
    }
    if paths.len() != 1 || !shell_file_path_token_is_safe(&paths[0]) {
        return None;
    }
    Some((mode, n, paths.remove(0)))
}

pub(super) fn rewrite_readonly_file_read_run_cmd_to_fs_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state) {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallSkill { skill, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallSkill { skill, args };
                }
                let Some((mode, n, path)) = readonly_file_read_from_shell_command(command) else {
                    return AgentAction::CallSkill { skill, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "read_text_range",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "mode": mode,
                        "n": n,
                    }),
                }
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallTool { tool, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallTool { tool, args };
                }
                let Some((mode, n, path)) = readonly_file_read_from_shell_command(command) else {
                    return AgentAction::CallTool { tool, args };
                };
                changed = true;
                AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "read_text_range",
                        "path": absolutize_readonly_file_path_from_run_cmd_args(&path, &args),
                        "mode": mode,
                        "n": n,
                    }),
                }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_rewrite_readonly_file_read_run_cmd_to_fs_basic");
    }
    rewritten
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadonlyFindCommand {
    pub(super) root: String,
    pub(super) extension: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadonlyFindCountCommand {
    pub(super) root: String,
    pub(super) kind: ScalarCountInventoryKind,
    pub(super) recursive: bool,
    pub(super) extension: Option<String>,
}

pub(super) fn filesystem_find_route_prefers_structured_tool(
    route_result: Option<&RouteResult>,
) -> bool {
    route_result.is_some_and(|route| {
        !route.output_contract.delivery_required
            && matches!(
                route.effective_output_contract_semantic_kind(),
                crate::OutputSemanticKind::DirectoryNames
                    | crate::OutputSemanticKind::FileNames
                    | crate::OutputSemanticKind::FilePaths
            )
    })
}

pub(super) fn simple_shell_extension_pattern(pattern: &str) -> Option<String> {
    let pattern = pattern.trim();
    let candidate = pattern.strip_prefix("*.")?.trim();
    if candidate.is_empty()
        || candidate.contains('/')
        || candidate
            .chars()
            .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
    {
        return None;
    }
    Some(candidate.to_ascii_lowercase())
}

pub(super) fn readonly_find_extension_from_shell_command(
    command: &str,
) -> Option<ReadonlyFindCommand> {
    if command.contains('\n')
        || command.contains('\r')
        || command.contains('\0')
        || command.contains('`')
        || command.contains('<')
        || command.contains('>')
        || command.contains('&')
    {
        return None;
    }
    let words = shell_like_words(command);
    let pipe_index = words.iter().position(|word| word == "|");
    if let Some(index) = pipe_index {
        if !readonly_find_pipeline_suffix_is_supported(&words[index + 1..]) {
            return None;
        }
    }
    let find_words = match pipe_index {
        Some(index) => &words[..index],
        None => words.as_slice(),
    };
    if find_words
        .first()
        .map(|word| !command_basename(word).eq_ignore_ascii_case("find"))
        .unwrap_or(true)
    {
        return None;
    }
    let mut index = 1usize;
    let mut root = ".".to_string();
    if let Some(candidate) = find_words.get(index) {
        if !candidate.starts_with('-') {
            if !shell_file_path_token_is_safe(candidate) {
                return None;
            }
            root = candidate.to_string();
            index += 1;
        }
    }
    let mut extension = None;
    while index < find_words.len() {
        let word = find_words[index].as_str();
        match word {
            "-name" | "-iname" => {
                let pattern = find_words.get(index + 1)?;
                extension = Some(simple_shell_extension_pattern(pattern)?);
                index += 2;
            }
            "-type" => {
                if find_words.get(index + 1).map(String::as_str) != Some("f") {
                    return None;
                }
                index += 2;
            }
            "-maxdepth" | "-mindepth" => {
                find_words.get(index + 1)?;
                index += 2;
            }
            "-exec" => {
                let executable = find_words.get(index + 1)?;
                if !command_basename(executable).eq_ignore_ascii_case("dirname") {
                    return None;
                }
                let mut end = index + 2;
                while end < find_words.len() && find_words[end] != ";" {
                    end += 1;
                }
                if end >= find_words.len() {
                    return None;
                }
                index = end + 1;
            }
            _ => return None,
        }
    }
    Some(ReadonlyFindCommand {
        root,
        extension: extension?,
    })
}

pub(super) fn readonly_find_count_from_shell_command(
    command: &str,
) -> Option<ReadonlyFindCountCommand> {
    if command.contains('\n')
        || command.contains('\r')
        || command.contains('\0')
        || command.contains('`')
        || command.contains('<')
        || command.contains('>')
        || command.contains('&')
    {
        return None;
    }
    let words = shell_like_words(command);
    let pipe_index = words.iter().position(|word| word == "|")?;
    if !readonly_find_count_pipeline_suffix_is_supported(&words[pipe_index + 1..]) {
        return None;
    }
    let find_words = &words[..pipe_index];
    if find_words
        .first()
        .map(|word| !command_basename(word).eq_ignore_ascii_case("find"))
        .unwrap_or(true)
    {
        return None;
    }
    let mut index = 1usize;
    let mut root = ".".to_string();
    if let Some(candidate) = find_words.get(index) {
        if !candidate.starts_with('-') {
            if !shell_file_path_token_is_safe(candidate) {
                return None;
            }
            root = candidate.to_string();
            index += 1;
        }
    }
    let mut kind = ScalarCountInventoryKind::Any;
    let mut recursive = true;
    let mut extension = None;
    while index < find_words.len() {
        let word = find_words[index].as_str();
        match word {
            "-type" => {
                kind = match find_words.get(index + 1).map(String::as_str)? {
                    "f" => ScalarCountInventoryKind::Files,
                    "d" => ScalarCountInventoryKind::Dirs,
                    _ => return None,
                };
                index += 2;
            }
            "-maxdepth" => {
                let depth = find_words.get(index + 1)?.parse::<u64>().ok()?;
                recursive = depth > 1;
                index += 2;
            }
            "-mindepth" => {
                find_words.get(index + 1)?.parse::<u64>().ok()?;
                index += 2;
            }
            "-name" | "-iname" => {
                let pattern = find_words.get(index + 1)?;
                extension = Some(simple_shell_extension_pattern(pattern)?);
                index += 2;
            }
            "-print" => {
                index += 1;
            }
            _ => return None,
        }
    }
    if extension.is_some() && !matches!(kind, ScalarCountInventoryKind::Files) {
        return None;
    }
    Some(ReadonlyFindCountCommand {
        root,
        kind,
        recursive,
        extension,
    })
}

fn readonly_find_count_pipeline_suffix_is_supported(words: &[String]) -> bool {
    matches!(
        words,
        [cmd, flag]
            if command_basename(cmd).eq_ignore_ascii_case("wc")
                && matches!(flag.as_str(), "-l" | "--lines")
    )
}

fn fs_basic_count_entries_action_from_readonly_find_count(
    count: ReadonlyFindCountCommand,
) -> AgentAction {
    let mut args = serde_json::json!({
        "action": "count_entries",
        "path": count.root,
        "recursive": count.recursive,
    });
    if let Some(obj) = args.as_object_mut() {
        match count.kind {
            ScalarCountInventoryKind::Any => {}
            ScalarCountInventoryKind::Files => {
                obj.insert("kind_filter".to_string(), Value::String("file".to_string()));
                obj.insert("count_files".to_string(), Value::Bool(true));
                obj.insert("count_dirs".to_string(), Value::Bool(false));
                obj.insert("files_only".to_string(), Value::Bool(true));
                obj.insert("dirs_only".to_string(), Value::Bool(false));
            }
            ScalarCountInventoryKind::Dirs => {
                obj.insert("kind_filter".to_string(), Value::String("dir".to_string()));
                obj.insert("count_files".to_string(), Value::Bool(false));
                obj.insert("count_dirs".to_string(), Value::Bool(true));
                obj.insert("dirs_only".to_string(), Value::Bool(true));
                obj.insert("files_only".to_string(), Value::Bool(false));
            }
        }
        if let Some(extension) = count.extension {
            obj.insert("ext_filter".to_string(), Value::String(extension));
            obj.insert("kind_filter".to_string(), Value::String("file".to_string()));
            obj.insert("count_files".to_string(), Value::Bool(true));
            obj.insert("count_dirs".to_string(), Value::Bool(false));
            obj.insert("files_only".to_string(), Value::Bool(true));
            obj.insert("dirs_only".to_string(), Value::Bool(false));
        }
    }
    AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    }
}

pub(super) fn readonly_find_pipeline_suffix_is_supported(words: &[String]) -> bool {
    let segments = words
        .split(|word| word == "|")
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    match segments.as_slice() {
        [single] => {
            readonly_find_suffix_is_sort_unique(single)
                || readonly_find_suffix_is_parent_projection(single)
        }
        [project, sort] => {
            readonly_find_suffix_is_parent_projection(project)
                && readonly_find_suffix_is_sort_unique(sort)
        }
        _ => false,
    }
}

pub(super) fn readonly_find_suffix_is_sort_unique(words: &[String]) -> bool {
    matches!(
        words,
        [cmd, flag]
            if command_basename(cmd).eq_ignore_ascii_case("sort")
                && matches!(flag.as_str(), "-u" | "--unique")
    )
}

pub(super) fn readonly_find_suffix_is_parent_projection(words: &[String]) -> bool {
    match words {
        [cmd, expr] if command_basename(cmd).eq_ignore_ascii_case("sed") => {
            readonly_sed_parent_projection_expr(expr)
        }
        [cmd, flag, expr] if command_basename(cmd).eq_ignore_ascii_case("sed") && flag == "-e" => {
            readonly_sed_parent_projection_expr(expr)
        }
        [cmd, dirname] if command_basename(cmd).eq_ignore_ascii_case("xargs") => {
            command_basename(dirname).eq_ignore_ascii_case("dirname")
        }
        [cmd, n_flag, n_value, dirname]
            if command_basename(cmd).eq_ignore_ascii_case("xargs")
                && matches!(n_flag.as_str(), "-n" | "--max-args")
                && n_value == "1" =>
        {
            command_basename(dirname).eq_ignore_ascii_case("dirname")
        }
        [cmd, n_flag, dirname]
            if command_basename(cmd).eq_ignore_ascii_case("xargs") && n_flag == "-n1" =>
        {
            command_basename(dirname).eq_ignore_ascii_case("dirname")
        }
        _ => false,
    }
}

pub(super) fn readonly_sed_parent_projection_expr(expr: &str) -> bool {
    matches!(
        expr,
        "s|/[^/]*$||"
            | "s#/[^/]*$##"
            | "s,/[^/]*$,,"
            | "s|/[^/]*$|.|"
            | "s#/[^/]*$#.#"
            | "s,/[^/]*$,.,"
    )
}

pub(super) fn fs_basic_find_entries_extension_from_action(action: &AgentAction) -> Option<String> {
    let (name, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill.as_str(), args),
        AgentAction::CallTool { tool, args } => (tool.as_str(), args),
        _ => return None,
    };
    if !name.eq_ignore_ascii_case("fs_basic")
        || args.get("action").and_then(Value::as_str) != Some("find_entries")
    {
        return None;
    }
    args.get("extension")
        .or_else(|| args.get("ext"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|ext| !ext.is_empty())
        .map(|ext| ext.to_ascii_lowercase())
}

pub(super) fn rewrite_readonly_find_run_cmd_to_fs_basic(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state)
        || !filesystem_find_route_prefers_structured_tool(route_result)
    {
        return actions;
    }
    let existing_find_extensions = actions
        .iter()
        .filter_map(fs_basic_find_entries_extension_from_action)
        .collect::<Vec<_>>();
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .filter_map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return Some(AgentAction::CallSkill { skill, args });
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return Some(AgentAction::CallSkill { skill, args });
                }
                let Some(find) = readonly_find_extension_from_shell_command(command) else {
                    return Some(AgentAction::CallSkill { skill, args });
                };
                if existing_find_extensions
                    .iter()
                    .any(|ext| ext.eq_ignore_ascii_case(&find.extension))
                {
                    changed = true;
                    return None;
                }
                changed = true;
                Some(AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "find_entries",
                        "root": find.root,
                        "extension": find.extension,
                        "files_only": true,
                        "recursive": true,
                    }),
                })
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return Some(AgentAction::CallTool { tool, args });
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return Some(AgentAction::CallTool { tool, args });
                }
                let Some(find) = readonly_find_extension_from_shell_command(command) else {
                    return Some(AgentAction::CallTool { tool, args });
                };
                if existing_find_extensions
                    .iter()
                    .any(|ext| ext.eq_ignore_ascii_case(&find.extension))
                {
                    changed = true;
                    return None;
                }
                changed = true;
                Some(AgentAction::CallTool {
                    tool: "fs_basic".to_string(),
                    args: serde_json::json!({
                        "action": "find_entries",
                        "root": find.root,
                        "extension": find.extension,
                        "files_only": true,
                        "recursive": true,
                    }),
                })
            }
            other => Some(other),
        })
        .collect();
    if changed {
        info!("plan_rewrite_readonly_find_run_cmd_to_fs_basic");
    }
    rewritten
}

pub(super) fn rewrite_readonly_count_run_cmd_to_fs_basic(
    state: &AppState,
    user_text: &str,
    original_user_text: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !fs_basic_read_available_for_plan(state) {
        return actions;
    }
    let mut changed = false;
    let rewritten = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, args }
                if skill.trim().eq_ignore_ascii_case("run_cmd") =>
            {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallSkill { skill, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallSkill { skill, args };
                }
                let Some(count) = readonly_find_count_from_shell_command(command) else {
                    return AgentAction::CallSkill { skill, args };
                };
                changed = true;
                fs_basic_count_entries_action_from_readonly_find_count(count)
            }
            AgentAction::CallTool { tool, args } if tool.trim().eq_ignore_ascii_case("run_cmd") => {
                let Some(command) = run_cmd_command_from_args(&args) else {
                    return AgentAction::CallTool { tool, args };
                };
                if args
                    .get(super::super::CLAWD_LITERAL_COMMAND_ARG)
                    .and_then(Value::as_bool)
                    == Some(true)
                    || should_preserve_user_supplied_shell_command(
                        command,
                        user_text,
                        original_user_text,
                    )
                {
                    return AgentAction::CallTool { tool, args };
                }
                let Some(count) = readonly_find_count_from_shell_command(command) else {
                    return AgentAction::CallTool { tool, args };
                };
                changed = true;
                fs_basic_count_entries_action_from_readonly_find_count(count)
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_rewrite_readonly_count_run_cmd_to_fs_basic");
    }
    rewritten
}

pub(super) fn docker_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("docker_basic")
}

pub(super) fn docker_readonly_action_from_command(command: &str) -> Option<&'static str> {
    let words = shell_like_words(command);
    let first = words.first().map(|word| command_basename(word))?;
    let mut index = if first.eq_ignore_ascii_case("docker") {
        1
    } else if first.eq_ignore_ascii_case("sudo")
        && words
            .get(1)
            .map(|word| command_basename(word).eq_ignore_ascii_case("docker"))
            == Some(true)
    {
        2
    } else {
        return None;
    };
    while words
        .get(index)
        .is_some_and(|word| word.starts_with('-') && word != "-")
    {
        index += 1;
    }
    let subcommand = words.get(index)?.trim().to_ascii_lowercase();
    match subcommand.as_str() {
        "ps" => Some("ps"),
        "images" => Some("images"),
        "version" => Some("version"),
        "container" => match words.get(index + 1).map(|word| word.to_ascii_lowercase()) {
            Some(next) if matches!(next.as_str(), "ls" | "list" | "ps") => Some("ps"),
            _ => None,
        },
        "image" => match words.get(index + 1).map(|word| word.to_ascii_lowercase()) {
            Some(next) if matches!(next.as_str(), "ls" | "list") => Some("images"),
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn rewrite_docker_readonly_run_cmd_to_docker_basic(
    state: &AppState,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    if !docker_basic_available_for_plan(state) {
        return actions;
    }
    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if !action_skill_is_run_cmd(action) {
            continue;
        }
        let Some(docker_action) =
            run_cmd_command_arg(action).and_then(docker_readonly_action_from_command)
        else {
            continue;
        };
        *action = AgentAction::CallSkill {
            skill: "docker_basic".to_string(),
            args: serde_json::json!({
                "action": docker_action,
            }),
        };
        changed = true;
    }
    if changed {
        info!("plan_rewrite_docker_readonly_run_cmd_to_docker_basic");
    }
    rewritten
}

pub(super) fn action_is_path_metadata_facts_for_pair(
    action: &AgentAction,
    source: &str,
    archive: &str,
) -> bool {
    if !planned_action_is_path_metadata_facts(action) {
        return false;
    }
    let Some(args) = action_args(action).and_then(Value::as_object) else {
        return false;
    };
    let mut paths = string_list_from_value(args.get("paths").or_else(|| args.get("path")));
    paths.extend(string_list_from_value(args.get("targets")));
    if let Some(path) = args.get("left_path").and_then(Value::as_str) {
        paths.push(path.to_string());
    }
    if let Some(path) = args.get("right_path").and_then(Value::as_str) {
        paths.push(path.to_string());
    }
    paths
        .iter()
        .any(|path| path.ends_with(source) || path == source)
        && paths
            .iter()
            .any(|path| path.ends_with(archive) || path == archive)
}

pub(super) fn rewrite_archive_pack_plan_to_archive_basic(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    preserve_explicit_command: bool,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if preserve_explicit_command {
        return actions;
    }
    let Some(route) = route_result else {
        return actions;
    };
    let Some((source, archive)) = archive_pack_pair_for_route(route) else {
        return actions;
    };
    if archive_pack_observed_for_route(loop_state, &archive) {
        return actions;
    }
    if actions.iter().any(action_is_archive_basic_pack) {
        return actions;
    }

    let mut rewritten = actions;
    let mut changed = false;
    for action in rewritten.iter_mut() {
        if action_is_archive_basic(action)
            || action_is_path_metadata_facts_for_pair(action, &source, &archive)
            || action_skill_is_run_cmd(action)
        {
            *action = AgentAction::CallSkill {
                skill: "archive_basic".to_string(),
                args: serde_json::json!({
                    "action": "pack",
                    "source": source,
                    "archive": archive,
                    "format": archive_format_for_path(&archive),
                }),
            };
            changed = true;
            break;
        }
    }
    if !changed {
        return rewritten;
    }

    let mut saw_pack = false;
    let mut has_post_pack_synthesis = false;
    rewritten.retain(|action| {
        if action_is_archive_basic(action) {
            saw_pack = true;
            return true;
        }
        if saw_pack && matches!(action, AgentAction::SynthesizeAnswer { .. }) {
            has_post_pack_synthesis = true;
            return true;
        }
        if saw_pack && matches!(action, AgentAction::Respond { .. }) {
            return false;
        }
        true
    });
    if !has_post_pack_synthesis {
        rewritten.push(AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        });
    }
    info!("plan_rewrite_archive_pack_plan_to_archive_basic");
    rewritten
}

fn archive_pack_observed_for_route(loop_state: &LoopState, archive: &str) -> bool {
    loop_state.executed_step_results.iter().rev().any(|step| {
        if !step.is_ok() || step.skill != "archive_basic" {
            return false;
        }
        let Some(output) = step.output.as_deref() else {
            return false;
        };
        let Ok(value) = serde_json::from_str::<Value>(output.trim()) else {
            return false;
        };
        let action = value
            .get("action")
            .or_else(|| value.get("extra").and_then(|extra| extra.get("action")))
            .and_then(Value::as_str)
            .map(str::trim);
        if action != Some("pack") {
            return false;
        }
        ["archive", "archive_path", "path"].iter().any(|key| {
            value
                .get(*key)
                .or_else(|| value.get("extra").and_then(|extra| extra.get(*key)))
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|observed| archive_paths_match(observed, archive))
        })
    })
}

fn archive_paths_match(observed: &str, expected: &str) -> bool {
    let observed = observed.trim().replace('\\', "/");
    let expected = expected.trim().replace('\\', "/");
    if observed.is_empty() || expected.is_empty() {
        return false;
    }
    observed == expected
        || observed.ends_with(&format!("/{expected}"))
        || expected.ends_with(&format!("/{observed}"))
}

/// 检测 `respond.content` 是否是裸的 `{{last_output}}` / `{{last_output.xxx}}` /
/// `{{last_output[xxx]}}` 之类纯模板占位符。
///
/// 这种形态会被 `delivery_text_classifier` 判为 `non_informative_placeholder`，
/// 触发 `plan_missing_terminal_user_answer` 重修，进而陷入 vendor patch 都救不回来的死循环
/// （兼容模型在 short-answer 类 act 任务里可能反复踩这个坑，prompt 指令忠实度不够）。
pub(super) fn is_bare_last_output_placeholder(content: &str) -> bool {
    let trimmed = content.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len() - 2].trim();
    let lower = inner.to_ascii_lowercase();
    lower == "last_output" || lower.starts_with("last_output.") || lower.starts_with("last_output[")
}

pub(super) fn is_bare_template_placeholder(content: &str) -> bool {
    let trimmed = content.trim();
    if !trimmed.starts_with("{{") || !trimmed.ends_with("}}") {
        return false;
    }
    let inner = trimmed[2..trimmed.len() - 2].trim();
    !inner.is_empty() && !inner.contains("{{") && !inner.contains("}}")
}

pub(super) fn extract_output_placeholder_evidence_refs(text: &str) -> Vec<String> {
    static PLACEHOLDER_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    static PLACEHOLDER_REF_RE: OnceLock<Regex> = OnceLock::new();
    let block_re = PLACEHOLDER_BLOCK_RE
        .get_or_init(|| Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").expect("placeholder block regex"));
    let ref_re = PLACEHOLDER_REF_RE.get_or_init(|| {
        Regex::new(
            r"\b(last_output(?:[.\[][^\s{}]*)?|s\d+(?:[._]?output)?|step_?\d+(?:[._]?output)?)\b",
        )
        .expect("placeholder reference regex")
    });
    let mut refs = Vec::new();
    for block in block_re.captures_iter(text) {
        let Some(inner) = block.get(1) else {
            continue;
        };
        let mut found_ref = false;
        for captures in ref_re.captures_iter(inner.as_str()) {
            let Some(matched) = captures.get(1) else {
                continue;
            };
            let token = normalize_output_placeholder_reference(matched.as_str());
            if !refs.iter().any(|existing| existing == &token) {
                refs.push(token);
            }
            found_ref = true;
        }
        if !found_ref && !refs.iter().any(|existing| existing == "last_output") {
            refs.push("last_output".to_string());
        }
    }
    refs
}

pub(super) fn normalize_output_placeholder_reference(raw: &str) -> String {
    static STEP_UNDERSCORE_OUTPUT_RE: OnceLock<Regex> = OnceLock::new();
    static STEP_BARE_RE: OnceLock<Regex> = OnceLock::new();
    static S_UNDERSCORE_OUTPUT_RE: OnceLock<Regex> = OnceLock::new();
    let lower = raw.trim().to_ascii_lowercase();
    if lower.starts_with("last_output.") || lower.starts_with("last_output[") {
        return "last_output".to_string();
    }
    let step_underscore_output_re = STEP_UNDERSCORE_OUTPUT_RE.get_or_init(|| {
        Regex::new(r"^step_?(\d+)_output$").expect("step output placeholder regex")
    });
    if let Some(captures) = step_underscore_output_re.captures(&lower) {
        if let Some(number) = captures.get(1) {
            return format!("step_{}", number.as_str());
        }
    }
    let step_bare_re =
        STEP_BARE_RE.get_or_init(|| Regex::new(r"^step_?(\d+)$").expect("step placeholder regex"));
    if let Some(captures) = step_bare_re.captures(&lower) {
        if let Some(number) = captures.get(1) {
            return format!("step_{}", number.as_str());
        }
    }
    let s_underscore_output_re = S_UNDERSCORE_OUTPUT_RE
        .get_or_init(|| Regex::new(r"^s(\d+)_output$").expect("short step output regex"));
    if let Some(captures) = s_underscore_output_re.captures(&lower) {
        if let Some(number) = captures.get(1) {
            return format!("s{}", number.as_str());
        }
    }
    lower
}

pub(super) fn has_loop_observation(loop_state: &LoopState) -> bool {
    loop_state.has_tool_or_skill_output
        || !loop_state.executed_step_results.is_empty()
        || loop_state
            .last_output
            .as_deref()
            .map(str::trim)
            .is_some_and(|text| !text.is_empty())
}

pub(super) fn is_concrete_final_respond_content(content: &str) -> bool {
    let trimmed = content.trim();
    !trimmed.is_empty()
        && !is_bare_last_output_placeholder(trimmed)
        && extract_output_placeholder_evidence_refs(trimmed).is_empty()
}

pub(super) fn route_should_prefer_observed_terminal_synthesis(route: Option<&RouteResult>) -> bool {
    let Some(route) = route else {
        return false;
    };
    if route.output_contract_marker_is(crate::OutputSemanticKind::ServiceStatus) {
        return false;
    }
    route.output_contract.requires_content_evidence
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::OneSentence | crate::OutputResponseShape::Strict
        )
}

pub(super) fn textual_grounding_token_has_signal(token: &str) -> bool {
    let token = token.trim();
    if token.len() < 2 {
        return false;
    }
    let has_ascii_alpha = token.chars().any(|ch| ch.is_ascii_alphabetic());
    let uppercase_count = token.chars().filter(|ch| ch.is_ascii_uppercase()).count();
    let lowercase_count = token.chars().filter(|ch| ch.is_ascii_lowercase()).count();
    let digit_count = token.chars().filter(|ch| ch.is_ascii_digit()).count();
    if token.contains('.') || token.contains('_') || token.contains('-') || token.contains('/') {
        return token.len() >= 3;
    }
    if digit_count > 0 && has_ascii_alpha && token.len() >= 3 {
        return true;
    }
    if uppercase_count >= 2 && token.len() <= 16 {
        return true;
    }
    uppercase_count >= 1
        && lowercase_count >= 1
        && token.chars().skip(1).any(|ch| ch.is_ascii_uppercase())
}

pub(super) fn push_textual_grounding_tokens(raw: &str, out: &mut Vec<String>) {
    static TOKEN_RE: OnceLock<Regex> = OnceLock::new();
    let re = TOKEN_RE.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9][A-Za-z0-9._/-]{1,63}").expect("valid text token regex")
    });
    for token in re.find_iter(raw).map(|m| m.as_str()) {
        if textual_grounding_token_has_signal(token) {
            out.push(token.to_string());
        }
    }
}

pub(super) fn push_structural_grounding_tokens(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::Null | Value::Bool(_) => {}
        Value::Number(number) => {
            let token = number.to_string();
            if token.chars().filter(|ch| ch.is_ascii_digit()).count() >= 2 {
                out.push(token);
            }
        }
        Value::String(raw) => {
            let token = raw.trim().replace('\\', "/");
            push_textual_grounding_tokens(&token, out);
            if token.len() < 3 || token.chars().any(char::is_whitespace) && !token.contains('/') {
                return;
            }
            let has_structural_shape = token.contains('/')
                || token.contains('.')
                || token.contains('_')
                || token.contains('-')
                || token.chars().all(|ch| ch.is_ascii_digit());
            if !has_structural_shape {
                return;
            }
            out.push(token.clone());
            if let Some(basename) = token.rsplit('/').next().filter(|part| part.len() >= 3) {
                out.push(basename.to_string());
            }
        }
        Value::Array(items) => {
            for item in items {
                push_structural_grounding_tokens(item, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                push_structural_grounding_tokens(value, out);
            }
        }
    }
}
