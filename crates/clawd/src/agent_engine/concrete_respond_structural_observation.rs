use super::*;

pub(super) fn concrete_respond_has_structural_observation_anchors(
    loop_state: &LoopState,
    content: &str,
) -> bool {
    let content = content.trim();
    if content.is_empty() || content.contains("{{") {
        return false;
    }
    let haystack = content.replace('\\', "/").to_ascii_lowercase();
    let mut matched = HashSet::new();
    for output in loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.is_ok()
                && !matches!(
                    step.skill.as_str(),
                    "respond" | "think" | "synthesize_answer"
                )
        })
        .filter_map(|step| step.output.as_deref())
    {
        let mut tokens = Vec::new();
        if let Ok(value) = serde_json::from_str::<Value>(output) {
            push_structural_grounding_tokens(&value, &mut tokens);
        } else {
            push_textual_grounding_tokens(output, &mut tokens);
            tokens.extend(
                output
                    .lines()
                    .map(str::trim)
                    .filter(|line| {
                        line.len() >= 3
                            && !line.chars().any(char::is_whitespace)
                            && (line.contains('/')
                                || line.contains('.')
                                || line.contains('_')
                                || line.contains('-')
                                || line.chars().all(|ch| ch.is_ascii_digit()))
                    })
                    .map(ToString::to_string),
            );
        }
        for token in tokens {
            let token = token.trim().to_ascii_lowercase();
            if token.len() >= 2 && haystack.contains(&token) {
                matched.insert(token);
                if matched.len() >= 2 {
                    return true;
                }
            }
        }
    }
    false
}

pub(super) fn route_allows_model_language_terminal_respond(route: Option<&RouteResult>) -> bool {
    let Some(route) = route else {
        return false;
    };
    if route.output_contract.delivery_required
        || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
    {
        return false;
    }
    crate::evidence_policy::final_answer_shape_for_route(route)
        .is_some_and(|shape| shape.allows_model_language())
}

pub(super) fn rewrite_observed_terminal_synthesis_concrete_respond(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2
        || !loop_state.has_tool_or_skill_output
        || !route_should_prefer_observed_terminal_synthesis(route_result)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    if !matches!(
        actions.get(last_idx - 1),
        Some(AgentAction::SynthesizeAnswer { .. })
    ) {
        return actions;
    }
    let Some(AgentAction::Respond { content }) = actions.get(last_idx) else {
        return actions;
    };
    if !is_concrete_final_respond_content(content) {
        return actions;
    }
    if route_result.is_some_and(|route| {
        route.output_contract.semantic_kind == crate::OutputSemanticKind::RecentScalarEqualityCheck
    }) {
        let mut rewritten = actions;
        rewritten[last_idx] = AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        };
        info!("plan_rewrite_scalar_equality_concrete_respond_after_synthesis");
        return rewritten;
    }
    if route_allows_model_language_terminal_respond(route_result)
        && concrete_respond_has_structural_observation_anchors(loop_state, content)
    {
        let mut rewritten = actions;
        rewritten.remove(last_idx - 1);
        info!("plan_drop_redundant_observed_synthesis_before_grounded_respond");
        return rewritten;
    }
    let content_excerpt_contract = route_result.is_some_and(|route| {
        route
            .output_contract
            .semantic_kind
            .is_content_excerpt_summary()
    });
    if content_excerpt_contract {
        let mut rewritten = actions;
        rewritten[last_idx] = AgentAction::Respond {
            content: "{{last_output}}".to_string(),
        };
        info!("plan_rewrite_content_excerpt_concrete_respond_after_synthesis");
        return rewritten;
    }
    if concrete_respond_has_structural_observation_anchors(loop_state, content) {
        info!("plan_keep_structurally_grounded_concrete_respond_after_synthesis");
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    info!("plan_rewrite_observed_terminal_synthesis_concrete_respond");
    rewritten
}

