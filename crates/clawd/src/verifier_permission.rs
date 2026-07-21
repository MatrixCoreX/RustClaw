use std::path::{Component, Path};

use serde_json::{json, Value};

use super::*;

fn step_permission_decision_json(
    state: &AppState,
    step: &PlanStep,
    planner_requested_approval: bool,
) -> Value {
    if !matches!(step.action_type.as_str(), "call_skill" | "call_tool") {
        return json!({
            "step_id": step.step_id,
            "action_type": step.action_type,
            "executable": false,
            "decision": crate::policy_decision::PolicyDecision::Deny.as_token(),
        });
    }

    let normalized_skill = state.resolve_canonical_skill_name(&step.skill);
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, &normalized_skill, &step.args);
    let risk_level = effective_step_risk_level(state, &normalized_skill, &step.args);
    let dry_run_observe_only =
        crate::execution_recipe::dry_run_observes_only_action(&normalized_skill, &step.args);
    let registry_non_mutating_action =
        registry_declares_non_mutating_planner_action(state, &normalized_skill, &step.args);
    let autonomous_workspace_fs_mutation =
        workspace_filesystem_mutation_can_run_autonomously(state, &normalized_skill, &step.args);
    let autonomous_validation_run_cmd =
        validation_run_cmd_can_run_autonomously(state, &normalized_skill, &step.args);
    let risk_requires_confirmation = !dry_run_observe_only
        && !registry_non_mutating_action
        && !autonomous_workspace_fs_mutation
        && !autonomous_validation_run_cmd
        && (state
            .skill_invocation_requires_confirmation_policy(&normalized_skill, Some(&step.args))
            || is_confirmation_like_skill(&normalized_skill)
            || high_risk_side_effect_requires_confirmation(effect, risk_level, &step.args));
    let action = step
        .args
        .as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(|value| value.as_str())
        .map(normalize_schema_token);
    let registry_policy = state
        .skill_manifest(&normalized_skill)
        .and_then(|manifest| {
            let mapping = claw_core::skill_registry::select_planner_capability_mapping(
                &manifest.planner_capabilities,
                action.as_deref(),
            )?;
            let policy = json!({
                "capability": mapping.name,
                "effect": mapping.effect.map(|effect| effect.as_token()),
                "risk_level": mapping.risk_level.map(risk_level_token),
                "isolation_profile": mapping.isolation_profile.map(|profile| profile.as_token()),
                "network_access": mapping.network_access,
                "filesystem_write": mapping.filesystem_write,
                "external_publish": mapping.external_publish,
                "credential_access": mapping.credential_access,
                "subprocess": mapping.subprocess,
                "package_install": mapping.package_install,
                "privilege_escalation": mapping.privilege_escalation,
                "once_per_task": mapping.once_per_task,
                "dedup_scope": mapping.dedup_scope.map(|scope| scope.as_token()),
                "idempotent": mapping.idempotent,
            });
            Some(policy)
        })
        .or_else(|| {
            state
                .mcp_tool(&normalized_skill)
                .map(|tool| tool.policy.permission_policy_json())
        });
    let requires_confirmation = state.skill_rt.tools_policy.approval_required(
        risk_requires_confirmation,
        planner_requested_approval,
        effect.mutates
            || registry_policy.as_ref().is_some_and(|policy| {
                policy.get("external_publish").and_then(Value::as_bool) == Some(true)
            }),
    );
    let sandbox_denial_reason =
        sandbox_denial_reason(state, &normalized_skill, effect, registry_policy.as_ref());
    let decision = if sandbox_denial_reason.is_some() {
        crate::policy_decision::PolicyDecision::Deny
    } else if requires_confirmation {
        crate::policy_decision::PolicyDecision::RequireConfirmation
    } else {
        crate::policy_decision::PolicyDecision::Allow
    };
    let sandbox_profile = verifier_sandbox_profile_token(registry_policy.as_ref(), effect);
    let sandbox_network = if registry_policy
        .as_ref()
        .and_then(|policy| policy.get("network_access"))
        .and_then(Value::as_bool)
        == Some(true)
    {
        crate::process_sandbox::ProcessNetworkPolicy::Inherit
    } else {
        crate::process_sandbox::ProcessNetworkPolicy::Deny
    };
    let sandbox_backend_diagnostics = crate::process_sandbox::sandbox_backend_diagnostics(
        state.skill_rt.tools_policy.sandbox_backend,
        state.skill_rt.tools_policy.sandbox_mode,
        sandbox_network,
    );

    json!({
        "step_id": step.step_id,
        "action_type": step.action_type,
        "executable": true,
        "decision": decision.as_token(),
        "skill": normalized_skill,
        "action": action,
        "action_effect": {
            "observes": effect.observes,
            "mutates": effect.mutates,
            "validates": effect.validates,
        },
        "risk_level": risk_level_token(risk_level),
        "requires_confirmation": requires_confirmation,
        "approval_policy": state.skill_rt.tools_policy.approval_policy_token(),
        "global_sandbox_mode": state.skill_rt.tools_policy.sandbox_mode_token(),
        "global_sandbox_backend": state.skill_rt.tools_policy.sandbox_backend_token(),
        "sandbox_backend_diagnostics": sandbox_backend_diagnostics,
        "sandbox_denial_reason": sandbox_denial_reason,
        "sandbox_profile": sandbox_profile.clone(),
        "sandbox": verifier_sandbox_summary(registry_policy.as_ref(), &sandbox_profile, effect),
        "workspace_scope": verifier_workspace_scope_summary(
            &step.args,
            &state.skill_rt.workspace_root,
            crate::execution_recipe::action_targets_external_workspace(
                state,
                &normalized_skill,
                &step.args
            )
        ),
        "registry_policy": registry_policy,
    })
}

