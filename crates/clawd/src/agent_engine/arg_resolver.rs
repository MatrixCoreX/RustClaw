use serde_json::Value;
use std::path::Path;

use super::LoopState;

pub(super) fn rewrite_run_cmd_with_written_aliases(
    command: &str,
    loop_state: &LoopState,
) -> String {
    if loop_state.written_file_aliases.is_empty() {
        return command.to_string();
    }
    let mut rewritten = command.to_string();
    for token in command.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| matches!(c, '"' | '\''));
        if trimmed.is_empty() {
            continue;
        }
        for (alias, effective) in &loop_state.written_file_aliases {
            if trimmed == alias || trimmed == alias.trim_start_matches("./") {
                rewritten = rewritten.replace(token, &token.replace(trimmed, effective));
                break;
            }
        }
    }
    rewritten
}

pub(super) fn rewrite_tool_path_with_written_aliases(
    tool: &str,
    args: &mut Value,
    loop_state: &LoopState,
) {
    if !matches!(tool, "read_file" | "remove_file") || loop_state.written_file_aliases.is_empty() {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    let Some(path) = obj.get("path").and_then(|v| v.as_str()) else {
        return;
    };
    let normalized = path.trim().trim_start_matches("./");
    let Some(effective) = loop_state.written_file_aliases.get(normalized) else {
        return;
    };
    obj.insert("path".to_string(), Value::String(effective.clone()));
}

pub(super) fn attach_recent_execution_context_to_chat_args(
    args: &mut Value,
    loop_state: &LoopState,
) {
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    if obj.contains_key("recent_execution_context") {
        return;
    }
    let mut context_lines = Vec::new();
    if let Some(path) = loop_state.last_written_file_path.as_deref() {
        context_lines.push(format!("last_written_file_path: {path}"));
    }
    if let Some(path) = loop_state.output_vars.get("last_file_path") {
        context_lines.push(format!("last_file_path: {path}"));
    }
    if let Some(path) = loop_state.output_vars.get("last_read_file_path") {
        context_lines.push(format!("last_read_file_path: {path}"));
    }
    if let Some(output) = loop_state
        .last_output
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        context_lines.push(format!("last_output: {}", crate::truncate_for_log(output)));
    }
    // Multi-step intra-turn bridge: when the LLM plan contains multiple observation steps
    // (e.g. `[read_file(乙), read_file(甲)]`), `last_output` only carries the last step's
    // output (甲), and earlier steps' real outputs (乙) would be silently dropped before the
    // chat-skill sees them. Surface every prior OK observation step so multi-evidence
    // questions ("读一下乙的开头，然后顺手说甲是干什么的", "对比文件 A 和 B") can ground
    // on full evidence. Skip the last step (already in `last_output`) and skip non-evidence
    // steps (chat / respond) to avoid feeding the chat-skill its own prior reply.
    {
        use crate::executor::StepExecutionStatus;
        let evidence_steps: Vec<(usize, &crate::executor::StepExecutionResult)> = loop_state
            .executed_step_results
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                matches!(s.status, StepExecutionStatus::Ok)
                    && !s.skill.eq_ignore_ascii_case("chat")
                    && !s.skill.eq_ignore_ascii_case("respond")
                    && s.output
                        .as_deref()
                        .map(|o| !o.trim().is_empty())
                        .unwrap_or(false)
            })
            .collect();
        if evidence_steps.len() > 1 {
            // All but the final evidence step (final one is already exposed via last_output).
            let prior = &evidence_steps[..evidence_steps.len() - 1];
            let mut prior_lines = Vec::with_capacity(prior.len());
            for (idx, step) in prior {
                let out = step.output.as_deref().unwrap_or("");
                prior_lines.push(format!(
                    "step[{}] skill={}: {}",
                    idx + 1,
                    step.skill,
                    crate::truncate_for_log(out.trim())
                ));
            }
            context_lines.push(format!(
                "prior_step_outputs (earlier observation steps in current turn; treat as authoritative observation evidence the same way as last_output):\n{}",
                prior_lines.join("\n")
            ));
        }
    }
    // Cross-turn bridge: when current turn references prior turns ("上一个文件 / 上上个 /
    // 那个文件 / 甲 / 乙" / "对比" / "比较" / "用 X 解释 Y"), the chat-skill LLM only sees
    // intra-turn last_output above. Append the task-level recent_execution_context (rendered
    // from past turns' tasks) so chat skill can ground its answer in earlier turns' outputs.
    if let Some(cross_turn) = loop_state
        .output_vars
        .get("cross_turn_recent_execution_context")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        context_lines.push(format!(
            "cross_turn_recent_execution_context (prior turns in this conversation; treat as authoritative observation evidence the same way as last_output):\n{cross_turn}"
        ));
    }
    if context_lines.is_empty() {
        return;
    }
    obj.insert(
        "recent_execution_context".to_string(),
        Value::String(context_lines.join("\n")),
    );
}

fn rewrite_path_field(args: &mut Value, auto_locator_path: &str) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    match obj.get("path").and_then(|v| v.as_str()) {
        Some(current) if current == auto_locator_path => false,
        Some(_) => {
            obj.insert(
                "path".to_string(),
                Value::String(auto_locator_path.to_string()),
            );
            true
        }
        None => false,
    }
}

