use std::path::Path;

use claw_core::skill_registry::PlannerCapabilityEffect;
use serde_json::Value;

use crate::AppState;

#[path = "execution_recipe_types.rs"]
mod execution_recipe_types;

pub(crate) use execution_recipe_types::{
    parse_execution_recipe_profile_text, profile_requires_specific_validation, ActionEffect,
    ExecutionRecipeKind, ExecutionRecipePhase, ExecutionRecipePlanHint, ExecutionRecipeProfile,
    ExecutionRecipeRuntimeState, ExecutionRecipeSpec, ExecutionRecipeTargetScope,
};

fn planner_capability_effect_to_action_effect(effect: PlannerCapabilityEffect) -> ActionEffect {
    match effect {
        PlannerCapabilityEffect::Observe => ActionEffect::observe(),
        PlannerCapabilityEffect::Mutate | PlannerCapabilityEffect::External => {
            ActionEffect::mutate()
        }
        PlannerCapabilityEffect::Validate => ActionEffect::validate(),
    }
}

fn registry_side_effect_fallback_action_effect(
    state: &AppState,
    normalized_skill: &str,
) -> Option<ActionEffect> {
    state
        .skill_manifest(normalized_skill)
        .and_then(|manifest| match manifest.side_effect {
            Some(false) => Some(ActionEffect::observe()),
            _ => None,
        })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ValidationObservation {
    Passed,
    Failed(String),
    Inconclusive,
}

pub(crate) const CLAWD_VALIDATION_ARG: &str = "_clawd_validation";

fn structured_validation_value(args: &Value) -> Option<&Value> {
    args.get(CLAWD_VALIDATION_ARG)
}

fn structured_validation_declared(args: &Value) -> bool {
    match structured_validation_value(args) {
        Some(Value::Bool(true)) => true,
        Some(Value::Object(map)) => {
            map.get("validation")
                .or_else(|| map.get("is_validation"))
                .or_else(|| map.get("intent"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
                || map
                    .get("profile")
                    .or_else(|| map.get("validation_profile"))
                    .is_some()
                || map.get("validator_type").is_some()
                || map.get("validated_target").is_some()
        }
        _ => false,
    }
}

fn structured_validation_profile(args: &Value) -> ExecutionRecipeProfile {
    let Some(Value::Object(map)) = structured_validation_value(args) else {
        return ExecutionRecipeProfile::None;
    };
    map.get("profile")
        .or_else(|| map.get("validation_profile"))
        .and_then(Value::as_str)
        .map(parse_execution_recipe_profile_text)
        .unwrap_or(ExecutionRecipeProfile::None)
}

fn structured_validation_satisfies_profile(
    recipe: ExecutionRecipeRuntimeState,
    args: &Value,
) -> bool {
    if !structured_validation_declared(args) {
        return false;
    }
    match recipe.profile {
        ExecutionRecipeProfile::None | ExecutionRecipeProfile::OpsService => true,
        expected => structured_validation_profile(args) == expected,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SuccessMarkerMatchMode {
    Contains,
    Equals,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StructuredSuccessMarker {
    marker: String,
    match_mode: SuccessMarkerMatchMode,
    case_sensitive: bool,
}

fn parse_success_marker_match_mode(value: Option<&str>) -> SuccessMarkerMatchMode {
    match value
        .unwrap_or("contains")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "equals" | "exact" | "exact_match" => SuccessMarkerMatchMode::Equals,
        _ => SuccessMarkerMatchMode::Contains,
    }
}

fn structured_validation_success_marker(args: &Value) -> Option<StructuredSuccessMarker> {
    let Value::Object(map) = structured_validation_value(args)? else {
        return None;
    };
    let raw_marker = map
        .get("success_marker")
        .or_else(|| map.get("required_success_marker"))
        .or_else(|| map.get("expected_output_marker"))
        .or_else(|| map.get("expect_contains"))?;
    match raw_marker {
        Value::String(marker) => {
            let marker = marker.trim();
            (!marker.is_empty()).then(|| StructuredSuccessMarker {
                marker: marker.to_string(),
                match_mode: SuccessMarkerMatchMode::Contains,
                case_sensitive: true,
            })
        }
        Value::Object(marker_obj) => {
            let marker = marker_obj
                .get("marker")
                .or_else(|| marker_obj.get("text"))
                .or_else(|| marker_obj.get("value"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|marker| !marker.is_empty())?;
            let match_mode = parse_success_marker_match_mode(
                marker_obj
                    .get("match_mode")
                    .or_else(|| marker_obj.get("mode"))
                    .and_then(Value::as_str),
            );
            let case_sensitive = marker_obj
                .get("case_sensitive")
                .and_then(Value::as_bool)
                .unwrap_or(true);
            Some(StructuredSuccessMarker {
                marker: marker.to_string(),
                match_mode,
                case_sensitive,
            })
        }
        _ => None,
    }
}

fn structured_success_marker_matches(output: &str, spec: &StructuredSuccessMarker) -> bool {
    let (candidate, marker) = if spec.case_sensitive {
        (output.to_string(), spec.marker.clone())
    } else {
        (output.to_lowercase(), spec.marker.to_lowercase())
    };
    match spec.match_mode {
        SuccessMarkerMatchMode::Contains => text_contains_success_marker(&candidate, &marker),
        SuccessMarkerMatchMode::Equals => candidate.trim() == marker.trim(),
    }
}

fn success_marker_boundary(ch: Option<char>) -> bool {
    ch.is_none_or(|ch| !ch.is_ascii_alphanumeric() && !matches!(ch, '_' | '-' | '.'))
}

fn text_contains_success_marker(candidate: &str, marker: &str) -> bool {
    let marker = marker.trim();
    if marker.is_empty() {
        return false;
    }
    candidate.match_indices(marker).any(|(start, _)| {
        let before = candidate[..start].chars().next_back();
        let after = candidate[start + marker.len()..].chars().next();
        success_marker_boundary(before) && success_marker_boundary(after)
    })
}

fn structured_success_marker_observation(args: &Value, output: &str) -> ValidationObservation {
    let Some(spec) = structured_validation_success_marker(args) else {
        return ValidationObservation::Inconclusive;
    };
    if structured_success_marker_matches(output, &spec) {
        ValidationObservation::Passed
    } else {
        ValidationObservation::Failed(format!(
            "validation_required_marker_missing:marker={}",
            spec.marker
        ))
    }
}

fn merge_structured_validation_effect(
    normalized_skill: &str,
    args: &Value,
    mut effect: ActionEffect,
) -> ActionEffect {
    let action = normalized_action_arg(args);
    if normalized_skill == "run_cmd" && matches!(action.as_str(), "" | "run_cmd") {
        let command_effect = args
            .get("command")
            .and_then(Value::as_str)
            .map(run_cmd_action_effect)
            .unwrap_or_default();
        effect = ActionEffect {
            observes: effect.observes || command_effect.observes,
            mutates: effect.mutates || command_effect.mutates,
            validates: effect.validates || command_effect.validates,
        };
    }
    let has_validation_expectation = structured_validation_declared(args)
        || structured_validation_success_marker(args).is_some()
        || (normalized_skill == "http_basic" && http_basic_has_validation_expectation(args));
    if !has_validation_expectation {
        return effect;
    }
    if effect.mutates && !effect.validates && normalized_skill != "run_cmd" {
        return effect;
    }
    ActionEffect {
        observes: true,
        mutates: effect.mutates,
        validates: true,
    }
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| text.contains(needle))
}

fn normalized_first_command_word(command: &str) -> Option<String> {
    command
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    ch.is_whitespace()
                        || matches!(
                            ch,
                            '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}'
                        )
                })
                .to_ascii_lowercase()
        })
        .find(|token| {
            !token.is_empty()
                && !(token.contains('=')
                    && !token.starts_with("./")
                    && !token.contains('/')
                    && !token.starts_with('-'))
        })
}

pub(crate) fn validation_satisfies_recipe_profile(
    recipe: ExecutionRecipeRuntimeState,
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    if structured_validation_satisfies_profile(recipe, args) {
        return true;
    }
    let normalized_skill = state.resolve_canonical_skill_name(skill_name);
    match recipe.profile {
        ExecutionRecipeProfile::None | ExecutionRecipeProfile::OpsService => {
            classify_skill_action_effect(state, &normalized_skill, args).validates
        }
        ExecutionRecipeProfile::ConfigChange => match normalized_skill.as_str() {
            "config_basic" | "config_guard" => {
                let action = normalized_action_arg(args);
                args.get("path").is_some()
                    && (action.is_empty()
                        || contains_any(&action, &["validate", "check", "read", "guard"]))
            }
            "config_edit" => {
                let action = normalized_action_arg(args);
                contains_any(&action, &["validate_config", "guard_config", "read_back"])
            }
            "service_control" | "health_check" | "http_basic" => true,
            "run_cmd" => run_cmd_validation_command(args),
            _ => false,
        },
        ExecutionRecipeProfile::CodeChange => match normalized_skill.as_str() {
            "service_control" | "health_check" | "http_basic" => true,
            "run_cmd" => run_cmd_validation_command(args),
            _ => false,
        },
        ExecutionRecipeProfile::SkillAuthoring => match normalized_skill.as_str() {
            "run_cmd" => run_cmd_validation_command(args),
            "extension_manager" => contains_any(
                &normalized_action_arg(args),
                &["validate_external_skill", "register_external_skill"],
            ),
            _ => false,
        },
        ExecutionRecipeProfile::PackageChange => match normalized_skill.as_str() {
            "run_cmd" => run_cmd_validation_command(args),
            "package_manager" => contains_any(&normalized_action_arg(args), &["detect"]),
            _ => false,
        },
        ExecutionRecipeProfile::DatabaseChange => match normalized_skill.as_str() {
            "db_basic" => contains_any(
                &normalized_action_arg(args),
                &["sqlite_query", "schema_version", "list_tables"],
            ),
            "run_cmd" => run_cmd_validation_command(args),
            _ => false,
        },
    }
}

fn normalized_action_arg(args: &Value) -> String {
    args.get("action")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|ch| {
            if matches!(ch, '-' | ' ' | '.') {
                '_'
            } else {
                ch
            }
        })
        .collect()
}