pub(super) fn preview_command_permission_decision_json(
    state: &AppState,
    command: &str,
    cwd: Option<&str>,
    sudo_allowed: bool,
) -> Value {
    let mut args = serde_json::Map::from_iter([(
        "command".to_string(),
        Value::String(command.trim().to_string()),
    )]);
    if let Some(cwd) = cwd.map(str::trim).filter(|value| !value.is_empty()) {
        args.insert("cwd".to_string(), Value::String(cwd.to_string()));
    }
    let step = PlanStep {
        step_id: "permission_preflight".to_string(),
        action_type: "call_tool".to_string(),
        skill: "run_cmd".to_string(),
        args: Value::Object(args),
        depends_on: Vec::new(),
        why: String::new(),
    };
    let raw = step_permission_decision_json(state, &step, false);
    let raw_decision = raw
        .get("decision")
        .and_then(Value::as_str)
        .and_then(crate::policy_decision::PolicyDecision::parse_token)
        .unwrap_or(crate::policy_decision::PolicyDecision::Deny);
    let mut decision = raw_decision;
    let mut reason_codes = Vec::new();

    if command.trim().is_empty() {
        decision = crate::policy_decision::PolicyDecision::Deny;
        reason_codes.push("command_missing");
    } else if command.len() > state.skill_rt.max_cmd_length {
        decision = crate::policy_decision::PolicyDecision::Deny;
        reason_codes.push("command_too_long");
    }
    if !sudo_allowed && crate::skills::command_requests_sudo(command) {
        decision = crate::policy_decision::PolicyDecision::Deny;
        reason_codes.push("sudo_not_allowed");
    }
    if let Some(reason) = raw.get("sandbox_denial_reason").and_then(Value::as_str) {
        if !reason_codes.contains(&reason) {
            reason_codes.push(reason);
        }
    }
    if reason_codes.is_empty() {
        reason_codes.push(match decision {
            crate::policy_decision::PolicyDecision::Allow => "policy_allowed",
            crate::policy_decision::PolicyDecision::RequireConfirmation => "confirmation_required",
            crate::policy_decision::PolicyDecision::Deny => "policy_denied",
            crate::policy_decision::PolicyDecision::BackgroundWait => "background_wait",
        });
    }

    json!({
        "schema_version": 1,
        "status_code": "permission_preflight_complete",
        "action": "preview_command_permission",
        "command": command.trim(),
        "dry_run": true,
        "preview_only": true,
        "target_skill": "run_cmd",
        "target_action": "run_command",
        "decision": decision.as_token(),
        "risk_level": raw.get("risk_level").cloned().unwrap_or(Value::String("unknown".to_string())),
        "confirmation_required": decision.requires_confirmation(),
        "approval_policy": raw.get("approval_policy").cloned().unwrap_or(Value::Null),
        "sandbox_mode": raw.get("global_sandbox_mode").cloned().unwrap_or(Value::Null),
        "sandbox_backend": raw.get("global_sandbox_backend").cloned().unwrap_or(Value::Null),
        "sandbox_backend_diagnostics": raw
            .get("sandbox_backend_diagnostics")
            .cloned()
            .unwrap_or(Value::Null),
        "sandbox_profile": raw.get("sandbox_profile").cloned().unwrap_or(Value::Null),
        "sandbox": raw.get("sandbox").cloned().unwrap_or(Value::Null),
        "workspace_scope": raw.get("workspace_scope").cloned().unwrap_or(Value::Null),
        "reason_codes": reason_codes,
        "would_execute": false,
    })
}

