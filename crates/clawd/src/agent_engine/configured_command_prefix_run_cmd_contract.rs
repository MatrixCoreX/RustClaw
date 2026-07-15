use super::*;

pub(super) fn annotate_readonly_cli_surface_run_cmds(
    state: &AppState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let mut changed = false;
    let actions = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if state.resolve_canonical_skill_name(&skill) == "run_cmd"
                    && annotate_readonly_cli_surface_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if state.resolve_canonical_skill_name(&tool) == "run_cmd"
                    && annotate_readonly_cli_surface_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_annotate_run_cmd_readonly_cli_surface");
    }
    actions
}

fn annotate_readonly_cli_surface_args(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    if obj.get("action").and_then(Value::as_str).is_some() {
        return false;
    }
    let Some(command) = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
    else {
        return false;
    };
    if !command_looks_like_readonly_cli_surface_probe(command) {
        return false;
    }
    obj.insert(
        "action".to_string(),
        Value::String("inspect_cli_help".to_string()),
    );
    obj.entry("timeout_seconds".to_string())
        .or_insert_with(|| Value::Number(10.into()));
    obj.entry("idle_timeout_seconds".to_string())
        .or_insert_with(|| Value::Number(5.into()));
    obj.entry("max_output_bytes".to_string())
        .or_insert_with(|| Value::Number(24000.into()));
    true
}

fn command_looks_like_readonly_cli_surface_probe(command: &str) -> bool {
    let command = command.trim();
    if command.is_empty() {
        return false;
    }
    let lower = command.to_ascii_lowercase();
    if command_contains_forbidden_cli_probe_token(&lower) {
        return false;
    }
    let tokens = shell_word_tokens(&lower);
    lower.contains("--help")
        || lower.contains(" -h")
        || lower.contains("--version")
        || lower.contains(" -v")
        || tokens
            .first()
            .is_some_and(|token| matches!(*token, "which" | "type"))
        || tokens
            .windows(2)
            .any(|pair| matches!(pair, ["command", "-v"] | ["command", "v"]))
}

fn command_contains_forbidden_cli_probe_token(command_lower: &str) -> bool {
    let forbidden = [
        "rm",
        "mv",
        "cp",
        "mkdir",
        "touch",
        "truncate",
        "install",
        "chmod",
        "chown",
        "ln",
        "sudo",
        "tee",
        "sed",
        "perl",
        "python",
        "python3",
        "node",
        "npm",
        "pnpm",
        "yarn",
        "bash",
        "sh",
        "zsh",
        "fish",
        "systemctl",
        "service",
        "kill",
        "pkill",
        "curl",
        "wget",
        "nc",
        "ssh",
        "scp",
        "rsync",
    ];
    let tokens = shell_word_tokens(command_lower);
    tokens
        .iter()
        .any(|token| forbidden.iter().any(|forbidden| token == forbidden))
}

fn shell_word_tokens(command_lower: &str) -> Vec<&str> {
    command_lower
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')))
        .filter(|token| !token.is_empty())
        .collect()
}

pub(super) fn ensure_clawcli_resume_surface_help_for_required_machine_field(
    state: &AppState,
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !run_cmd_available_for_plan(state)
        || !clawcli_resume_required_machine_surface_requested(
            route_result,
            user_text,
            original_user_text,
            plan_context,
        )
    {
        return actions;
    }
    if actions
        .iter()
        .any(|action| run_cmd_action_has_clawcli_resume_help(state, action))
    {
        return ensure_terminal_last_output_delivery(actions);
    }
    info!("plan_inject_clawcli_resume_help_for_required_machine_field");
    clawcli_resume_surface_help_actions()
}

fn clawcli_resume_required_machine_surface_requested(
    route_result: Option<&RouteResult>,
    user_text: &str,
    original_user_text: Option<&str>,
    plan_context: Option<&str>,
) -> bool {
    if route_result.is_some_and(route_marks_clawcli_resume_surface) {
        return true;
    }
    [Some(user_text), original_user_text, plan_context]
        .into_iter()
        .flatten()
        .any(clawcli_resume_required_machine_tokens_present)
}

fn route_marks_clawcli_resume_surface(route: &RouteResult) -> bool {
    let markers = crate::RouteReasonMarkers::new(&route.route_reason);
    markers
        .machine_value("surface")
        .is_some_and(|value| value.eq_ignore_ascii_case("clawcli"))
        && markers
            .machine_value("subcommand")
            .is_some_and(|value| value.eq_ignore_ascii_case("resume"))
}

fn clawcli_resume_required_machine_tokens_present(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let tokens = shell_word_tokens(&lower);
    tokens.iter().any(|token| clawcli_token(token))
        && tokens.iter().any(|token| *token == "resume")
        && tokens.iter().any(|token| *token == "resume_task_id")
}

fn run_cmd_action_has_clawcli_resume_help(state: &AppState, action: &AgentAction) -> bool {
    let Some(command) = run_cmd_action_command(state, action) else {
        return false;
    };
    command_is_clawcli_resume_help(command)
}

fn run_cmd_action_command<'a>(state: &AppState, action: &'a AgentAction) -> Option<&'a str> {
    if !action_is_run_cmd(state, action) {
        return None;
    }
    let args = match action {
        AgentAction::CallSkill { args, .. } | AgentAction::CallTool { args, .. } => args,
        AgentAction::CallCapability { .. }
        | AgentAction::SynthesizeAnswer { .. }
        | AgentAction::Respond { .. }
        | AgentAction::Think { .. } => return None,
    };
    args.get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
}

