use super::*;

pub(super) fn filesystem_mutation_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    user_text: &str,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.has_tool_or_skill_output
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || !route.output_contract_marker_is(crate::OutputSemanticKind::FilesystemMutationResult)
        || !fs_basic_available_for_plan(state)
    {
        return None;
    }
    let root = normalized_scratch_root(route.output_contract.locator_hint.as_str())?;
    let file_path = child_file_path_for_request(&root, user_text)?;
    let content_tokens = request_content_tokens(user_text, &root, &file_path);
    if content_tokens.len() != 2 {
        return None;
    }
    let write_content = format!("{}\n", content_tokens[0]);
    let append_content = format!("{}\n", content_tokens[1]);
    let actions = vec![
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({"action": "make_dir", "path": root}),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "write_text",
                "path": file_path,
                "content": write_content,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "append_text",
                "path": file_path,
                "content": append_content,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "read_text_range",
                "path": file_path,
                "mode": "head",
                "n": 20,
            }),
        },
        AgentAction::CallTool {
            tool: "fs_basic".to_string(),
            args: serde_json::json!({
                "action": "remove_path",
                "path": root,
                "target_kind": "directory",
                "recursive": true,
            }),
        },
        AgentAction::SynthesizeAnswer {
            evidence_refs: vec![
                "step_1".to_string(),
                "step_2".to_string(),
                "step_3".to_string(),
                "step_4".to_string(),
                "step_5".to_string(),
            ],
        },
        AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        },
    ];
    if !actions.iter().all(|action| action_allowed(route, action)) {
        return None;
    }
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn action_allowed(route: &RouteResult, action: &AgentAction) -> bool {
    let AgentAction::CallTool { tool, args } = action else {
        return true;
    };
    crate::contract_matrix::action_policy_for_route(Some(route), tool, args)
        .is_some_and(|policy| policy.is_allowed())
}

fn fs_basic_available_for_plan(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("fs_basic")
}

fn normalized_scratch_root(locator_hint: &str) -> Option<String> {
    let root = locator_hint.trim().trim_matches('/');
    if root.is_empty()
        || root.starts_with('/')
        || root.split('/').any(|part| part == ".." || part.is_empty())
        || !root.starts_with("tmp/")
    {
        return None;
    }
    Some(root.to_string())
}

fn child_file_path_for_request(root: &str, user_text: &str) -> Option<String> {
    let root_prefix = format!("{root}/");
    for token in pathish_tokens(user_text) {
        if !has_file_extension(token) {
            continue;
        }
        if token.starts_with(&root_prefix) {
            return Some(token.to_string());
        }
        if !token.contains('/') {
            return Some(format!("{root}/{token}"));
        }
    }
    None
}

fn request_content_tokens(user_text: &str, root: &str, file_path: &str) -> Vec<String> {
    let excluded = excluded_ascii_tokens(root, file_path);
    user_text
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|token| {
            !token.is_empty()
                && token.len() <= 64
                && token.chars().any(|ch| ch.is_ascii_alphabetic())
                && !excluded.contains(&token.to_ascii_lowercase())
        })
        .map(ToString::to_string)
        .collect()
}

fn excluded_ascii_tokens(root: &str, file_path: &str) -> std::collections::BTreeSet<String> {
    [root, file_path]
        .into_iter()
        .flat_map(|value| {
            value
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
                .map(str::trim)
                .filter(|token| !token.is_empty())
                .map(|token| token.to_ascii_lowercase())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn pathish_tokens(text: &str) -> impl Iterator<Item = &str> {
    text.split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/')))
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

fn has_file_extension(token: &str) -> bool {
    let Some((_, ext)) = token.rsplit_once('.') else {
        return false;
    };
    !ext.is_empty()
        && ext.len() <= 10
        && ext
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}