/// Planner-first shape guard: before any observation has run, a leading
/// `synthesize_answer` is redundant when a later concrete `respond` already
/// exists.
///
/// This is not a natural-language shortcut; it only repairs the plan graph so
/// a redundant synthesis node does not block an already concrete final answer.
pub(super) fn strip_pre_observation_synthesize_before_concrete_respond(
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 {
        return actions;
    }
    if loop_state.has_tool_or_skill_output || !loop_state.executed_step_results.is_empty() {
        return actions;
    }

    let mut has_future_concrete_respond = vec![false; actions.len()];
    let mut future_concrete_respond = false;
    for idx in (0..actions.len()).rev() {
        has_future_concrete_respond[idx] = future_concrete_respond;
        if let AgentAction::Respond { content } = &actions[idx] {
            future_concrete_respond |= is_concrete_final_respond_content(content);
        }
    }
    if !future_concrete_respond {
        return actions;
    }

    let mut rewritten = Vec::with_capacity(actions.len());
    let mut saw_observation_action = false;
    let mut dropped = 0usize;
    for (idx, action) in actions.into_iter().enumerate() {
        match &action {
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. } => {
                saw_observation_action = true;
                rewritten.push(action);
            }
            AgentAction::SynthesizeAnswer { .. }
                if !saw_observation_action && has_future_concrete_respond[idx] =>
            {
                dropped += 1;
            }
            _ => rewritten.push(action),
        }
    }

    if dropped > 0 {
        info!("plan_strip_pre_observation_synthesize_before_concrete_respond dropped={dropped}");
    }
    rewritten
}

/// §F1：检测「未观测先编造」的幻觉 Respond，把内容改写为 `{{last_output}}` 占位，
/// 让下游 [`inject_synthesize_answer_for_bare_placeholder_respond`] 把它包成
/// `synthesize_answer` 节点，从而在执行完上游观测步后再用真实输出生成回复。
///
/// 触发条件（必须**全部**满足）：
/// 1. `loop_state` 仍是 round-1 状态：`executed_step_results` 为空（没有任何
///    skill 实际跑过），`last_output` 为空 → 这一批 actions 全部都还没执行。
/// 2. `actions` 末尾是 `Respond` 步。
/// 3. 倒数第二步是 `CallSkill` / `CallTool`（即「先跑后说」的常见 plan 形态）。
/// 4. Respond 的 content 不包含 `{{last_output}}` 之类的占位符
///    （[`is_bare_last_output_placeholder`] 已经在主入口处理纯占位符路径），
///    并且 content 长度足够 + 含「观测过才能知道」的特征 token：
///    - 含至少一行以数字+点开头的列表项（`1. xxx` / `2. xxx` …）；或
///    - 含 3+ 行换行 + 至少一个文件路径字符（`/`、`.md`、`.toml` 等）；或
///    - 含 `result: ` / `count: ` / `size: ` 这种结构化字段标签。
///
/// 这一招针对兼容模型偶发的「planner 一次性把 list_dir + respond 编造
/// 内容写在同一个 plan，respond 直接交给用户」复现路径。
/// 不命中条件时 actions 原样返回，不破坏正确 plan。
pub(super) fn rewrite_pre_observation_concrete_respond_to_placeholder(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_result.is_some_and(|route| {
        route.output_contract.delivery_required
            || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
    }) {
        return actions;
    }
    if actions.len() < 2 {
        return actions;
    }
    if !loop_state.executed_step_results.is_empty() {
        return actions;
    }
    if loop_state
        .last_output
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.clone(),
        _ => return actions,
    };
    if crate::finalize::parse_delivery_file_token(respond_content.trim()).is_some() {
        return actions;
    }
    let prior_is_observation = matches!(
        &actions[last_idx - 1],
        AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
    );
    if !prior_is_observation {
        return actions;
    }
    if is_bare_last_output_placeholder(&respond_content) {
        return actions;
    }
    if generated_file_path_report_respond_matches_planned_write(
        state,
        route_result,
        &actions,
        respond_content.trim(),
    ) {
        return actions;
    }
    let contract_requires_observed_answer = route_result
        .map(|route| route.output_contract.requires_content_evidence)
        .unwrap_or(false);
    if !contract_requires_observed_answer
        && !has_pre_observation_structured_output_shape(&respond_content)
    {
        return actions;
    }
    let mut rewritten = actions;
    let original_len = respond_content.len();
    let respond_idx = rewritten.len() - 1;
    if let AgentAction::Respond { content } = &mut rewritten[respond_idx] {
        *content = "{{last_output}}".to_string();
    }
    info!(
        "plan_rewrite_pre_observation_concrete_respond_to_placeholder original_len={} source={}",
        original_len,
        if contract_requires_observed_answer {
            "output_contract"
        } else {
            "shape_guard"
        }
    );
    rewritten
}

