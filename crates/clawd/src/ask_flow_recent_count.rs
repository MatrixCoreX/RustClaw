use super::*;

pub(super) fn route_is_recent_count_comparison(
    current_user_request: &str,
    route: &crate::RouteResult,
    direction: RecentCountComparisonDirection,
) -> Option<RecentCountComparisonDirection> {
    if route.needs_clarify
        || route.wants_file_delivery
        || route.output_contract.delivery_required
        || matches!(
            route.output_contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
    {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    if surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference()
    {
        return None;
    }
    Some(direction)
}

pub(super) fn target_label_from_count_inventory_output(value: &Value) -> Option<String> {
    let raw = value
        .get("path")
        .and_then(Value::as_str)
        .or_else(|| value.get("resolved_path").and_then(Value::as_str))?
        .trim();
    if raw.is_empty() || raw == "." {
        return None;
    }
    let trimmed = raw.trim_end_matches(['/', '\\']);
    let label = Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed);
    Some(label.to_string())
}

pub(super) fn count_observation_from_output_excerpt(
    output_excerpt: &str,
) -> Option<RecentCountObservation> {
    let value: Value = serde_json::from_str(output_excerpt.trim()).ok()?;
    count_observation_from_count_inventory_value(&value)
        .or_else(|| {
            value
                .get("extra")
                .and_then(count_observation_from_count_inventory_value)
        })
        .or_else(|| {
            value
                .get("text")
                .and_then(Value::as_str)
                .and_then(|text| serde_json::from_str::<Value>(text.trim()).ok())
                .and_then(|nested| count_observation_from_count_inventory_value(&nested))
        })
}

pub(super) fn count_observation_from_count_inventory_value(
    value: &Value,
) -> Option<RecentCountObservation> {
    if value.get("action").and_then(Value::as_str) != Some("count_inventory") {
        return None;
    }
    let total = value
        .get("counts")
        .and_then(|counts| counts.get("total"))
        .and_then(Value::as_i64)?;
    let target_label = target_label_from_count_inventory_output(&value)?;
    Some(RecentCountObservation {
        target_label,
        total,
    })
}

pub(super) fn count_observation_from_task_result_json(
    result_json: &str,
) -> Option<RecentCountObservation> {
    let value: Value = serde_json::from_str(result_json).ok()?;
    let steps = value
        .pointer("/task_journal/trace/step_results")
        .and_then(Value::as_array)?;
    steps.iter().rev().find_map(|step| {
        step.get("output_excerpt")
            .and_then(Value::as_str)
            .and_then(count_observation_from_output_excerpt)
    })
}

pub(super) fn recent_count_observations_from_completed_tasks(
    state: &AppState,
    task: &ClaimedTask,
    limit: usize,
) -> Vec<RecentCountObservation> {
    let Ok(db) = state.core.db.get() else {
        return Vec::new();
    };
    let user_key = task
        .user_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("anon:{}:{}", task.user_id, task.chat_id));
    let Ok(mut stmt) = db.prepare(
        "SELECT result_json
         FROM tasks
         WHERE user_id = ?1
           AND chat_id = ?2
           AND COALESCE(user_key, '') = ?3
           AND kind = 'ask'
           AND status = 'succeeded'
           AND task_id != ?4
           AND result_json IS NOT NULL
         ORDER BY updated_at DESC
         LIMIT ?5",
    ) else {
        return Vec::new();
    };
    let Ok(rows) = stmt.query_map(
        rusqlite::params![
            task.user_id,
            task.chat_id,
            user_key,
            task.task_id,
            limit as i64
        ],
        |row| row.get::<_, String>(0),
    ) else {
        return Vec::new();
    };
    rows.filter_map(Result::ok)
        .filter_map(|result_json| count_observation_from_task_result_json(&result_json))
        .collect()
}