fn run_cmd_validation_command(args: &Value) -> bool {
    args.get("command")
        .and_then(Value::as_str)
        .is_some_and(run_cmd_looks_validation)
}

pub(crate) fn validation_detail_for_recipe(recipe: ExecutionRecipeRuntimeState) -> &'static str {
    match recipe.profile {
        ExecutionRecipeProfile::ConfigChange => {
            "config_change requires post-change validation through parsing, checking, reloading, or effective-state verification"
        }
        ExecutionRecipeProfile::CodeChange => {
            "code_change requires compile/test/build or runtime verification after mutation"
        }
        ExecutionRecipeProfile::SkillAuthoring => {
            "skill_authoring requires integration validation after mutation through build/test checks or extension registration verification"
        }
        ExecutionRecipeProfile::PackageChange => {
            "package_change requires package state, build/test, or runtime command validation after mutation"
        }
        ExecutionRecipeProfile::DatabaseChange => {
            "database_change requires schema, table, version, or query validation after mutation"
        }
        _ => "ops_closed_loop requires a machine-verifiable validation step after mutation",
    }
}

pub(crate) fn target_scope_detail_for_recipe(recipe: ExecutionRecipeRuntimeState) -> &'static str {
    match recipe.target_scope {
        ExecutionRecipeTargetScope::CurrentRepo => {
            "current_repo scope must stay inside the current workspace and should not drift to external absolute paths"
        }
        ExecutionRecipeTargetScope::ExternalWorkspace => {
            "external_workspace scope requires an explicit external path or working directory outside the current workspace"
        }
        ExecutionRecipeTargetScope::Greenfield => {
            "greenfield scope requires creating a new file, directory, or scaffold before verification"
        }
        _ => "execution recipe target scope is misaligned with the planned actions",
    }
}

fn trim_path_like_token(token: &str) -> &str {
    token.trim_matches(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                '"' | '\'' | '`' | ',' | ';' | '(' | ')' | '[' | ']' | '{' | '}' | ':'
            )
    })
}