pub(super) fn generated_file_path_report_respond_matches_planned_write(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
    respond_content: &str,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::GeneratedFilePathReport
        || route.output_contract.delivery_required
        || route.output_contract.response_shape != crate::OutputResponseShape::Scalar
        || !field_token_looks_like_locator(respond_content)
    {
        return false;
    }
    actions.iter().any(|action| {
        let (tool, args) = match action {
            AgentAction::CallTool { tool, args } | AgentAction::CallSkill { skill: tool, args } => {
                (tool.as_str(), args)
            }
            _ => return false,
        };
        if !tool.eq_ignore_ascii_case("fs_basic")
            || args.get("action").and_then(Value::as_str).map(str::trim) != Some("write_text")
        {
            return false;
        }
        let Some(write_path) =
            json_trimmed_string_arg(args, &["path", "file", "file_path", "target"])
        else {
            return false;
        };
        same_existing_or_display_path(
            &resolve_workspace_path(&state.skill_rt.workspace_root, &write_path),
            &resolve_workspace_path(&state.skill_rt.workspace_root, respond_content),
        )
    })
}

pub(super) fn rewrite_terminal_placeholder_respond_to_synthesize_answer(
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 || !loop_state.executed_step_results.is_empty() {
        return actions;
    }
    if loop_state
        .last_output
        .as_deref()
        .map(|text| !text.trim().is_empty())
        .unwrap_or(false)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    if is_bare_last_output_placeholder(respond_content) {
        return actions;
    }
    let evidence_refs = extract_output_placeholder_evidence_refs(respond_content);
    if evidence_refs.is_empty() {
        return actions;
    }
    if mixed_last_output_respond_has_concrete_text(respond_content, &evidence_refs) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !matches!(
        previous_action,
        AgentAction::CallSkill { .. }
            | AgentAction::CallTool { .. }
            | AgentAction::SynthesizeAnswer { .. }
    ) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    rewritten.insert(
        last_idx,
        AgentAction::SynthesizeAnswer {
            evidence_refs: evidence_refs.clone(),
        },
    );
    info!(
        "plan_rewrite_terminal_placeholder_respond_to_synthesize_answer refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

pub(super) fn content_excerpt_summary_target_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || route.output_contract.delivery_required
        || !(route
            .output_contract
            .semantic_kind
            .is_content_excerpt_summary()
            || route.output_contract.semantic_kind
                == crate::OutputSemanticKind::ExcerptKindJudgment)
    {
        return None;
    }
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            (!hint.is_empty() && Path::new(hint).is_file()).then_some(hint)
        })
        .filter(|path| Path::new(path).is_file())
        .map(ToString::to_string)
}

