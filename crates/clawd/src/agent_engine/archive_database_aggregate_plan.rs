use super::*;

pub(super) fn archive_database_aggregate_deterministic_plan_result(
    state: &AppState,
    goal: &str,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    current_user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<PlanResult> {
    let route = route_result?;
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route_requests_archive_database_aggregate_capabilities(route)
        || !archive_basic_enabled_for_planning(state)
        || !db_basic_enabled_for_planning(state)
    {
        return None;
    }

    let archive = aggregate_archive_path(route, current_user_text, auto_locator_path)?;
    let member = aggregate_archive_member(route, current_user_text, &archive)?;
    let db_path = aggregate_sqlite_path(route, current_user_text, auto_locator_path)?;
    let mut actions = vec![
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: serde_json::json!({
                "action": "list",
                "archive": archive,
            }),
        },
        AgentAction::CallSkill {
            skill: "archive_basic".to_string(),
            args: serde_json::json!({
                "action": "read",
                "archive": archive,
                "member": member,
            }),
        },
        AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "list_tables",
                "db_path": db_path,
            }),
        },
    ];
    if route_requests_schema_version_machine_token(route) {
        actions.push(AgentAction::CallSkill {
            skill: "db_basic".to_string(),
            args: serde_json::json!({
                "action": "schema_version",
                "db_path": db_path,
            }),
        });
    }
    if !actions
        .iter()
        .all(|action| aggregate_action_allowed(route, action))
    {
        return None;
    }

    let evidence_refs = observation_action_evidence_refs(&actions);
    actions.push(AgentAction::SynthesizeAnswer { evidence_refs });
    actions.push(AgentAction::Respond {
        content: "{{last_output}}".to_string(),
    });
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

fn db_basic_enabled_for_planning(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("db_basic")
}

fn route_requests_archive_database_aggregate_capabilities(route: &RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action_name(route, &["archive"], &["list"])
        && crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["archive"],
            &["read"],
        )
        && crate::machine_capability_ref::route_has_capability_action_name(
            route,
            &["database"],
            &["list_tables"],
        )
}

fn aggregate_archive_path(
    route: &RouteResult,
    current_user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let mut candidates = Vec::new();
    for text in [
        auto_locator_path,
        Some(route.output_contract.locator_hint.as_str()),
        Some(current_user_text),
        Some(route.resolved_intent.as_str()),
        Some(route.route_reason.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        collect_archive_locator_candidates(&mut candidates, text);
    }
    candidates
        .into_iter()
        .find(|candidate| is_supported_archive_path(candidate))
}

fn aggregate_archive_member(
    route: &RouteResult,
    current_user_text: &str,
    archive_path: &str,
) -> Option<String> {
    let mut candidates = Vec::new();
    for text in [
        current_user_text,
        route.resolved_intent.as_str(),
        route.route_reason.as_str(),
    ] {
        collect_archive_member_candidates(&mut candidates, text, archive_path);
    }
    candidates.into_iter().next()
}

fn collect_archive_member_candidates(out: &mut Vec<String>, text: &str, archive_path: &str) {
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
    {
        push_aggregate_archive_member_candidate(out, &locator.locator_hint, archive_path);
    }
    for filename in crate::delivery_utils::extract_filename_candidates(text) {
        push_aggregate_archive_member_candidate(out, &filename, archive_path);
    }
}

fn push_aggregate_archive_member_candidate(
    out: &mut Vec<String>,
    candidate: &str,
    archive_path: &str,
) {
    let Some(candidate) = normalize_archive_entry_target_candidate(candidate, archive_path) else {
        return;
    };
    if is_sqlite_database_path(&candidate) || is_supported_archive_path(&candidate) {
        return;
    }
    if !archive_member_path_is_safe(&candidate) {
        return;
    }
    if !out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&candidate))
    {
        out.push(candidate);
    }
}

fn aggregate_sqlite_path(
    route: &RouteResult,
    current_user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let mut candidates = Vec::new();
    for text in [
        auto_locator_path,
        Some(route.output_contract.locator_hint.as_str()),
        Some(current_user_text),
        Some(route.resolved_intent.as_str()),
        Some(route.route_reason.as_str()),
    ]
    .into_iter()
    .flatten()
    {
        collect_sqlite_path_candidates(&mut candidates, text);
    }
    candidates
        .into_iter()
        .find(|candidate| is_sqlite_database_path(candidate))
}

fn collect_sqlite_path_candidates(out: &mut Vec<String>, text: &str) {
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    push_sqlite_path_candidates_from_locator_text(out, text);
    for locator in
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
    {
        push_sqlite_path_candidates_from_locator_text(out, &locator.locator_hint);
    }
    for filename in crate::delivery_utils::extract_filename_candidates(text) {
        push_sqlite_path_candidate(out, &filename);
    }
}

fn push_sqlite_path_candidates_from_locator_text(out: &mut Vec<String>, text: &str) {
    let parts: Vec<_> = text
        .split(|ch| matches!(ch, '|' | '\n' | '\r'))
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect();
    if parts.len() > 1 {
        for part in parts {
            push_sqlite_path_candidate(out, part);
        }
        return;
    }
    push_sqlite_path_candidate(out, text);
}

fn push_sqlite_path_candidate(out: &mut Vec<String>, candidate: &str) {
    let candidate = candidate.trim().trim_matches(|ch: char| {
        matches!(
            ch,
            '"' | '\'' | '`' | ',' | '，' | '。' | ';' | '；' | '(' | ')' | '（' | '）'
        )
    });
    if candidate.contains('|') || candidate.contains('\n') || candidate.contains('\r') {
        return;
    }
    if !is_sqlite_database_path(candidate) {
        return;
    }
    if !out
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(candidate))
    {
        out.push(candidate.to_string());
    }
}

fn route_requests_schema_version_machine_token(route: &RouteResult) -> bool {
    crate::machine_capability_ref::route_has_capability_action_name(
        route,
        &["database"],
        &["schema_version"],
    )
}

fn aggregate_action_allowed(route: &RouteResult, action: &AgentAction) -> bool {
    let (AgentAction::CallSkill { skill, args } | AgentAction::CallTool { tool: skill, args }) =
        action
    else {
        return true;
    };
    crate::evidence_policy::capability_ref_action_policy_for_route(Some(route), skill, args)
        .is_some_and(|policy| policy.is_allowed() && policy.action_matches_preferred())
}