fn path_candidate_scope(
    candidate: &str,
    workspace_root: &Path,
) -> Option<ExecutionRecipeTargetScope> {
    let candidate = trim_path_like_token(candidate);
    if candidate.is_empty() || candidate.contains("://") {
        return None;
    }
    let path = Path::new(candidate);
    if path.is_absolute() {
        return Some(if path.starts_with(workspace_root) {
            ExecutionRecipeTargetScope::CurrentRepo
        } else {
            ExecutionRecipeTargetScope::ExternalWorkspace
        });
    }
    if candidate.starts_with("./")
        || candidate.starts_with("../")
        || candidate.contains('/')
        || candidate.starts_with("~/")
    {
        return Some(ExecutionRecipeTargetScope::CurrentRepo);
    }
    None
}

fn arg_path_candidates(args: &Value) -> Vec<String> {
    let mut candidates = Vec::new();
    for key in [
        "path",
        "cwd",
        "dir",
        "directory",
        "root",
        "workspace",
        "workspace_root",
        "output_path",
        "target_path",
    ] {
        if let Some(value) = args.get(key).and_then(|value| value.as_str()) {
            let trimmed = trim_path_like_token(value);
            if !trimmed.is_empty() {
                candidates.push(trimmed.to_string());
            }
        }
    }
    candidates
}

fn run_cmd_path_candidates(args: &Value) -> Vec<String> {
    let mut candidates = arg_path_candidates(args);
    let Some(command) = args.get("command").and_then(|value| value.as_str()) else {
        return candidates;
    };
    let mut expect_cd_target = false;
    for raw_token in command.split_whitespace() {
        let token = trim_path_like_token(raw_token);
        if token.is_empty() {
            continue;
        }
        if expect_cd_target {
            candidates.push(token.to_string());
            expect_cd_target = false;
            continue;
        }
        if matches!(token, "cd" | "pushd") {
            expect_cd_target = true;
            continue;
        }
        if token.starts_with('/')
            || token.starts_with("./")
            || token.starts_with("../")
            || token.starts_with("~/")
        {
            candidates.push(token.to_string());
        }
    }
    candidates
}

fn action_path_candidates(state: &AppState, skill_name: &str, args: &Value) -> Vec<String> {
    let normalized_skill = state.resolve_canonical_skill_name(skill_name);
    match normalized_skill.as_str() {
        "run_cmd" => run_cmd_path_candidates(args),
        _ => arg_path_candidates(args),
    }
}

fn run_cmd_looks_greenfield_creation(command_lower: &str) -> bool {
    contains_any(
        command_lower,
        &[
            "cargo new",
            "cargo init",
            "npm create",
            "pnpm create",
            "yarn create",
            "bun create",
            "go mod init",
            "python -m venv",
            "python3 -m venv",
            "uv init",
            "mkdir ",
            "mkdir -p",
            "touch ",
        ],
    )
}

pub(crate) fn action_targets_external_workspace(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    action_path_candidates(state, skill_name, args)
        .into_iter()
        .any(|candidate| {
            matches!(
                path_candidate_scope(&candidate, &state.skill_rt.workspace_root),
                Some(ExecutionRecipeTargetScope::ExternalWorkspace)
            )
        })
}

pub(crate) fn action_conflicts_with_recipe_target_scope(
    recipe: ExecutionRecipeRuntimeState,
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    match recipe.target_scope {
        ExecutionRecipeTargetScope::CurrentRepo => {
            action_targets_external_workspace(state, skill_name, args)
        }
        ExecutionRecipeTargetScope::ExternalWorkspace => {
            let candidates = action_path_candidates(state, skill_name, args);
            !candidates.is_empty()
                && candidates.into_iter().any(|candidate| {
                    matches!(
                        path_candidate_scope(&candidate, &state.skill_rt.workspace_root),
                        Some(ExecutionRecipeTargetScope::CurrentRepo)
                    )
                })
        }
        _ => false,
    }
}

pub(crate) fn action_satisfies_greenfield_creation(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> bool {
    if matches!(skill_name.trim(), "write_file" | "make_dir") {
        return true;
    }
    match state.resolve_canonical_skill_name(skill_name).as_str() {
        "write_file" | "make_dir" => true,
        "fs_basic" => args
            .get("action")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .is_some_and(|action| matches!(action, "make_dir" | "write_text")),
        "run_cmd" => args
            .get("command")
            .and_then(|value| value.as_str())
            .map(|command| {
                let lower = command.to_ascii_lowercase();
                run_cmd_looks_greenfield_creation(&lower)
                    || run_cmd_has_explicit_write_marker(command)
            })
            .unwrap_or(false),
        _ => false,
    }
}

pub(crate) fn apply_target_scope_progress(
    recipe: &mut ExecutionRecipeRuntimeState,
    state: &AppState,
    skill_name: &str,
    args: &Value,
    action_succeeded: bool,
) {
    if !recipe.is_active() {
        return;
    }
    if matches!(
        recipe.target_scope,
        ExecutionRecipeTargetScope::ExternalWorkspace
    ) && action_targets_external_workspace(state, skill_name, args)
    {
        recipe.saw_external_target = true;
    }
    if action_succeeded
        && matches!(recipe.target_scope, ExecutionRecipeTargetScope::Greenfield)
        && action_satisfies_greenfield_creation(state, skill_name, args)
    {
        recipe.saw_greenfield_creation = true;
    }
}

fn run_cmd_has_explicit_write_marker(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first_word = normalized_first_command_word(command);
    shell_has_output_redirection_marker(command)
        || lower.contains(" tee ")
        || lower.starts_with("tee ")
        || lower.contains(" sed -i")
        || lower.starts_with("sed -i")
        || lower.contains(" perl -pi")
        || lower.starts_with("perl -pi")
        || lower.contains("systemctl start")
        || lower.contains("systemctl stop")
        || lower.contains("systemctl restart")
        || lower.contains("systemctl reload")
        || lower.contains("systemctl enable")
        || lower.contains("systemctl disable")
        || lower.contains(" service ")
            && contains_any(
                &lower,
                &[
                    " start", " stop", " restart", " reload", " enable", " disable",
                ],
            )
        || matches!(
            first_word.as_deref(),
            Some(
                "cp" | "mv"
                    | "rm"
                    | "mkdir"
                    | "touch"
                    | "truncate"
                    | "install"
                    | "dd"
                    | "chmod"
                    | "chown"
                    | "ln"
                    | "launchctl"
            )
        )
}

fn shell_has_output_redirection_marker(command: &str) -> bool {
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let chars: Vec<char> = command.chars().collect();
    for (idx, ch) in chars.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        if *ch == '\\' && !in_single {
            escaped = true;
            continue;
        }
        if *ch == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if *ch == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if *ch != '>' || in_single || in_double {
            continue;
        }
        let prev = chars[..idx]
            .iter()
            .rev()
            .find(|value| !value.is_whitespace())
            .copied();
        let next = chars
            .get(idx + 1)
            .copied()
            .filter(|value| !value.is_whitespace());
        if prev == Some('=') || next == Some('=') {
            continue;
        }
        return true;
    }
    false
}