pub(super) fn ensure_content_excerpt_summary_has_bounded_content(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    let Some(path) = content_excerpt_summary_target_path(route_result, auto_locator_path) else {
        return actions;
    };
    let mut rewritten = actions;
    if !rewritten.iter().any(action_observes_bounded_file_content) {
        let insert_at = rewritten
            .iter()
            .position(|action| {
                matches!(
                    action,
                    AgentAction::SynthesizeAnswer { .. } | AgentAction::Respond { .. }
                )
            })
            .unwrap_or(rewritten.len());
        rewritten.insert(
            insert_at,
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "read_text_range",
                    "path": path,
                    "mode": "head",
                    "n": 40
                }),
            },
        );
        info!("plan_insert_content_excerpt_summary_read_range");
    }
    if !rewritten
        .iter()
        .any(|action| matches!(action, AgentAction::SynthesizeAnswer { .. }))
    {
        let evidence_refs = observation_action_evidence_refs(&rewritten);
        if !evidence_refs.is_empty() {
            let insert_at = rewritten
                .iter()
                .rposition(|action| matches!(action, AgentAction::Respond { .. }))
                .unwrap_or(rewritten.len());
            rewritten.insert(
                insert_at,
                AgentAction::SynthesizeAnswer {
                    evidence_refs: evidence_refs.clone(),
                },
            );
            match rewritten.get_mut(insert_at + 1) {
                Some(AgentAction::Respond { content }) => {
                    *content = "{{last_output}}".to_string();
                }
                _ => rewritten.push(AgentAction::Respond {
                    content: "{{last_output}}".to_string(),
                }),
            }
            info!(
                "plan_insert_content_excerpt_summary_synthesis refs={}",
                evidence_refs.join(",")
            );
        }
    }
    rewritten
}

pub(super) fn rewrite_terminal_synthesis_placeholder_respond(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    if !is_bare_template_placeholder(respond_content) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !matches!(previous_action, AgentAction::SynthesizeAnswer { .. }) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    info!("plan_rewrite_terminal_synthesis_placeholder_respond");
    rewritten
}

pub(super) fn route_requires_observed_synthesis_for_mixed_placeholder(route: &RouteResult) -> bool {
    route.output_contract.requires_content_evidence
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::RecentArtifactsJudgment
                | crate::OutputSemanticKind::DirectoryPurposeSummary
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
                | crate::OutputSemanticKind::ExcerptKindJudgment
                | crate::OutputSemanticKind::WorkspaceProjectSummary
                | crate::OutputSemanticKind::SqliteDatabaseKindJudgment
                | crate::OutputSemanticKind::ServiceStatus
        )
}