pub(super) fn verify_permission_decision_json(
    state: &AppState,
    plan_result: &PlanResult,
    mode: VerifyMode,
    approved: bool,
    needs_confirmation: bool,
    blocked_reason: Option<&str>,
    shadow_blocked_reason: Option<&str>,
    issues: &[VerifyIssue],
) -> Value {
    let first_issue = first_blocking_issue(issues);
    let denied_by_policy = issues
        .iter()
        .any(|issue| issue_is_policy_denial(issue.kind));
    let decision = crate::policy_decision::PolicyDecision::from_permission_flags(
        approved,
        needs_confirmation,
        denied_by_policy,
        false,
    );
    json!({
        "schema_version": 1,
        "owner_layer": "plan_verifier",
        "mode": mode.as_str(),
        "decision": decision.as_token(),
        "allowed": approved && !needs_confirmation,
        "approved": approved,
        "needs_confirmation": needs_confirmation,
        "denied_by_policy": denied_by_policy,
        "dry_run_required": false,
        "external_provider_blocked": false,
        "reason_code": first_issue
            .map(|issue| issue.kind.reason_code())
            .unwrap_or("verify_allowed"),
        "status_code": first_issue
            .map(|issue| issue.kind.status_code())
            .unwrap_or("allowed"),
        "message_key": first_issue
            .map(|issue| issue.kind.message_key())
            .unwrap_or("clawd.verify.allowed"),
        "issue_count": issues.len(),
        "blocked_reason_present": blocked_reason
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        "shadow_blocked_reason_present": shadow_blocked_reason
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false),
        "steps": plan_result
            .steps
            .iter()
            .map(|step| step_permission_decision_json(state, step, plan_result.needs_confirmation))
            .collect::<Vec<_>>(),
    })
}

pub(super) fn step_sandbox_denial_reason(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> Option<&'static str> {
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args);
    let action = args
        .get("action")
        .and_then(Value::as_str)
        .map(normalize_schema_token);
    let registry_policy = state
        .skill_manifest(normalized_skill)
        .and_then(|manifest| {
            claw_core::skill_registry::select_planner_capability_mapping(
                &manifest.planner_capabilities,
                action.as_deref(),
            )
            .map(|mapping| {
                json!({
                    "isolation_profile": mapping.isolation_profile.map(|profile| profile.as_token()),
                    "network_access": mapping.network_access,
                    "filesystem_write": mapping.filesystem_write,
                    "external_publish": mapping.external_publish,
                    "credential_access": mapping.credential_access,
                    "subprocess": mapping.subprocess,
                    "package_install": mapping.package_install,
                    "privilege_escalation": mapping.privilege_escalation,
                })
            })
        })
        .or_else(|| {
            state
                .mcp_tool(normalized_skill)
                .map(|tool| tool.policy.permission_policy_json())
        });
    sandbox_denial_reason(state, normalized_skill, effect, registry_policy.as_ref())
}

fn sandbox_denial_reason(
    state: &AppState,
    normalized_skill: &str,
    effect: crate::execution_recipe::ActionEffect,
    registry_policy: Option<&Value>,
) -> Option<&'static str> {
    let bool_field = |key: &str| {
        registry_policy
            .and_then(|policy| policy.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    };
    let run_cmd = normalized_skill == "run_cmd";
    state
        .skill_rt
        .tools_policy
        .sandbox_denial(crate::runtime::policy::SandboxRequirements {
            mutates: effect.mutates,
            network_access: !run_cmd && bool_field("network_access"),
            filesystem_write: if run_cmd {
                effect.mutates
            } else {
                bool_field("filesystem_write")
            },
            external_publish: !run_cmd && bool_field("external_publish"),
            credential_access: !run_cmd && bool_field("credential_access"),
            subprocess: run_cmd || bool_field("subprocess"),
            package_install: bool_field("package_install"),
            privilege_escalation: bool_field("privilege_escalation"),
            isolation_profile: registry_policy
                .and_then(|policy| policy.get("isolation_profile"))
                .and_then(Value::as_str),
        })
}