fn shell_contains_command_invocation(command_lower: &str, word: &str) -> bool {
    command_lower.starts_with(&format!("{word} "))
        || command_lower.contains(&format!("\n{word} "))
        || ["&&", ";", "|", "||", "("]
            .into_iter()
            .any(|prefix| command_lower.contains(&format!("{prefix} {word} ")))
}

pub(crate) fn run_cmd_looks_validation(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    let first_word = normalized_first_command_word(command);
    contains_any(
        &lower,
        &[
            " check",
            "check ",
            " test",
            "test ",
            " verify",
            "verify ",
            " validate",
            "validate ",
            "cargo check",
            "cargo test",
            "cargo clippy",
            "cargo build",
            "cargo run",
            "pytest",
            "python -m pytest",
            "python3 -m pytest",
            "python -m unittest",
            "python3 -m unittest",
            "uv run pytest",
            "uv run python",
            "npm run test",
            "npm run build",
            "npm run lint",
            "pnpm run test",
            "pnpm run build",
            "pnpm run lint",
            "yarn test",
            "yarn build",
            "yarn lint",
            "bun test",
            "bun run test",
            "bun run build",
            "bun run lint",
            "go test",
            "go build",
            "go run",
            "make test",
            "make check",
            "make build",
            "just test",
            "just check",
            "mvn test",
            "gradle test",
            "systemctl status",
            "systemctl is-active",
            " service status",
            "nginx -t",
            "sing-box check",
            "docker ps",
            "docker inspect",
            "docker compose ps",
            "kubectl get",
            "kubectl describe",
            "journalctl",
            "health",
            "validation_passed",
            "validation_failed",
        ],
    ) || matches!(
        first_word.as_deref(),
        Some("curl" | "wget" | "nc" | "ss" | "lsof")
    ) || ["curl", "wget", "nc", "ss", "lsof"]
        .into_iter()
        .any(|word| shell_contains_command_invocation(&lower, word))
        || run_cmd_looks_inline_python_validation_probe(command)
}

fn run_cmd_looks_inline_python_validation_probe(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    if !run_cmd_invokes_inline_python(&lower) {
        return false;
    }
    if inline_python_probe_has_mutation_or_escape_signal(&lower) {
        return false;
    }
    contains_any(
        &lower,
        &[
            "print(", "assert ", "from ", "import ", "unittest", "pytest",
        ],
    )
}

fn run_cmd_invokes_inline_python(command_lower: &str) -> bool {
    ["python", "python3"].into_iter().any(|word| {
        let heredoc = format!("{word} - <<");
        let heredoc_space = format!("{word} -  <<");
        let inline = format!("{word} -c ");
        command_lower.starts_with(&heredoc)
            || command_lower.starts_with(&heredoc_space)
            || command_lower.starts_with(&inline)
            || ["&&", ";", "|", "||", "("].into_iter().any(|prefix| {
                command_lower.contains(&format!("{prefix} {heredoc}"))
                    || command_lower.contains(&format!("{prefix} {heredoc_space}"))
                    || command_lower.contains(&format!("{prefix} {inline}"))
            })
    })
}

fn inline_python_probe_has_mutation_or_escape_signal(command_lower: &str) -> bool {
    contains_any(
        command_lower,
        &[
            " pip install",
            "python -m pip",
            "python3 -m pip",
            "subprocess",
            "os.system(",
            "popen(",
            "socket.",
            "requests.",
            "urllib.",
            "http.client",
            "open(",
            ".write(",
            ".write_text(",
            ".write_bytes(",
            "unlink(",
            "remove(",
            "rmdir(",
            "rmtree(",
            "mkdir(",
            "makedirs(",
            "rename(",
            "replace(",
            "chmod(",
            "chown(",
            "symlink(",
            "truncate(",
            "shutil.copy",
            "shutil.move",
        ],
    )
}

fn combined_action_effect(mutates: bool, validates: bool) -> ActionEffect {
    if !mutates && !validates {
        return ActionEffect::observe();
    }
    ActionEffect {
        observes: validates,
        mutates,
        validates,
    }
}

fn run_cmd_action_effect(command: &str) -> ActionEffect {
    let mutates = run_cmd_has_explicit_write_marker(command);
    let validates = run_cmd_looks_validation(command);
    if command.trim().is_empty() {
        ActionEffect::default()
    } else {
        combined_action_effect(mutates, validates)
    }
}

pub(crate) fn split_run_cmd_mutation_and_validation(command: &str) -> Option<(String, String)> {
    let effect = run_cmd_action_effect(command);
    if !effect.mutates || !effect.validates {
        return None;
    }
    let bytes = command.as_bytes();
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte != b'&' {
            continue;
        }
        let prev = idx.checked_sub(1).and_then(|pos| bytes.get(pos)).copied();
        let next = bytes.get(idx + 1).copied();
        if prev == Some(b'&')
            || next == Some(b'&')
            || prev == Some(b'>')
            || next == Some(b'>')
            || next.is_some_and(|value| value.is_ascii_digit())
        {
            continue;
        }
        let mutate_part = command[..=idx].trim();
        let validate_part = command[idx + 1..].trim();
        if mutate_part.is_empty() || validate_part.is_empty() {
            continue;
        }
        if run_cmd_has_explicit_write_marker(mutate_part) && run_cmd_looks_validation(validate_part)
        {
            return Some((mutate_part.to_string(), validate_part.to_string()));
        }
    }
    None
}