pub(super) fn rewrite_mixed_placeholder_observed_synthesis_respond(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !route_requires_observed_synthesis_for_mixed_placeholder(route)
        || actions.len() < 2
        || has_loop_observation(loop_state)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(respond_content);
    if !mixed_last_output_respond_has_concrete_text(respond_content, &evidence_refs) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !matches!(
        previous_action,
        AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
    ) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    rewritten.insert(
        last_idx,
        AgentAction::SynthesizeAnswer {
            evidence_refs: if evidence_refs.is_empty() {
                vec!["last_output".to_string()]
            } else {
                evidence_refs.clone()
            },
        },
    );
    info!(
        "plan_rewrite_mixed_placeholder_observed_synthesis_respond refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

pub(super) fn action_emits_structured_output_for_placeholder(action: &AgentAction) -> bool {
    match action {
        AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args } => {
            let action_name = args
                .get("action")
                .and_then(Value::as_str)
                .map(|action| action.trim().to_ascii_lowercase());
            let action_name = action_name.as_deref();
            match skill.as_str() {
                "fs_basic" => matches!(
                    action_name,
                    Some(
                        "stat_paths"
                            | "list_dir"
                            | "count_entries"
                            | "read_text_range"
                            | "find_entries"
                            | "grep_text"
                            | "compare_paths"
                    )
                ),
                "system_basic" => matches!(
                    action_name,
                    Some(
                        "inventory_dir"
                            | "count_inventory"
                            | "workspace_glance"
                            | "tree_summary"
                            | "extract_field"
                            | "extract_fields"
                            | "structured_keys"
                            | "validate_structured"
                            | "find_path"
                            | "read_range"
                            | "compare_paths"
                            | "path_batch_facts"
                            | "diagnose_runtime"
                    )
                ),
                "config_basic" => matches!(
                    action_name,
                    Some("read_field" | "read_fields" | "list_keys" | "validate")
                ),
                _ => false,
            }
        }
        _ => false,
    }
}

pub(super) fn rewrite_mixed_placeholder_structured_output_respond(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if route_explicitly_requests_raw_command_output(route_result)
        || actions.len() < 2
        || has_loop_observation(loop_state)
    {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let respond_content = match &actions[last_idx] {
        AgentAction::Respond { content } => content.as_str(),
        _ => return actions,
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(respond_content);
    if !mixed_last_output_respond_has_concrete_text(respond_content, &evidence_refs) {
        return actions;
    }
    let Some(previous_action) = actions[..last_idx]
        .iter()
        .rev()
        .find(|candidate| !matches!(candidate, AgentAction::Think { .. }))
    else {
        return actions;
    };
    if !action_emits_structured_output_for_placeholder(previous_action) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten[last_idx] = AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    };
    rewritten.insert(
        last_idx,
        AgentAction::SynthesizeAnswer {
            evidence_refs: evidence_refs.clone(),
        },
    );
    info!(
        "plan_rewrite_mixed_placeholder_structured_output_respond refs={}",
        evidence_refs.join(",")
    );
    rewritten
}

pub(super) fn mixed_last_output_respond_has_concrete_text(
    content: &str,
    evidence_refs: &[String],
) -> bool {
    if evidence_refs.is_empty()
        || !evidence_refs
            .iter()
            .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
    {
        return false;
    }
    static PLACEHOLDER_BLOCK_RE: OnceLock<Regex> = OnceLock::new();
    let block_re = PLACEHOLDER_BLOCK_RE
        .get_or_init(|| Regex::new(r"\{\{\s*([^{}]+?)\s*\}\}").expect("placeholder block regex"));
    let outside_placeholder_text = block_re.replace_all(content, "");
    !outside_placeholder_text.trim().is_empty()
}

pub(super) fn has_mixed_last_output_terminal_respond(actions: &[AgentAction]) -> bool {
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return false;
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(content);
    mixed_last_output_respond_has_concrete_text(content, &evidence_refs)
}

pub(super) fn terminal_mixed_last_output_respond_content(
    actions: &[AgentAction],
) -> Option<String> {
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return None;
    };
    let evidence_refs = extract_output_placeholder_evidence_refs(content);
    mixed_last_output_respond_has_concrete_text(content, &evidence_refs).then(|| content.clone())
}

pub(super) fn restore_terminal_mixed_last_output_respond(
    route_result: Option<&RouteResult>,
    planned_content: Option<String>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !route_result.is_some_and(|route| {
        route.output_contract.response_shape == crate::OutputResponseShape::Strict
            && route.output_contract.semantic_kind == crate::OutputSemanticKind::RawCommandOutput
    }) {
        return actions;
    }
    let Some(planned_content) = planned_content else {
        return actions;
    };
    if actions.len() < 3 {
        return actions;
    }
    let last_idx = actions.len() - 1;
    let synth_idx = last_idx - 1;
    let terminal_is_bare_last_output = matches!(
        &actions[last_idx],
        AgentAction::Respond { content } if is_bare_last_output_placeholder(content)
    );
    let synth_uses_last_output_only = matches!(
        &actions[synth_idx],
        AgentAction::SynthesizeAnswer { evidence_refs }
            if !evidence_refs.is_empty()
                && evidence_refs
                    .iter()
                    .all(|reference| reference.trim().eq_ignore_ascii_case("last_output"))
    );
    if !terminal_is_bare_last_output || !synth_uses_last_output_only {
        return actions;
    }

    let mut rewritten = Vec::with_capacity(actions.len() - 1);
    rewritten.extend(actions[..synth_idx].iter().cloned());
    rewritten.push(AgentAction::Respond {
        content: planned_content,
    });
    info!("plan_restore_terminal_mixed_last_output_respond");
    rewritten
}

/// §F1 结构 guard：判断 Respond.content 是否像「未观测就编造」的工具输出形态。
///
/// 这个 guard 只看输出形态，不判断用户意图：
/// - 含至少一行以数字+点+空格开头的枚举项（最少 1 行；`1. foo` / `2. bar`）
/// - 含 3+ 行（`\n` ≥ 2）且至少含一个 `/` 或常见文件后缀
/// - 含明显结构化字段标签（`result:` / `count:` / `size:` / `path:`，大小写不敏感）
///
/// 这些形态在 round 1 尚未执行观察步骤时不应出现在直接 Respond 里；
/// 语义路由仍由 normalizer/planner 负责，不能在这里增加自然语言词面规则。
pub(super) fn has_pre_observation_structured_output_shape(content: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.len() < 8 {
        return false;
    }
    // 1) 数字枚举项（`\d+\. xxx`），至少 1 行。
    for line in trimmed.lines() {
        let l = line.trim_start();
        let bytes = l.as_bytes();
        if bytes.is_empty() || !bytes[0].is_ascii_digit() {
            continue;
        }
        let mut idx = 1usize;
        while idx < bytes.len() && bytes[idx].is_ascii_digit() {
            idx += 1;
        }
        if idx + 1 < bytes.len()
            && bytes[idx] == b'.'
            && (bytes[idx + 1] == b' ' || bytes[idx + 1] == b'\t')
        {
            return true;
        }
    }
    // 2) 3+ 行 + 含路径分隔符或常见后缀。
    let line_count = trimmed.lines().count();
    if line_count >= 3 {
        let lower = trimmed.to_ascii_lowercase();
        let has_pathlike = lower.contains('/')
            || lower.contains(".md")
            || lower.contains(".toml")
            || lower.contains(".json")
            || lower.contains(".rs")
            || lower.contains(".log")
            || lower.contains(".sh")
            || lower.contains(".py");
        if has_pathlike {
            return true;
        }
    }
    // 3) 结构化字段标签。
    let lower = trimmed.to_ascii_lowercase();
    for label in ["result:", "count:", "size:", "path:", "files:", "items:"] {
        if lower.contains(label) {
            return true;
        }
    }
    false
}

/// 当 plan 末尾是 `respond.content="{{last_output}}"` 这种裸 placeholder 时，
/// runtime 主动在 `respond` 之前注入一个 `synthesize_answer` 节点，
/// 把原始观察输出（命令 stdout / 文件内容 / 列表 / JSON / 错误信息）转成
/// 自然语言再交给 respond。这样 respond 拿到的 `{{last_output}}` 已经是
/// synthesize 节点产出的自然语言，能通过 `delivery_text_classifier` 的 publishable 检查。
///
/// 设计动机：
/// * Runtime 这一道兜底把证据归纳交给 `SynthesizeAnswer`，避免 planner 生成
///   只有 `{{last_output}}` 的不可发布回复。
/// * 不破坏正确 plan：仅当末尾是裸 placeholder Respond 且其前一步不是
///   `synthesize_answer` 时才注入。
pub(super) fn inject_synthesize_answer_for_bare_placeholder_respond(
    actions: Vec<AgentAction>,
    _user_text: &str,
) -> Vec<AgentAction> {
    if actions.len() < 2 {
        // 只有一个 Respond 时，前面没有 observation 可供 synthesis 使用，不动。
        return actions;
    }
    let last_idx = actions.len() - 1;
    let needs_inject = match &actions[last_idx] {
        AgentAction::Respond { content } => is_bare_last_output_placeholder(content),
        _ => false,
    };
    if !needs_inject {
        return actions;
    }
    match &actions[last_idx - 1] {
        AgentAction::SynthesizeAnswer { .. } => {
            return actions;
        }
        _ => {}
    }
    let mut rewritten = actions;
    let evidence_refs = observation_action_evidence_refs(&rewritten[..last_idx]);
    let synth_step = AgentAction::SynthesizeAnswer {
        evidence_refs: if evidence_refs.is_empty() {
            vec!["last_output".to_string()]
        } else {
            evidence_refs
        },
    };
    let respond = rewritten.pop().expect("non-empty checked above");
    rewritten.push(synth_step);
    rewritten.push(respond);
    info!("plan_inject_synthesize_answer_for_bare_placeholder_respond");
    rewritten
}

pub(super) fn route_requires_terminal_observation_synthesis(
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::Scalar | crate::OutputResponseShape::FileToken
        )
    {
        return false;
    }
    if route.output_contract.response_shape == crate::OutputResponseShape::OneSentence
        || route.output_contract.exact_sentence_count.is_some()
    {
        return matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None
                | crate::OutputSemanticKind::RawCommandOutput
                | crate::OutputSemanticKind::CommandOutputSummary
                | crate::OutputSemanticKind::DirectoryPurposeSummary
                | crate::OutputSemanticKind::ContentExcerptSummary
                | crate::OutputSemanticKind::ContentExcerptWithSummary
                | crate::OutputSemanticKind::WorkspaceProjectSummary
        );
    }
    matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::CommandOutputSummary
            | crate::OutputSemanticKind::DirectoryPurposeSummary
            | crate::OutputSemanticKind::ContentExcerptSummary
            | crate::OutputSemanticKind::ContentExcerptWithSummary
            | crate::OutputSemanticKind::WorkspaceProjectSummary
    )
}