pub(super) fn audit_permission_decision(
    state: &AppState,
    task: &ClaimedTask,
    permission_decision: &Value,
) {
    let detail = json!({
        "task_id": task.task_id,
        "permission_decision": permission_decision,
    })
    .to_string();
    if let Err(err) = crate::repo::insert_audit_log(
        state,
        Some(task.user_id),
        "plan_verifier.permission_decision",
        Some(&detail),
        None,
    ) {
        tracing::warn!(error = %err, "plan_verifier_permission_decision_audit_failed");
    }
}

fn fs_basic_action(args: &Value) -> Option<String> {
    args.as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .map(normalize_schema_token)
        .filter(|action| !action.is_empty())
}

pub(super) fn workspace_filesystem_mutation_can_run_autonomously(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    if normalized_skill == "run_cmd" {
        return workspace_coding_run_cmd_can_run_autonomously(state, args);
    }
    if normalized_skill != "fs_basic" {
        return false;
    }
    if fs_basic_action(args).as_deref() == Some("apply_patch") {
        return workspace_patch_can_run_autonomously(state, args);
    }
    if !matches!(
        fs_basic_action(args).as_deref(),
        Some("make_dir" | "write_text" | "append_text")
    ) {
        return false;
    }
    path_args(args)
        .into_iter()
        .any(|path| path_value_is_workspace_scoped(path, &state.skill_rt.workspace_root))
}

fn workspace_patch_can_run_autonomously(state: &AppState, args: &Value) -> bool {
    if !matches!(
        state.skill_rt.tools_policy.sandbox_mode,
        claw_core::config::ToolSandboxMode::WorkspaceWrite
            | claw_core::config::ToolSandboxMode::IsolatedWorktree
    ) {
        return false;
    }
    let Some(patch) = args
        .get("patch")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|patch| !patch.is_empty())
    else {
        return false;
    };
    let mut saw_workspace_target = false;
    for line in patch.lines() {
        let Some(raw_path) = line
            .strip_prefix("--- ")
            .or_else(|| line.strip_prefix("+++ "))
        else {
            continue;
        };
        let raw_path = raw_path.split('\t').next().unwrap_or(raw_path).trim();
        if raw_path == "/dev/null" {
            continue;
        }
        if raw_path.starts_with('"') || raw_path.ends_with('"') {
            return false;
        }
        let path = raw_path
            .strip_prefix("a/")
            .or_else(|| raw_path.strip_prefix("b/"))
            .unwrap_or(raw_path);
        if path.is_empty() || !path_value_is_workspace_scoped(path, &state.skill_rt.workspace_root)
        {
            return false;
        }
        saw_workspace_target = true;
    }
    saw_workspace_target
}

fn workspace_coding_run_cmd_can_run_autonomously(state: &AppState, args: &Value) -> bool {
    if !matches!(
        state.skill_rt.tools_policy.sandbox_mode,
        claw_core::config::ToolSandboxMode::WorkspaceWrite
            | claw_core::config::ToolSandboxMode::IsolatedWorktree
    ) || crate::execution_recipe::action_targets_external_workspace(state, "run_cmd", args)
    {
        return false;
    }
    let Some(command) = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return false;
    };
    let effect = crate::execution_recipe::classify_skill_action_effect(state, "run_cmd", args);
    !crate::skills::command_requests_sudo(command)
        && effect.mutates
        && autonomous_workspace_coding_shell_shape(command)
}