pub(crate) fn classify_skill_action_effect(
    state: &AppState,
    skill_name: &str,
    args: &Value,
) -> ActionEffect {
    let normalized_skill = state.resolve_canonical_skill_name(skill_name);
    if crate::agent_engine::planner_internal_tool_is_observe_only(&normalized_skill) {
        return ActionEffect::observe();
    }
    if let Some(tool) = state.mcp_tool(&normalized_skill) {
        return match tool.policy.effect.as_str() {
            "observe" => ActionEffect::observe(),
            "validate" => ActionEffect::validate(),
            "mutate" | "external" => ActionEffect::mutate(),
            _ => ActionEffect::default(),
        };
    }
    if dry_run_observes_only_action(&normalized_skill, args) {
        return ActionEffect::observe();
    }
    if let Some(effect) = args
        .get("action")
        .and_then(|value| value.as_str())
        .map(|value| {
            value
                .trim()
                .to_ascii_lowercase()
                .chars()
                .map(|ch| {
                    if matches!(ch, '-' | ' ' | '.') {
                        '_'
                    } else {
                        ch
                    }
                })
                .collect::<String>()
        })
        .filter(|action| !action.is_empty())
        .and_then(|action| {
            state
                .skill_manifest(&normalized_skill)
                .and_then(|manifest| {
                    manifest
                        .planner_capabilities
                        .into_iter()
                        .find(|mapping| mapping.action.as_deref() == Some(action.as_str()))
                        .and_then(|mapping| mapping.effect)
                })
        })
        .map(planner_capability_effect_to_action_effect)
    {
        return merge_structured_validation_effect(&normalized_skill, args, effect);
    }
    if let Some(effect) = registry_side_effect_fallback_action_effect(state, &normalized_skill) {
        return merge_structured_validation_effect(&normalized_skill, args, effect);
    }
    let effect = match normalized_skill.as_str() {
        "read_file" | "list_dir" | "fs_search" | "git_basic" | "process_basic" | "log_analyze" => {
            ActionEffect::observe()
        }
        "write_file" | "remove_file" | "make_dir" | "install_module" => ActionEffect::mutate(),
        "fs_basic" => match normalized_action_arg(args).as_str() {
            "write_text" | "append_text" | "make_dir" | "remove_path" => ActionEffect::mutate(),
            _ => ActionEffect::observe(),
        },
        "package_manager" => {
            let action = normalized_action_arg(args);
            if contains_any(&action, &["install", "uninstall", "smart_install"]) {
                ActionEffect::mutate()
            } else if contains_any(&action, &["detect"]) {
                ActionEffect::observe()
            } else {
                ActionEffect::default()
            }
        }
        "health_check" => ActionEffect::validate(),
        "http_basic" => http_basic_action_effect(args),
        "system_basic" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if contains_any(&action, &["check", "health"]) {
                ActionEffect::validate()
            } else if !action.is_empty() {
                ActionEffect::observe()
            } else {
                ActionEffect::default()
            }
        }
        "config_guard" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if args.get("key").is_some() || args.get("value").is_some() {
                ActionEffect::mutate()
            } else if contains_any(&action, &["patch", "write", "set", "update", "modify"]) {
                ActionEffect::mutate()
            } else if contains_any(&action, &["validate", "check"]) {
                ActionEffect::validate()
            } else {
                ActionEffect::observe()
            }
        }
        "service_control" => {
            let action = args
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if contains_any(
                &action,
                &["start", "stop", "restart", "reload", "enable", "disable"],
            ) {
                ActionEffect::mutate()
            } else if contains_any(&action, &["status", "verify"]) {
                ActionEffect::validate()
            } else if !action.is_empty() {
                ActionEffect::observe()
            } else {
                ActionEffect::default()
            }
        }
        "run_cmd" => {
            let command = args
                .get("command")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            run_cmd_action_effect(command)
        }
        "db_basic" => {
            let action = normalized_action_arg(args);
            if contains_any(&action, &["sqlite_execute"]) {
                ActionEffect::mutate()
            } else if contains_any(&action, &["sqlite_query", "schema_version", "list_tables"]) {
                ActionEffect::observe()
            } else {
                ActionEffect::default()
            }
        }
        _ => ActionEffect::default(),
    };
    merge_structured_validation_effect(&normalized_skill, args, effect)
}

pub(crate) fn dry_run_observes_only_action(normalized_skill: &str, args: &Value) -> bool {
    args.get("dry_run").and_then(Value::as_bool) == Some(true)
        && (package_manager_dry_run_install_action(normalized_skill, args)
            || task_control_lifecycle_dry_run_action(normalized_skill, args)
            || media_generation_dry_run_action(normalized_skill))
}

fn media_generation_dry_run_action(normalized_skill: &str) -> bool {
    matches!(
        normalized_skill,
        "image_generate" | "image_edit" | "audio_synthesize" | "video_generate" | "music_generate"
    )
}

fn package_manager_dry_run_install_action(normalized_skill: &str, args: &Value) -> bool {
    if normalized_skill != "package_manager" {
        return false;
    }
    if args.get("dry_run").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    let action = normalized_action_arg(args);
    contains_any(&action, &["install", "uninstall", "smart_install"])
}

fn task_control_lifecycle_dry_run_action(normalized_skill: &str, args: &Value) -> bool {
    if normalized_skill != "task_control" {
        return false;
    }
    if args.get("dry_run").and_then(Value::as_bool) != Some(true) {
        return false;
    }
    let action = normalized_action_arg(args);
    contains_any(&action, &["resume", "pause"])
}

fn service_state_is_healthy(state: &str) -> bool {
    let lower = state.trim().to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "active" | "running" | "active (running)" | "started" | "healthy" | "ok"
    ) || (lower.contains("active") && lower.contains("running"))
}

fn service_state_looks_failed(state: &str) -> bool {
    let lower = state.trim().to_ascii_lowercase();
    contains_any(
        &lower,
        &[
            "inactive",
            "stopped",
            "failed",
            "dead",
            "not running",
            "unhealthy",
            "unknown",
            "error",
        ],
    )
}