fn rewrite_root_field(args: &mut Value, auto_locator_path: &str) -> bool {
    let Some(obj) = args.as_object_mut() else {
        return false;
    };
    match obj.get("root").and_then(|v| v.as_str()) {
        Some(current) if current == auto_locator_path => false,
        Some(_) => {
            obj.insert(
                "root".to_string(),
                Value::String(auto_locator_path.to_string()),
            );
            true
        }
        None => false,
    }
}

pub(super) fn rewrite_args_with_auto_locator_path(
    normalized_skill: &str,
    args: &mut Value,
    loop_state: &LoopState,
) -> bool {
    let Some(auto_locator_path) = loop_state
        .output_vars
        .get("auto_locator_path")
        .map(String::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return false;
    };
    let auto_path = Path::new(auto_locator_path);
    match normalized_skill {
        "read_file" if auto_path.is_file() => rewrite_path_field(args, auto_locator_path),
        "list_dir" if auto_path.is_dir() => rewrite_path_field(args, auto_locator_path),
        "system_basic" => {
            let action = args
                .as_object()
                .and_then(|obj| obj.get("action"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match action {
                "extract_field" | "extract_fields" | "structured_keys" | "read_range"
                    if auto_path.is_file() =>
                {
                    rewrite_path_field(args, auto_locator_path)
                }
                "inventory_dir" | "count_inventory" | "workspace_glance" | "tree_summary"
                    if auto_path.is_dir() =>
                {
                    rewrite_path_field(args, auto_locator_path)
                }
                "find_path" if auto_path.is_dir() => rewrite_root_field(args, auto_locator_path),
                _ => false,
            }
        }
        _ => false,
    }
}

fn replace_double_brace_placeholders(
    input: &str,
    vars: &std::collections::HashMap<String, String>,
) -> String {
    let mut out = input.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

fn single_brace_key(input: &str) -> Option<&str> {
    if !(input.starts_with('{') && input.ends_with('}')) {
        return None;
    }
    let inner = &input[1..input.len().saturating_sub(1)];
    if inner.is_empty() || inner.contains('{') || inner.contains('}') {
        return None;
    }
    Some(inner)
}

fn angle_bracket_key(input: &str) -> Option<&str> {
    if !(input.starts_with('<') && input.ends_with('>')) {
        return None;
    }
    let inner = &input[1..input.len().saturating_sub(1)];
    if inner.is_empty() || inner.contains('<') || inner.contains('>') {
        return None;
    }
    Some(inner)
}

pub(super) fn resolve_arg_string(input: &str, loop_state: &LoopState) -> String {
    let replaced = replace_double_brace_placeholders(input, &loop_state.output_vars);
    if let Some(key) = single_brace_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
    }
    if let Some(key) = angle_bracket_key(replaced.trim()) {
        if let Some(v) = loop_state.output_vars.get(key) {
            return v.clone();
        }
        let normalized_key = key.trim().to_ascii_lowercase();
        if let Some(v) = loop_state.output_vars.get(&normalized_key) {
            return v.clone();
        }
    }
    replaced
}

pub(super) fn resolve_arg_value(value: &Value, loop_state: &LoopState) -> Value {
    match value {
        Value::String(s) => Value::String(resolve_arg_string(s, loop_state)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|v| resolve_arg_value(v, loop_state))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                out.insert(k.clone(), resolve_arg_value(v, loop_state));
            }
            Value::Object(out)
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::rewrite_args_with_auto_locator_path;
    use crate::agent_engine::LoopState;
    use serde_json::json;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDirGuard {
        path: PathBuf,
    }

    impl TempDirGuard {
        fn new(prefix: &str) -> Self {
            let mut path = std::env::temp_dir();
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time before unix epoch")
                .as_nanos();
            path.push(format!(
                "clawd_arg_resolver_{prefix}_{}_{}",
                std::process::id(),
                nanos
            ));
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn auto_locator_rewrites_system_basic_file_path() {
        let root = TempDirGuard::new("readme_file");
        let readme = root.path.join("README.md");
        fs::write(&readme, "# title\n").expect("write readme");
        let readme_path = readme.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), readme_path.clone());
        let mut args = json!({
            "action": "read_range",
            "path": "/tmp/README",
            "mode": "head",
            "n": 20
        });
        assert!(rewrite_args_with_auto_locator_path(
            "system_basic",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("path").and_then(|v| v.as_str()),
            Some(readme_path.as_str())
        );
    }

    #[test]
    fn auto_locator_rewrites_directory_root_for_find_path() {
        let root = TempDirGuard::new("workspace_dir");
        let document = root.path.join("document");
        fs::create_dir_all(&document).expect("create document");
        let document_path = document.display().to_string();
        let mut loop_state = LoopState::new(2);
        loop_state
            .output_vars
            .insert("auto_locator_path".to_string(), document_path.clone());
        let mut args = json!({
            "action": "find_path",
            "root": ".",
            "name": "manual_note.txt"
        });
        assert!(rewrite_args_with_auto_locator_path(
            "system_basic",
            &mut args,
            &loop_state
        ));
        assert_eq!(
            args.get("root").and_then(|v| v.as_str()),
            Some(document_path.as_str())
        );
    }
}