fn autonomous_workspace_coding_shell_shape(command: &str) -> bool {
    let command_lines = shell_command_lines_without_heredoc_bodies(command);
    if command_lines.is_empty() {
        return false;
    }
    let mut saw_write = false;
    for line in command_lines {
        if line.contains("$(") || line.contains('`') {
            return false;
        }
        for segment in split_shell_control_segments(&line) {
            let segment = segment.trim();
            if segment.is_empty() {
                continue;
            }
            if shell_segment_has_unsafe_expansion(segment) {
                return false;
            }
            let Some(program) = segment.split_whitespace().next() else {
                continue;
            };
            match program {
                "mkdir" | "touch" => saw_write = true,
                "cat" => {
                    if !segment.contains('>') {
                        return false;
                    }
                    saw_write = true;
                }
                "printf" | "echo" => {
                    if segment.contains('>') {
                        saw_write = true;
                    }
                }
                "set" if safe_fail_fast_shell_options(segment) => {}
                "cd" | "pwd" | "ls" | "test" | "[" => {}
                _ if local_validation_program(program)
                    && crate::execution_recipe::run_cmd_looks_validation(segment) => {}
                _ => return false,
            }
        }
    }
    saw_write
}

fn shell_segment_has_unsafe_expansion(segment: &str) -> bool {
    let mut chars = segment.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            continue;
        }
        if chars.next() != Some('?') {
            return true;
        }
    }
    false
}

fn local_validation_program(program: &str) -> bool {
    matches!(
        program,
        "python"
            | "python3"
            | "pytest"
            | "cargo"
            | "npm"
            | "pnpm"
            | "yarn"
            | "bun"
            | "go"
            | "make"
            | "just"
            | "mvn"
            | "gradle"
    )
}

fn safe_fail_fast_shell_options(segment: &str) -> bool {
    let options = segment.split_whitespace().skip(1).collect::<Vec<_>>();
    !options.is_empty()
        && options.iter().all(|option| {
            matches!(
                *option,
                "-e" | "-u" | "-eu" | "-ue" | "-euo" | "-o" | "pipefail"
            )
        })
}

fn shell_command_lines_without_heredoc_bodies(command: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut heredoc_end: Option<String> = None;
    for raw_line in command.lines() {
        if let Some(end) = heredoc_end.as_deref() {
            if raw_line.trim() == end {
                heredoc_end = None;
            }
            continue;
        }
        lines.push(raw_line.to_string());
        heredoc_end = heredoc_delimiter(raw_line);
    }
    lines
}

fn heredoc_delimiter(line: &str) -> Option<String> {
    let (_, tail) = line.split_once("<<")?;
    let token = tail
        .trim_start()
        .trim_start_matches('-')
        .split_whitespace()
        .next()?
        .trim_matches(|ch| matches!(ch, '\'' | '"'));
    (!token.is_empty()).then(|| token.to_string())
}

fn split_shell_control_segments(line: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            current.push(ch);
            escaped = true;
            continue;
        }
        if matches!(ch, '\'' | '"') {
            if quote == Some(ch) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(ch);
            }
            current.push(ch);
            continue;
        }
        if quote.is_none() && matches!(ch, ';' | '|' | '&') {
            if !current.trim().is_empty() {
                segments.push(std::mem::take(&mut current));
            }
            continue;
        }
        current.push(ch);
    }
    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
}

pub(super) fn validation_run_cmd_can_run_autonomously(
    state: &AppState,
    normalized_skill: &str,
    args: &Value,
) -> bool {
    if normalized_skill != "run_cmd"
        || crate::execution_recipe::action_targets_external_workspace(state, normalized_skill, args)
    {
        return false;
    }
    let effect =
        crate::execution_recipe::classify_skill_action_effect(state, normalized_skill, args);
    effect.validates && !effect.mutates
}