fn assess_service_control_validation(output: &str) -> ValidationObservation {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return ValidationObservation::Inconclusive;
    };
    if value.get("status").and_then(|v| v.as_str()) == Some("error") {
        let detail = value
            .get("failure_reason")
            .and_then(|v| v.as_str())
            .filter(|text| !text.trim().is_empty())
            .or_else(|| value.get("error_code").and_then(|v| v.as_str()))
            .unwrap_or("service_control_error");
        return ValidationObservation::Failed(detail.to_string());
    }
    if value
        .get("verified")
        .and_then(|v| v.as_bool())
        .is_some_and(|verified| !verified)
    {
        let detail = value
            .get("failure_reason")
            .and_then(|v| v.as_str())
            .filter(|text| !text.trim().is_empty())
            .or_else(|| value.get("post_state").and_then(|v| v.as_str()))
            .or_else(|| value.get("pre_state").and_then(|v| v.as_str()))
            .unwrap_or("service_verification_failed");
        return ValidationObservation::Failed(detail.to_string());
    }
    let state = value
        .get("post_state")
        .and_then(|v| v.as_str())
        .or_else(|| value.get("pre_state").and_then(|v| v.as_str()))
        .unwrap_or_default();
    if !state.is_empty() {
        if service_state_is_healthy(state) {
            return ValidationObservation::Passed;
        }
        if service_state_looks_failed(state) {
            return ValidationObservation::Failed(state.to_string());
        }
    }
    if value
        .get("verified")
        .and_then(|v| v.as_bool())
        .is_some_and(|verified| verified)
    {
        return ValidationObservation::Passed;
    }
    ValidationObservation::Inconclusive
}

fn assess_health_check_validation(output: &str) -> ValidationObservation {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return ValidationObservation::Inconclusive;
    };
    let clawd_count = value.get("clawd_process_count").and_then(|v| v.as_u64());
    let telegramd_count = value
        .get("telegramd_process_count")
        .and_then(|v| v.as_u64());
    let clawd_port_open = value
        .get("clawd_health_port_open")
        .and_then(|v| v.as_bool());
    if clawd_count == Some(0) || clawd_port_open == Some(false) {
        return ValidationObservation::Failed("clawd_health_check_failed".to_string());
    }
    if telegramd_count == Some(0) {
        return ValidationObservation::Failed("telegramd_process_not_running".to_string());
    }
    if clawd_count.is_some() && clawd_port_open.is_some() {
        return ValidationObservation::Passed;
    }
    ValidationObservation::Inconclusive
}

fn has_strong_run_cmd_failure_marker(output_lower: &str) -> bool {
    contains_any(
        output_lower,
        &[
            "inactive",
            "stopped",
            "failed",
            "not running",
            "unhealthy",
            "validation_failed",
            "connection refused",
            "connection reset",
            "timed out",
            "timeout",
            "unreachable",
            "permission denied",
            "no such host",
            "could not",
            "syntax error",
            "panic",
            "error:",
            "not ok",
        ],
    )
}

fn command_declares_validation_sentinel(command: &str) -> bool {
    let command_lower = command.to_ascii_lowercase();
    command_lower.contains("validation_passed") || command_lower.contains("validation_failed")
}

fn declared_validation_sentinel_observation(command: &str, output: &str) -> ValidationObservation {
    if !command_declares_validation_sentinel(command) {
        return ValidationObservation::Inconclusive;
    }
    let has_passed = output
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("validation_passed"));
    let has_failed = output
        .lines()
        .any(|line| line.trim().eq_ignore_ascii_case("validation_failed"));
    match (has_passed, has_failed) {
        (true, false) => ValidationObservation::Passed,
        (false, true) => ValidationObservation::Failed(output.trim().to_string()),
        _ => ValidationObservation::Inconclusive,
    }
}

fn first_nonempty_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

fn second_nonempty_line(text: &str) -> Option<&str> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .nth(1)
}

fn output_is_exit_zero_sentinel(output: &str) -> bool {
    output
        .trim()
        .to_ascii_lowercase()
        .starts_with("exit=0 command=")
}

fn explicit_exit_status_observation(output: &str) -> Option<ValidationObservation> {
    output.lines().rev().find_map(|line| {
        let (key, value) = line.trim().split_once('=')?;
        let normalized_key = key.trim().trim_start_matches('-');
        if !matches!(
            normalized_key.to_ascii_lowercase().as_str(),
            "exit" | "exit_code"
        ) {
            return None;
        }
        let exit_code = value.trim().parse::<i32>().ok()?;
        Some(if exit_code == 0 {
            ValidationObservation::Passed
        } else {
            ValidationObservation::Failed(format!(
                "validation_command_exit_nonzero:exit_code={exit_code}"
            ))
        })
    })
}

pub(crate) fn run_cmd_successful_exit_is_validation(command: &str) -> bool {
    if !run_cmd_looks_validation(command) || command_declares_validation_sentinel(command) {
        return false;
    }
    let lower = command.trim().to_ascii_lowercase();
    if contains_any(
        &lower,
        &[
            "systemctl is-active",
            "systemctl status",
            " service status",
            "service --status-all",
            "nginx -t",
            "sing-box check",
        ],
    ) {
        return false;
    }
    !["curl", "wget", "nc", "ss", "lsof"]
        .into_iter()
        .any(|word| shell_contains_command_invocation(&lower, word))
}

fn assess_systemctl_is_active_validation(output: &str) -> ValidationObservation {
    match first_nonempty_line(output)
        .map(|line| line.to_ascii_lowercase())
        .as_deref()
    {
        Some("active") => ValidationObservation::Passed,
        Some("inactive" | "failed" | "deactivating" | "activating") => {
            ValidationObservation::Failed(output.trim().to_string())
        }
        _ => ValidationObservation::Inconclusive,
    }
}

