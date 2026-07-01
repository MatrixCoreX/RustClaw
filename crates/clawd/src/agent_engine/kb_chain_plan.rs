use super::*;

pub(super) fn kb_chain_deterministic_plan_result(
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
        || !route.is_execute_gate()
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !route.output_contract_marker_is(crate::OutputSemanticKind::CommandOutputSummary)
        || !kb_enabled_for_planning(state)
        || !route_declares_kb_chain(route)
    {
        return None;
    }

    let namespace = kb_namespace(route, current_user_text)?;
    let source_path = kb_source_path(route, current_user_text, auto_locator_path)?;
    let query = kb_search_query(route)?;

    let mut actions = vec![
        AgentAction::CallSkill {
            skill: "kb".to_string(),
            args: serde_json::json!({
                "action": "ingest",
                "namespace": namespace,
                "paths": [source_path],
                "overwrite": true,
            }),
        },
        AgentAction::CallSkill {
            skill: "kb".to_string(),
            args: serde_json::json!({
                "action": "search",
                "namespace": namespace,
                "query": query,
                "top_k": 5,
            }),
        },
        AgentAction::CallSkill {
            skill: "kb".to_string(),
            args: serde_json::json!({
                "action": "stats",
                "namespace": namespace,
            }),
        },
    ];

    if !actions
        .iter()
        .all(|action| kb_action_allowed(route, action))
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

fn kb_enabled_for_planning(state: &AppState) -> bool {
    let enabled_skills = state.get_skills_list();
    enabled_skills.is_empty() || enabled_skills.contains("kb")
}

fn route_declares_kb_chain(route: &RouteResult) -> bool {
    let machine_text = format!("{}\n{}", route.route_reason, route.resolved_intent);
    ["kb.ingest", "kb.search", "kb.stats"]
        .iter()
        .all(|token| machine_text.contains(token))
}

fn kb_namespace(route: &RouteResult, current_user_text: &str) -> Option<String> {
    for text in [
        current_user_text,
        route.output_contract.locator_hint.as_str(),
        route.route_reason.as_str(),
        route.resolved_intent.as_str(),
    ] {
        if let Some(namespace) = namespace_from_machine_token(text) {
            return Some(namespace);
        }
    }
    None
}

fn namespace_from_machine_token(text: &str) -> Option<String> {
    static NAMESPACE_RE: OnceLock<Regex> = OnceLock::new();
    let re = NAMESPACE_RE.get_or_init(|| {
        Regex::new(r"(?i)\bnamespace\s*=\s*([A-Za-z0-9_.-]+)\b")
            .expect("valid namespace token regex")
    });
    re.captures(text)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().trim().to_string())
        .filter(|namespace| !namespace.is_empty())
}

fn kb_source_path(
    route: &RouteResult,
    current_user_text: &str,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    auto_locator_path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(ToString::to_string)
        .or_else(|| {
            [
                route.output_contract.locator_hint.as_str(),
                current_user_text,
                route.route_reason.as_str(),
                route.resolved_intent.as_str(),
            ]
            .into_iter()
            .find_map(first_explicit_path_locator)
        })
}

fn first_explicit_path_locator(text: &str) -> Option<String> {
    crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(text)
        .into_iter()
        .map(|candidate| candidate.locator_hint.trim().to_string())
        .find(|candidate| looks_like_path_locator(candidate))
}

fn looks_like_path_locator(candidate: &str) -> bool {
    !candidate.is_empty()
        && !candidate.contains('\n')
        && !candidate.contains('\r')
        && (candidate.contains('/') || candidate.starts_with("./") || candidate.starts_with("../"))
}

fn kb_search_query(route: &RouteResult) -> Option<String> {
    [route.route_reason.as_str(), route.resolved_intent.as_str()]
        .into_iter()
        .find_map(search_query_from_machine_text)
}

fn search_query_from_machine_text(text: &str) -> Option<String> {
    static QUOTED_SEARCH_RE: OnceLock<Regex> = OnceLock::new();
    let quoted = QUOTED_SEARCH_RE.get_or_init(|| {
        Regex::new(r#"(?i)\b(?:kb\.search|search|query)\s*[:=]?\s*['"]([^'"]+)['"]"#)
            .expect("valid kb search query regex")
    });
    if let Some(query) = quoted
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().trim().to_string())
        .filter(|query| !query.is_empty())
    {
        return Some(query);
    }

    static TOKEN_SEARCH_RE: OnceLock<Regex> = OnceLock::new();
    let token = TOKEN_SEARCH_RE.get_or_init(|| {
        Regex::new(r#"(?i)\bquery\s*=\s*([A-Za-z0-9_.:/-]+(?:\s+[A-Za-z0-9_.:/-]+){0,4})"#)
            .expect("valid kb query token regex")
    });
    token
        .captures(text)
        .and_then(|captures| captures.get(1))
        .map(|capture| capture.as_str().trim().to_string())
        .filter(|query| !query.is_empty())
}

fn kb_action_allowed(route: &RouteResult, action: &AgentAction) -> bool {
    let AgentAction::CallSkill { skill, args } = action else {
        return true;
    };
    crate::contract_matrix::action_policy_for_route(Some(route), skill, args)
        .is_some_and(|policy| policy.is_allowed() && policy.action_matches_preferred())
}