pub(super) fn context_bundle_has_redacted_workspace_child_locator(summary: Option<&str>) -> bool {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let Some(summary) = summary else {
        return false;
    };
    summary.split(START).skip(1).any(|tail| {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            return false;
        };
        value
            .get("current_request_locator")
            .and_then(|locator| locator.get("resolved_workspace_child_redacted"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

fn path_args(args: &Value) -> Vec<&str> {
    let Some(obj) = args.as_object() else {
        return Vec::new();
    };
    ["path", "file_path", "target_path", "requested_path"]
        .into_iter()
        .filter_map(|key| {
            obj.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .collect()
}

fn path_value_is_workspace_scoped(path: &str, workspace_root: &Path) -> bool {
    let candidate = Path::new(path);
    if candidate
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return false;
    }
    if !candidate.is_absolute() {
        return true;
    }
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let target = candidate
        .canonicalize()
        .unwrap_or_else(|_| candidate.to_path_buf());
    target.starts_with(root)
}

fn verifier_workspace_scope_summary(
    args: &Value,
    workspace_root: &Path,
    external_workspace: bool,
) -> Value {
    let paths = path_args(args);
    let untrusted_path_present = paths
        .iter()
        .any(|path| !path_value_is_workspace_scoped(path, workspace_root));
    let scope = if untrusted_path_present || external_workspace {
        "external_or_untrusted"
    } else if paths.is_empty() {
        "unspecified"
    } else {
        "workspace_scoped"
    };
    json!({
        "schema_version": 1,
        "scope": scope,
        "path_arg_count": paths.len(),
        "cwd_present": args
            .get("cwd")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty()),
        "untrusted_path_present": untrusted_path_present,
        "external_workspace": external_workspace,
    })
}

fn verifier_sandbox_profile_token(
    registry_policy: Option<&Value>,
    effect: crate::execution_recipe::ActionEffect,
) -> String {
    registry_policy
        .and_then(|policy| policy.get("isolation_profile"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| {
            if effect.mutates {
                "local_current_workspace".to_string()
            } else {
                "read_only_or_validation".to_string()
            }
        })
}

fn verifier_sandbox_summary(
    registry_policy: Option<&Value>,
    sandbox_profile: &str,
    effect: crate::execution_recipe::ActionEffect,
) -> Value {
    json!({
        "schema_version": 1,
        "profile": sandbox_profile,
        "source": if registry_policy
            .and_then(|policy| policy.get("isolation_profile"))
            .and_then(Value::as_str)
            .is_some_and(|value| !value.trim().is_empty())
        {
            "registry_capability_policy"
        } else {
            "effect_default"
        },
        "filesystem_write": registry_policy
            .and_then(|policy| policy.get("filesystem_write"))
            .and_then(Value::as_bool)
            .unwrap_or(effect.mutates),
        "network_access": registry_policy
            .and_then(|policy| policy.get("network_access"))
            .and_then(Value::as_bool),
        "external_publish": registry_policy
            .and_then(|policy| policy.get("external_publish"))
            .and_then(Value::as_bool),
        "credential_access": registry_policy
            .and_then(|policy| policy.get("credential_access"))
            .and_then(Value::as_bool),
    })
}

fn args_have_untrusted_path_value(args: &Value, workspace_root: &Path) -> bool {
    path_args(args)
        .into_iter()
        .any(|path| !path_value_is_workspace_scoped(path, workspace_root))
}

pub(super) fn step_reads_path_content_under_unbound_locator(
    step: &PlanStep,
    normalized_skill: &str,
    workspace_root: &Path,
) -> bool {
    if path_args(&step.args).is_empty()
        || !args_have_untrusted_path_value(&step.args, workspace_root)
    {
        return false;
    }
    match step.action_type.as_str() {
        "call_capability" => action_key_reads_path_content(step.skill.trim()),
        "call_skill" | "call_tool" => {
            let action_key =
                crate::evidence_policy::ActionRef::from_skill_args(normalized_skill, &step.args)
                    .map(|action| action.as_key());
            action_key
                .as_deref()
                .is_some_and(action_key_reads_path_content)
        }
        _ => false,
    }
}

fn action_key_reads_path_content(action_key: &str) -> bool {
    let Some(action_key) =
        crate::evidence_policy::ActionRef::parse(action_key).map(|action| action.as_key())
    else {
        return false;
    };
    matches!(
        action_key.as_str(),
        "filesystem.read_text_range"
            | "filesystem.read_file"
            | "filesystem.grep_text"
            | "fs_basic.read_text_range"
            | "fs_basic.grep_text"
            | "system_basic.read_range"
            | "system_basic.read_file"
            | "read_file"
    )
}

pub(super) fn push_unbound_locator_boundary_clarify_issue(
    issues: &mut Vec<VerifyIssue>,
    step_id: &str,
) {
    if issues.iter().any(|issue| {
        issue.step_id == step_id
            && matches!(issue.kind, VerifyIssueKind::BoundaryClarifyRequired)
            && issue.detail.contains("resolved_workspace_child_redacted")
    }) {
        return;
    }
    issues.push(VerifyIssue {
        step_id: step_id.to_string(),
        kind: VerifyIssueKind::BoundaryClarifyRequired,
        detail: "unbound_locator_requires_clarify; boundary=resolved_workspace_child_redacted"
            .to_string(),
        missing_fields: vec!["execution_target_or_boundary".to_string()],
    });
}