fn assess_service_lifecycle_validation(output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    if lower.contains("active: active (running)")
        || lower.contains(" is running")
        || lower.contains("start/running")
    {
        return ValidationObservation::Passed;
    }
    if lower.contains("active: inactive")
        || lower.contains("active: failed")
        || lower.contains(" is not running")
        || lower.contains("stop/waiting")
        || lower.contains("inactive (dead)")
    {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    ValidationObservation::Inconclusive
}

fn assess_nginx_test_validation(output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    if output_is_exit_zero_sentinel(output) {
        return ValidationObservation::Passed;
    }
    if lower.contains("syntax is ok") && lower.contains("test is successful") {
        return ValidationObservation::Passed;
    }
    if has_strong_run_cmd_failure_marker(&lower) {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    ValidationObservation::Inconclusive
}

fn assess_sing_box_check_validation(output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    if output_is_exit_zero_sentinel(output)
        || lower.contains("configuration ok")
        || lower.contains("config ok")
        || lower.contains("check passed")
        || lower.contains("syntax is ok")
    {
        return ValidationObservation::Passed;
    }
    if has_strong_run_cmd_failure_marker(&lower)
        || lower.contains("decode config")
        || lower.contains("parse config")
    {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    ValidationObservation::Inconclusive
}

fn assess_http_probe_validation(command: &str, output: &str) -> ValidationObservation {
    let lower = output.trim().to_ascii_lowercase();
    let sentinel = declared_validation_sentinel_observation(command, output);
    if !matches!(sentinel, ValidationObservation::Inconclusive) {
        return sentinel;
    }
    if let Some(status_line) =
        first_nonempty_line(output).and_then(|line| line.strip_prefix("status="))
    {
        if let Ok(code) = status_line.trim().parse::<u16>() {
            return match code {
                200..=399 => ValidationObservation::Passed,
                _ => {
                    ValidationObservation::Failed(format!("http_status_not_success:status={code}"))
                }
            };
        }
    }
    if output_is_exit_zero_sentinel(output)
        && normalized_first_command_word(command)
            .as_deref()
            .is_some_and(|cmd| matches!(cmd, "curl" | "wget" | "nc"))
    {
        return ValidationObservation::Passed;
    }
    if has_strong_run_cmd_failure_marker(&lower) {
        return ValidationObservation::Failed(output.trim().to_string());
    }
    if command.to_ascii_lowercase().contains("grep") && !output.trim().is_empty() {
        return ValidationObservation::Passed;
    }
    ValidationObservation::Inconclusive
}

fn assess_socket_listing_validation(output: &str) -> ValidationObservation {
    let first = first_nonempty_line(output);
    let second = second_nonempty_line(output);
    match (first, second) {
        (Some(_header), Some(_row)) => ValidationObservation::Passed,
        (Some(_header), None) => {
            ValidationObservation::Failed("socket_listing_no_matching_rows".to_string())
        }
        _ => ValidationObservation::Inconclusive,
    }
}

fn assess_run_cmd_validation(command: &str, output: &str) -> ValidationObservation {
    if !run_cmd_looks_validation(command) {
        return ValidationObservation::Inconclusive;
    }
    if let Some(observation) = explicit_exit_status_observation(output) {
        return observation;
    }
    let command_lower = command.trim().to_ascii_lowercase();
    if command_lower.contains("systemctl is-active") {
        return assess_systemctl_is_active_validation(output);
    }
    if command_lower.contains("systemctl status")
        || command_lower.contains(" service status")
        || command_lower.contains("service --status-all")
    {
        return assess_service_lifecycle_validation(output);
    }
    if command_lower.contains("nginx -t") {
        return assess_nginx_test_validation(output);
    }
    if command_lower.contains("sing-box check") {
        return assess_sing_box_check_validation(output);
    }
    if normalized_first_command_word(command)
        .as_deref()
        .is_some_and(|cmd| matches!(cmd, "curl" | "wget" | "nc"))
    {
        return assess_http_probe_validation(command, output);
    }
    if normalized_first_command_word(command)
        .as_deref()
        .is_some_and(|cmd| matches!(cmd, "ss" | "lsof"))
    {
        return assess_socket_listing_validation(output);
    }
    declared_validation_sentinel_observation(command, output)
}

fn assess_system_basic_validation(args: &Value, output: &str) -> ValidationObservation {
    let action = args
        .get("action")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if action == "diagnose_runtime" {
        return assess_health_check_validation(output);
    }
    ValidationObservation::Inconclusive
}

fn assess_http_basic_validation(args: &Value, output: &str) -> ValidationObservation {
    let status_code = output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|line| line.strip_prefix("status="))
        .and_then(|digits| digits.trim().parse::<u16>().ok());
    match status_code {
        Some(code) => {
            if let Some(expected_status) = http_basic_expected_status(args) {
                if code == expected_status {
                    return ValidationObservation::Passed;
                }
                return ValidationObservation::Failed(format!(
                    "http_status_mismatch:status={code}:expected_status={expected_status}"
                ));
            }
            if http_basic_expect_success(args) && !(200..=299).contains(&code) {
                return ValidationObservation::Failed(format!(
                    "http_status_not_success:status={code}"
                ));
            }
            let expected = args
                .get("expect_contains")
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty());
            if let Some(expected) = expected {
                if !(200..=299).contains(&code) && !http_basic_accept_non_success(args) {
                    return ValidationObservation::Failed(format!(
                        "http_status_not_success:status={code}"
                    ));
                }
                let body = output.lines().skip(1).collect::<Vec<_>>().join("\n");
                if body.contains(expected) {
                    return ValidationObservation::Passed;
                } else {
                    return ValidationObservation::Failed(format!(
                        "http_expected_body_marker_missing:marker={expected}"
                    ));
                }
            }
            ValidationObservation::Passed
        }
        None => ValidationObservation::Inconclusive,
    }
}

fn http_basic_action_effect(args: &Value) -> ActionEffect {
    if http_basic_has_validation_expectation(args) || structured_validation_declared(args) {
        ActionEffect::validate()
    } else {
        ActionEffect::observe()
    }
}

fn http_basic_has_validation_expectation(args: &Value) -> bool {
    args.get("expect_contains")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
        || http_basic_expected_status(args).is_some()
        || args.get("expect_success").and_then(Value::as_bool) == Some(true)
        || args.get("require_success_status").and_then(Value::as_bool) == Some(true)
}

fn http_basic_expected_status(args: &Value) -> Option<u16> {
    args.get("expect_status")
        .or_else(|| args.get("expected_status"))
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.trim().parse::<u64>().ok())
        })
        .and_then(|value| u16::try_from(value).ok())
}