fn command_is_clawcli_resume_help(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let tokens = shell_word_tokens(&lower);
    tokens.iter().any(|token| clawcli_token(token))
        && tokens.iter().any(|token| *token == "resume")
        && (tokens.iter().any(|token| *token == "--help")
            || tokens.iter().any(|token| *token == "-h"))
}

fn clawcli_token(token: &str) -> bool {
    token == "clawcli" || token == "clawcli.sh"
}

fn ensure_terminal_last_output_delivery(mut actions: Vec<AgentAction>) -> Vec<AgentAction> {
    if !actions
        .iter()
        .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        actions.push(AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        });
    }
    if !actions
        .iter()
        .any(|action| matches!(action, AgentAction::Respond { .. }))
    {
        actions.push(AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        });
    }
    actions
}

fn clawcli_resume_surface_help_actions() -> Vec<AgentAction> {
    vec![
        AgentAction::CallTool {
            tool: "run_cmd".to_string(),
            args: serde_json::json!({
                "action": "inspect_cli_help",
                "command": "scripts/clawcli.sh resume --help",
                "timeout_seconds": 10,
                "idle_timeout_seconds": 5,
                "max_output_bytes": 24000
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec!["last_output".to_string()],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ]
}

pub(super) fn ensure_run_cmd_async_start_for_runtime_async_job_contract(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !crate::async_job_contract::route_requests_runtime_async_job_contract(route) {
        return actions;
    }
    let mut changed = false;
    let actions = actions
        .into_iter()
        .map(|action| match action {
            AgentAction::CallSkill { skill, mut args } => {
                if state.resolve_canonical_skill_name(&skill) == "run_cmd"
                    && ensure_run_cmd_async_start_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallSkill { skill, args }
            }
            AgentAction::CallTool { tool, mut args } => {
                if state.resolve_canonical_skill_name(&tool) == "run_cmd"
                    && ensure_run_cmd_async_start_args(&mut args)
                {
                    changed = true;
                }
                AgentAction::CallTool { tool, args }
            }
            other => other,
        })
        .collect();
    if changed {
        info!("plan_inject_run_cmd_async_start_for_async_job_contract");
    }
    actions
}

fn ensure_run_cmd_async_start_args(args: &mut Value) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    let Some(command) = obj
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
    else {
        return false;
    };
    if run_cmd_command_claims_runtime_async_metadata(command) {
        return false;
    }
    let mut changed = false;
    if obj.get("async_start").and_then(Value::as_bool) != Some(true) {
        obj.insert("async_start".to_string(), Value::Bool(true));
        changed = true;
    }
    if !obj.contains_key("poll_after_seconds") {
        obj.insert("poll_after_seconds".to_string(), Value::from(2));
        changed = true;
    }
    if !obj.contains_key("expires_in_seconds") {
        obj.insert("expires_in_seconds".to_string(), Value::from(600));
        changed = true;
    }
    if obj
        .get(crate::agent_engine::CLAWD_RUNTIME_ASYNC_JOB_START_ARG)
        .and_then(Value::as_str)
        != Some("async_job_protocol")
    {
        obj.insert(
            crate::agent_engine::CLAWD_RUNTIME_ASYNC_JOB_START_ARG.to_string(),
            Value::String("async_job_protocol".to_string()),
        );
        changed = true;
    }
    changed
}

fn run_cmd_command_claims_runtime_async_metadata(command: &str) -> bool {
    let command = command.to_ascii_lowercase();
    [
        "checkpoint_id",
        "poll_ref",
        "next_check_after",
        "status=background",
        "pending_async_job",
        "job_id=",
    ]
    .iter()
    .any(|token| command.contains(token))
}

pub(super) fn rewrite_backend_identity_metadata_respond_to_runtime_identity(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_reason_has_backend_identity_metadata_marker(route) {
        return actions;
    }
    let [AgentAction::Respond { content }] = actions.as_slice() else {
        return actions;
    };
    if !respond_content_mentions_backend_identity_metadata(state, content) {
        return actions;
    }
    info!("plan_rewrite_backend_identity_metadata_respond_to_runtime_identity");
    vec![AgentAction::Respond {
        content: state.agent_runtime_identity_label().to_string(),
    }]
}

fn route_reason_has_backend_identity_metadata_marker(route: &RouteResult) -> bool {
    route_reason_has_structural_marker(route, "agent_display_name_hint_backend_metadata_removed")
}

fn respond_content_mentions_backend_identity_metadata(state: &AppState, content: &str) -> bool {
    let normalized_content = normalize_backend_identity_token(content);
    if normalized_content.is_empty() {
        return false;
    }
    state.core.llm_providers.iter().any(|provider| {
        provider
            .config
            .name
            .trim()
            .strip_prefix("vendor-")
            .into_iter()
            .chain([
                provider.config.name.trim(),
                provider.config.model.trim(),
                provider.config.provider_type.trim(),
            ])
            .map(normalize_backend_identity_token)
            .filter(|token| token.len() >= 4)
            .any(|token| normalized_content.contains(&token))
    })
}

fn normalize_backend_identity_token(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}