pub(super) fn append_terminal_synthesize_for_observation_summary_contract(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !route_requires_terminal_observation_synthesis(route_result) {
        return actions;
    }
    if actions.is_empty() {
        return actions;
    }
    if matches!(
        actions.last(),
        Some(AgentAction::Respond { .. } | AgentAction::SynthesizeAnswer { .. })
    ) {
        return actions;
    }
    if !matches!(
        actions.last(),
        Some(
            AgentAction::CallSkill { .. }
                | AgentAction::CallTool { .. }
                | AgentAction::CallCapability { .. }
        )
    ) {
        return actions;
    }
    let mut rewritten = actions;
    rewritten.push(AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["last_output".to_string()],
    });
    info!("plan_append_terminal_synthesize_for_observation_summary_contract");
    rewritten
}

pub(super) fn strip_intermediate_synthesize_before_later_execution(
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if actions.len() < 2 {
        return actions;
    }
    let mut stripped = Vec::with_capacity(actions.len());
    let mut changed = false;
    for (idx, action) in actions.iter().enumerate() {
        if matches!(action, AgentAction::SynthesizeAnswer { .. })
            && actions[idx + 1..].iter().any(|later| {
                matches!(
                    later,
                    AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
                )
            })
        {
            changed = true;
            continue;
        }
        stripped.push(action.clone());
    }
    if changed {
        info!("plan_strip_intermediate_synthesize_before_later_execution");
    }
    stripped
}

pub(super) fn strip_terminal_placeholder_respond_for_exact_listing_contract(
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !matches!(
        route.output_contract.semantic_kind,
        crate::OutputSemanticKind::FileNames
            | crate::OutputSemanticKind::DirectoryNames
            | crate::OutputSemanticKind::FilePaths
    ) {
        return actions;
    }
    if !actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { .. } | AgentAction::CallTool { .. }
        )
    }) {
        return actions;
    }
    let Some(AgentAction::Respond { content }) = actions.last() else {
        return actions;
    };
    if !is_bare_last_output_placeholder(content) {
        return actions;
    }
    let mut stripped = actions;
    stripped.pop();
    info!("plan_strip_terminal_placeholder_respond_for_exact_listing_contract");
    stripped
}