fn http_basic_expect_success(args: &Value) -> bool {
    args.get("expect_success").and_then(Value::as_bool) == Some(true)
        || args.get("require_success_status").and_then(Value::as_bool) == Some(true)
}

fn http_basic_accept_non_success(args: &Value) -> bool {
    args.get("accept_non_success").and_then(Value::as_bool) == Some(true)
        || args.get("allow_non_success").and_then(Value::as_bool) == Some(true)
}

fn structured_validation_result(value: &Value) -> ValidationObservation {
    let result = value
        .get("result")
        .or_else(|| value.get("status"))
        .and_then(Value::as_str)
        .map(|text| text.trim().to_ascii_lowercase());
    match result.as_deref() {
        Some("passed" | "pass" | "ok" | "success" | "succeeded") => {
            return ValidationObservation::Passed;
        }
        Some("failed" | "fail" | "error" | "rejected") => {
            let detail = value
                .get("detail")
                .or_else(|| value.get("reason"))
                .or_else(|| value.get("error"))
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or("structured_validation_failed");
            return ValidationObservation::Failed(detail.to_string());
        }
        _ => {}
    }
    if let Some(passed) = value
        .get("passed")
        .or_else(|| value.get("valid"))
        .or_else(|| value.get("satisfies_contract"))
        .and_then(Value::as_bool)
    {
        if passed {
            return ValidationObservation::Passed;
        }
        let detail = value
            .get("detail")
            .or_else(|| value.get("reason"))
            .or_else(|| value.get("error"))
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
            .unwrap_or("structured_validation_failed");
        return ValidationObservation::Failed(detail.to_string());
    }
    ValidationObservation::Inconclusive
}

fn structured_validation_from_output_text(output: &str) -> ValidationObservation {
    let Ok(value) = serde_json::from_str::<Value>(output) else {
        return ValidationObservation::Inconclusive;
    };
    if let Some(validation) = value.get("validation") {
        return structured_validation_result(validation);
    }
    value
        .get("extra")
        .and_then(|extra| extra.get("validation"))
        .map(structured_validation_result)
        .unwrap_or(ValidationObservation::Inconclusive)
}

fn declared_validation_success_fallback(
    _state: &AppState,
    _skill_name: &str,
    _output: &str,
) -> ValidationObservation {
    ValidationObservation::Passed
}

pub(crate) fn assess_validation_output_with_structured(
    state: &AppState,
    skill_name: &str,
    args: &Value,
    output: &str,
    structured_validation: Option<&Value>,
) -> ValidationObservation {
    if let Some(validation) = structured_validation {
        let observation = structured_validation_result(validation);
        if !matches!(observation, ValidationObservation::Inconclusive) {
            return observation;
        }
    }
    let observation_from_text = structured_validation_from_output_text(output);
    if !matches!(observation_from_text, ValidationObservation::Inconclusive) {
        return observation_from_text;
    }
    let marker_observation = structured_success_marker_observation(args, output);
    if !matches!(marker_observation, ValidationObservation::Inconclusive) {
        return marker_observation;
    }
    let observation = match state.resolve_canonical_skill_name(skill_name).as_str() {
        "service_control" => assess_service_control_validation(output),
        "health_check" => assess_health_check_validation(output),
        "http_basic" => assess_http_basic_validation(args, output),
        "system_basic" => assess_system_basic_validation(args, output),
        "run_cmd" => args
            .get("command")
            .and_then(|value| value.as_str())
            .map(|command| assess_run_cmd_validation(command, output))
            .unwrap_or(ValidationObservation::Inconclusive),
        _ => ValidationObservation::Inconclusive,
    };
    if matches!(observation, ValidationObservation::Inconclusive)
        && structured_validation_declared(args)
    {
        return declared_validation_success_fallback(state, skill_name, output);
    }
    observation
}

#[cfg(test)]
pub(crate) fn assess_validation_output(
    state: &AppState,
    skill_name: &str,
    args: &Value,
    output: &str,
) -> ValidationObservation {
    assess_validation_output_with_structured(state, skill_name, args, output, None)
}

pub(crate) fn stop_signal_for_validation_failure(
    state: &ExecutionRecipeRuntimeState,
) -> &'static str {
    if state.is_active() && state.repair_count > state.max_repairs {
        "recipe_repair_budget_exhausted"
    } else {
        "recoverable_failure_continue_round"
    }
}

pub(crate) fn effective_action_effect_for_recipe(
    state: ExecutionRecipeRuntimeState,
    effect: ActionEffect,
) -> ActionEffect {
    if state.is_active() && effect.validates && !effect.mutates && !state.saw_mutation {
        return ActionEffect::observe();
    }
    effect
}

pub(crate) fn apply_action_effect_success(
    state: &mut ExecutionRecipeRuntimeState,
    effect: ActionEffect,
) {
    if !state.is_active() {
        return;
    }
    if effect.observes {
        state.saw_inspect = true;
    }
    if effect.mutates {
        state.saw_mutation = true;
        state.saw_validation = false;
    }
    if effect.validates && state.saw_mutation {
        state.saw_validation = true;
        state.phase = ExecutionRecipePhase::Done;
        return;
    }
    if effect.mutates {
        state.phase = ExecutionRecipePhase::Validate;
        return;
    }
    if matches!(state.phase, ExecutionRecipePhase::Inspect) && state.saw_inspect {
        state.phase = ExecutionRecipePhase::Apply;
    }
}

pub(crate) fn apply_action_effect_failure(
    state: &mut ExecutionRecipeRuntimeState,
    effect: ActionEffect,
) {
    if !state.is_active() {
        return;
    }
    if effect.observes {
        state.saw_inspect = true;
        if matches!(state.phase, ExecutionRecipePhase::Inspect) && !state.saw_mutation {
            state.phase = ExecutionRecipePhase::Apply;
        }
    }
    if effect.validates && state.saw_mutation && !state.saw_validation {
        state.repair_count += 1;
        state.phase = ExecutionRecipePhase::Repair;
    }
}

#[cfg(test)]
#[path = "execution_recipe_tests.rs"]
mod tests;