pub(super) fn recent_count_comparison_direct_answer(
    state: &AppState,
    task: &ClaimedTask,
    current_user_request: &str,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> Option<String> {
    let ctx = agent_run_context?;
    let route = ctx.route_result.as_ref()?;
    let direction = recent_count_selection_from_turn_analysis(ctx.turn_analysis.as_ref())?;
    let observations = recent_count_observations_from_completed_tasks(state, task, 8);
    let latest = observations.first()?;
    let previous = observations.get(1)?;
    let direction = route_is_recent_count_comparison(current_user_request, route, direction)?;
    recent_count_comparison_winner_label(latest, previous, direction)
}

pub(super) fn direct_answer_gate_can_skip_for_recent_count_context(
    state: &AppState,
    task: &ClaimedTask,
    agent_run_context: Option<&crate::agent_engine::AgentRunContext>,
) -> bool {
    let Some(ctx) = agent_run_context else {
        return false;
    };
    if recent_count_selection_from_turn_analysis(ctx.turn_analysis.as_ref()).is_none() {
        return false;
    }
    let current_user_request = task_payload_text(task).unwrap_or_default();
    recent_count_comparison_direct_answer(state, task, &current_user_request, Some(ctx)).is_some()
}

pub(super) fn recent_count_selection_from_turn_analysis(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<RecentCountComparisonDirection> {
    let quantity_comparison = turn_analysis?
        .state_patch
        .as_ref()?
        .get("quantity_comparison")?;
    if quantity_comparison.get("source").and_then(Value::as_str) != Some("recent_count_inventory") {
        return None;
    }
    let selection = quantity_comparison.get("selection")?.as_str()?;
    match selection {
        "max" => Some(RecentCountComparisonDirection::More),
        "min" => Some(RecentCountComparisonDirection::Less),
        _ => None,
    }
}

pub(super) fn direct_answer_gate_state_patch_is_meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Object(map) => map
            .values()
            .any(direct_answer_gate_state_patch_is_meaningful),
        Value::Array(items) => items
            .iter()
            .any(direct_answer_gate_state_patch_is_meaningful),
        Value::String(text) => !text.trim().is_empty(),
        _ => true,
    }
}

pub(super) fn merge_direct_answer_gate_state_patch(
    ctx: &mut crate::agent_engine::AgentRunContext,
    state_patch: Option<&Value>,
) {
    let Some(state_patch) = state_patch
        .filter(|value| direct_answer_gate_state_patch_is_meaningful(value))
        .cloned()
    else {
        return;
    };
    if let Some(analysis) = ctx.turn_analysis.as_mut() {
        analysis.state_patch = match analysis.state_patch.take() {
            Some(Value::Object(mut existing)) => match state_patch {
                Value::Object(incoming) => {
                    for (key, value) in incoming {
                        existing.insert(key, value);
                    }
                    Some(Value::Object(existing))
                }
                other => Some(other),
            },
            Some(_) | None => Some(state_patch),
        };
        return;
    }
    ctx.turn_analysis = Some(crate::intent_router::TurnAnalysis {
        turn_type: Some(crate::intent_router::TurnType::TaskRequest),
        target_task_policy: Some(crate::intent_router::TargetTaskPolicy::Standalone),
        should_interrupt_active_run: false,
        state_patch: Some(state_patch),
        attachment_processing_required: false,
    });
}

pub(super) fn direct_answer_gate_has_recent_count_selection(gate: &DirectAnswerGateOut) -> bool {
    let Some(quantity_comparison) = gate
        .state_patch
        .as_ref()
        .and_then(|patch| patch.get("quantity_comparison"))
    else {
        return false;
    };
    quantity_comparison.get("source").and_then(Value::as_str) == Some("recent_count_inventory")
        && matches!(
            quantity_comparison.get("selection").and_then(Value::as_str),
            Some("max" | "min")
        )
}

pub(super) fn direct_answer_gate_recent_count_contract_can_stay_direct(
    current_user_request: &str,
    contract: &crate::IntentOutputContract,
) -> bool {
    if contract.delivery_required
        || matches!(
            contract.response_shape,
            crate::OutputResponseShape::FileToken
        )
        || contract.semantic_kind != crate::OutputSemanticKind::QuantityComparison
    {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(current_user_request);
    !(surface.has_explicit_path_or_url()
        || surface.has_filename_candidates()
        || surface.locator_target_pair.is_some()
        || surface.has_delivery_token_reference())
}

pub(super) fn recent_count_comparison_winner_label(
    latest: &RecentCountObservation,
    previous: &RecentCountObservation,
    direction: RecentCountComparisonDirection,
) -> Option<String> {
    let winner = match direction {
        RecentCountComparisonDirection::More => match latest.total.cmp(&previous.total) {
            std::cmp::Ordering::Greater => latest,
            std::cmp::Ordering::Less => previous,
            std::cmp::Ordering::Equal => return None,
        },
        RecentCountComparisonDirection::Less => match latest.total.cmp(&previous.total) {
            std::cmp::Ordering::Less => latest,
            std::cmp::Ordering::Greater => previous,
            std::cmp::Ordering::Equal => return None,
        },
    };
    Some(winner.target_label.clone())
}
