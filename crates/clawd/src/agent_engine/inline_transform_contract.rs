use super::*;

pub(super) fn route_has_inline_transform_contract(route_result: Option<&RouteResult>) -> bool {
    route_result.is_some_and(|route| {
        route
            .route_reason
            .contains("inline_json_transform_structured_execute")
            || route
                .route_reason
                .contains("parsed_inline_json_transform_contract_repair")
            || route
                .route_reason
                .contains("normalizer_unavailable_inline_json_transform")
            || route
                .route_reason
                .contains("inline_structured_transform_contract_repair")
            || route
                .route_reason
                .contains("direct_answer_gate_inline_transform_execute")
            || route
                .route_reason
                .contains("inline_structured_payload_context_execute")
    })
}

pub(super) fn transformable_input_value(value: &Value) -> bool {
    match value {
        Value::Array(items) => !items.is_empty() && items.iter().any(Value::is_object),
        Value::Object(obj) => {
            if obj
                .get("data")
                .or_else(|| obj.get("records"))
                .or_else(|| obj.get("input"))
                .is_some_and(|item| {
                    item.as_array().is_some_and(|items| {
                        !items.is_empty() && items.iter().any(Value::is_object)
                    }) || item.is_object()
                })
            {
                return true;
            }
            !obj.is_empty()
                && !obj.contains_key("action")
                && !obj.contains_key("skill")
                && !obj.contains_key("operation")
        }
        _ => false,
    }
}

pub(super) fn last_transformable_input_value(text: &str) -> Option<Value> {
    last_transformable_input_value_with_raw(text).map(|(_, value)| value)
}

pub(super) fn last_transformable_input_value_with_raw(text: &str) -> Option<(String, Value)> {
    json_values_any_raw(text)
        .into_iter()
        .rev()
        .find(|(_, value)| transformable_input_value(value))
}

pub(super) fn remove_last_json_payload_from_text<'a>(text: &'a str, raw: &str) -> Option<String> {
    let start = text.rfind(raw)?;
    let end = start.saturating_add(raw.len());
    let mut remaining = String::with_capacity(text.len().saturating_sub(raw.len()) + 1);
    remaining.push_str(&text[..start]);
    remaining.push(' ');
    remaining.push_str(&text[end..]);
    Some(remaining)
}

pub(super) fn schema_field_token(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '-' || ch.is_ascii_alphanumeric())
}

pub(super) fn schema_shaped_target_token(candidate: &str, source: &str) -> bool {
    schema_field_token(candidate)
        && candidate != source
        && !candidate.chars().all(|ch| ch.is_ascii_uppercase())
        && (candidate.contains('_')
            || candidate.contains('-')
            || candidate.chars().any(|ch| ch.is_ascii_digit())
            || source.contains('_')
            || source.contains('-')
            || source.chars().any(|ch| ch.is_ascii_digit()))
}

pub(super) fn schema_tokens_in_text(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch == '_' || ch == '-' || ch.is_ascii_alphanumeric() {
            current.push(ch);
            continue;
        }
        if schema_field_token(&current) {
            tokens.push(std::mem::take(&mut current));
        } else {
            current.clear();
        }
    }
    if schema_field_token(&current) {
        tokens.push(current);
    }
    tokens
}

pub(super) fn derive_single_object_rename_args_from_text(text: &str) -> Option<Value> {
    let (raw, input_value) = last_transformable_input_value_with_raw(text)?;
    let input_obj = input_value.as_object()?;
    if input_obj.is_empty() {
        return None;
    }
    let instruction = remove_last_json_payload_from_text(text, &raw)?;
    let tokens = schema_tokens_in_text(&instruction);
    let input_keys = input_obj.keys().map(String::as_str).collect::<Vec<_>>();
    let mut source_positions = tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| input_keys.iter().any(|key| key == &token.as_str()))
        .collect::<Vec<_>>();
    source_positions.dedup_by(|(_, left), (_, right)| left == right);
    if source_positions.len() != 1 {
        return None;
    }
    let (source_index, source_token) = source_positions[0];
    let target_candidates = tokens
        .iter()
        .skip(source_index + 1)
        .filter(|token| !input_keys.iter().any(|key| key == &token.as_str()))
        .filter(|token| schema_shaped_target_token(token, source_token))
        .fold(Vec::<&String>::new(), |mut acc, token| {
            if !acc
                .iter()
                .any(|existing| existing.as_str() == token.as_str())
            {
                acc.push(token);
            }
            acc
        });
    if target_candidates.len() != 1 {
        return None;
    }
    Some(normalize_transform_args(serde_json::json!({
        "action": "transform_data",
        "data": input_value,
        "ops": [{
            "op": "rename",
            "from": source_token,
            "to": target_candidates[0]
        }],
        "result_shape": "single_object",
        "output_format": "json"
    })))
}

pub(super) fn answer_candidate_from_route(route_result: Option<&RouteResult>) -> Option<&str> {
    let resolved = route_result?.resolved_intent.as_str();
    let (_, candidate) = resolved.rsplit_once("\nanswer_candidate:")?;
    let candidate = crate::visible_text::strip_internal_context_sections(candidate).trim();
    Some(candidate).filter(|candidate| !candidate.is_empty())
}

pub(super) fn parse_answer_candidate_value(candidate: &str) -> Option<Value> {
    let trimmed = candidate.trim();
    serde_json::from_str::<Value>(trimmed)
        .ok()
        .or_else(|| {
            crate::extract_first_json_value_any(trimmed)
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        })
        .or_else(|| {
            trimmed
                .parse::<i64>()
                .ok()
                .map(|value| Value::Number(value.into()))
        })
        .or_else(|| {
            trimmed
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
        })
}

pub(super) fn json_sort_key(value: &Value) -> String {
    serde_json::to_string(value).unwrap_or_default()
}

pub(super) fn json_rows_equal(left: &[Value], right: &[Value]) -> bool {
    left.len() == right.len() && left.iter().zip(right).all(|(a, b)| a == b)
}

pub(super) fn json_multiset_equal(left: &[Value], right: &[Value]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut left_keys = left.iter().map(json_sort_key).collect::<Vec<_>>();
    let mut right_keys = right.iter().map(json_sort_key).collect::<Vec<_>>();
    left_keys.sort();
    right_keys.sort();
    left_keys == right_keys
}

pub(super) fn object_keys(value: &Value) -> Vec<String> {
    value
        .as_object()
        .map(|obj| obj.keys().cloned().collect())
        .unwrap_or_default()
}

pub(super) fn common_object_keys(rows: &[Value]) -> Vec<String> {
    let Some(first) = rows.first() else {
        return Vec::new();
    };
    object_keys(first)
        .into_iter()
        .filter(|key| {
            rows.iter()
                .all(|row| row.as_object().is_some_and(|obj| obj.contains_key(key)))
        })
        .collect()
}

pub(super) fn value_for_key<'a>(row: &'a Value, key: &str) -> &'a Value {
    row.as_object()
        .and_then(|obj| obj.get(key))
        .unwrap_or(&Value::Null)
}

pub(super) fn transform_cmp_values(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a
            .as_f64()
            .partial_cmp(&b.as_f64())
            .unwrap_or(std::cmp::Ordering::Equal),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        _ => json_sort_key(a).cmp(&json_sort_key(b)),
    }
}

pub(super) fn derive_sort_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if !json_multiset_equal(input, output) || json_rows_equal(input, output) {
        return None;
    }
    for key in common_object_keys(input) {
        let mut asc = input.to_vec();
        asc.sort_by(|a, b| transform_cmp_values(value_for_key(a, &key), value_for_key(b, &key)));
        if json_rows_equal(&asc, output) {
            return Some(serde_json::json!({"op": "sort", "by": key, "order": "asc"}));
        }
        let mut desc = asc;
        desc.reverse();
        if json_rows_equal(&desc, output) {
            return Some(serde_json::json!({"op": "sort", "by": key, "order": "desc"}));
        }
    }
    None
}

pub(super) fn numeric_value(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

pub(super) fn numbers_equal(left: f64, right: f64) -> bool {
    (left - right).abs() < 1e-9
}

pub(super) fn derive_group_sum_op_from_candidate(
    input: &[Value],
    output: &[Value],
) -> Option<Value> {
    if input.is_empty() || output.is_empty() {
        return None;
    }
    let input_keys = common_object_keys(input);
    let output_keys = common_object_keys(output);
    for group_key in input_keys.iter().filter(|key| output_keys.contains(key)) {
        for input_value_key in input_keys.iter() {
            if input_value_key == group_key {
                continue;
            }
            if !input
                .iter()
                .any(|row| numeric_value(value_for_key(row, input_value_key)).is_some())
            {
                continue;
            }
            for output_value_key in output_keys.iter().filter(|key| *key != group_key) {
                let mut sums: HashMap<String, f64> = HashMap::new();
                for row in input {
                    let group_value = value_for_key(row, group_key);
                    let key = json_sort_key(group_value);
                    let value = numeric_value(value_for_key(row, input_value_key))?;
                    *sums.entry(key).or_insert(0.0) += value;
                }
                if sums.len() != output.len() {
                    continue;
                }
                let mut matched = true;
                for row in output {
                    let group = json_sort_key(value_for_key(row, group_key));
                    let Some(expected) = sums.get(&group) else {
                        matched = false;
                        break;
                    };
                    let Some(actual) = numeric_value(value_for_key(row, output_value_key)) else {
                        matched = false;
                        break;
                    };
                    if !numbers_equal(*expected, actual) {
                        matched = false;
                        break;
                    }
                }
                if matched {
                    return Some(serde_json::json!({
                        "op": "group",
                        "by": [group_key],
                        "aggregations": [{
                            "op": "sum",
                            "field": input_value_key,
                            "name": output_value_key
                        }]
                    }));
                }
            }
        }
    }
    None
}

pub(super) fn derive_project_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if input.len() != output.len() || input.is_empty() {
        return None;
    }
    let output_keys = common_object_keys(output);
    if output_keys.is_empty() {
        return None;
    }
    let input_keys = common_object_keys(input);
    if !output_keys.iter().all(|key| input_keys.contains(key)) {
        return None;
    }
    let projected = input
        .iter()
        .map(|row| {
            let mut obj = serde_json::Map::new();
            for key in &output_keys {
                obj.insert(key.clone(), value_for_key(row, key).clone());
            }
            Value::Object(obj)
        })
        .collect::<Vec<_>>();
    (projected == output).then(|| serde_json::json!({"op": "project", "fields": output_keys}))
}

pub(super) fn derive_filter_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if output.is_empty() || output.len() >= input.len() {
        return None;
    }
    for key in common_object_keys(input) {
        let mut seen_values = std::collections::HashSet::new();
        let candidate_values = output
            .iter()
            .filter_map(|row| {
                let value = value_for_key(row, &key).clone();
                seen_values.insert(json_sort_key(&value)).then_some(value)
            })
            .collect::<Vec<_>>();
        for value in candidate_values {
            let filtered = input
                .iter()
                .filter(|row| value_for_key(row, &key) == &value)
                .cloned()
                .collect::<Vec<_>>();
            if filtered == output {
                return Some(serde_json::json!({
                    "op": "filter",
                    "field": key,
                    "cmp": "eq",
                    "value": value
                }));
            }
        }
    }
    None
}

pub(super) fn derive_dedup_op_from_candidate(input: &[Value], output: &[Value]) -> Option<Value> {
    if output.is_empty() || output.len() >= input.len() {
        return None;
    }
    for key in common_object_keys(input) {
        let mut seen = std::collections::HashSet::new();
        let deduped = input
            .iter()
            .filter(|row| seen.insert(json_sort_key(value_for_key(row, &key))))
            .cloned()
            .collect::<Vec<_>>();
        if deduped == output {
            return Some(serde_json::json!({"op": "dedup", "field": key}));
        }
    }
    None
}

pub(super) fn derive_aggregate_scalar_op_from_candidate(
    input: &[Value],
    output: &Value,
) -> Option<Value> {
    let target = numeric_value(output)?;
    for key in common_object_keys(input) {
        let values = input
            .iter()
            .map(|row| numeric_value(value_for_key(row, &key)))
            .collect::<Option<Vec<_>>>()?;
        let sum = values.iter().sum::<f64>();
        if numbers_equal(sum, target) {
            return Some(serde_json::json!({
                "op": "aggregate",
                "aggregations": [{"op": "sum", "field": key, "name": "value"}]
            }));
        }
    }
    None
}

pub(super) fn unique_common_numeric_key(rows: &[Value]) -> Option<String> {
    let numeric_keys = common_object_keys(rows)
        .into_iter()
        .filter(|key| {
            rows.iter()
                .all(|row| numeric_value(value_for_key(row, key)).is_some())
        })
        .collect::<Vec<_>>();
    (numeric_keys.len() == 1).then(|| numeric_keys[0].clone())
}

pub(super) fn contextual_inline_structured_transform_args_from_payload(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    if !route_has_inline_transform_contract(route_result) {
        return None;
    }
    let input_value = last_transformable_input_value(text)?;
    let rows = input_value.as_array()?;
    if rows.is_empty() || !rows.iter().all(Value::is_object) {
        return None;
    }
    let sort_key = unique_common_numeric_key(rows)?;
    Some(normalize_transform_args(serde_json::json!({
        "action": "transform_data",
        "data": input_value,
        "ops": [{
            "op": "sort",
            "by": sort_key,
            "order": "desc"
        }],
        "output_format": "md_table"
    })))
}

pub(super) fn inline_json_scalar_count_args_from_contract(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    let route = route_result?;
    if route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount {
        return None;
    }
    let input_value = last_transformable_input_value(text)?;
    if !matches!(input_value, Value::Array(_)) {
        return None;
    }
    Some(normalize_transform_args(serde_json::json!({
        "action": "transform_data",
        "data": input_value,
        "ops": [{
            "op": "aggregate",
            "aggregations": [{"op": "count", "name": "count"}]
        }],
        "result_shape": "scalar",
        "output_format": "json"
    })))
}

pub(super) fn derive_rename_op_from_candidate(input: &Value, output: &Value) -> Option<Value> {
    let input_obj = input.as_object()?;
    let output_obj = output.as_object()?;
    let removed = input_obj
        .iter()
        .filter(|(key, _)| !output_obj.contains_key(*key))
        .collect::<Vec<_>>();
    let added = output_obj
        .iter()
        .filter(|(key, _)| !input_obj.contains_key(*key))
        .collect::<Vec<_>>();
    if removed.len() != 1 || added.len() != 1 {
        return None;
    }
    let (from, removed_value) = removed[0];
    let (to, added_value) = added[0];
    if removed_value != added_value {
        return None;
    }
    Some(serde_json::json!({"op": "rename", "from": from, "to": to}))
}

pub(super) fn inline_json_transform_args_from_candidate(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    if !crate::intent::surface_signals::inline_json_transform_request(text)
        && !route_has_inline_transform_contract(route_result)
    {
        return None;
    }
    let input_value = last_transformable_input_value(text)?;
    let candidate_value =
        answer_candidate_from_route(route_result).and_then(parse_answer_candidate_value)?;
    match (&input_value, &candidate_value) {
        (Value::Array(input), Value::Array(output)) => {
            let op = derive_sort_op_from_candidate(input, output)
                .or_else(|| derive_group_sum_op_from_candidate(input, output))
                .or_else(|| derive_project_op_from_candidate(input, output))
                .or_else(|| derive_filter_op_from_candidate(input, output))
                .or_else(|| derive_dedup_op_from_candidate(input, output))?;
            Some(normalize_transform_args(serde_json::json!({
                "action": "transform_data",
                "data": input_value,
                "ops": [op],
                "output_format": "json"
            })))
        }
        (Value::Array(input), scalar) => {
            let op = derive_aggregate_scalar_op_from_candidate(input, scalar)?;
            Some(normalize_transform_args(serde_json::json!({
                "action": "transform_data",
                "data": input_value,
                "ops": [op],
                "result_shape": "scalar",
                "output_format": "json"
            })))
        }
        (Value::Object(_), Value::Object(_)) => {
            let op = derive_rename_op_from_candidate(&input_value, &candidate_value)?;
            Some(normalize_transform_args(serde_json::json!({
                "action": "transform_data",
                "data": input_value,
                "ops": [op],
                "result_shape": "single_object",
                "output_format": "json"
            })))
        }
        _ => None,
    }
}

pub(super) fn answer_candidate_is_markdown_table(route_result: Option<&RouteResult>) -> bool {
    answer_candidate_from_route(route_result).is_some_and(|candidate| {
        let lines = candidate
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        lines.len() >= 2
            && lines
                .first()
                .is_some_and(|line| line.starts_with('|') && line.ends_with('|'))
            && lines
                .get(1)
                .is_some_and(|line| line.chars().all(|ch| matches!(ch, '|' | '-' | ':' | ' ')))
    })
}

pub(super) fn inline_csv_transform_args_from_text(
    text: &str,
    route_result: Option<&RouteResult>,
) -> Option<Value> {
    if !crate::intent::surface_signals::inline_json_transform_request(text)
        || crate::extract_first_json_value_any(text).is_some()
        || !answer_candidate_is_markdown_table(route_result)
    {
        return None;
    }
    let csv_lines = crate::intent::surface_signals::inline_csv_record_block(text)?;
    Some(serde_json::json!({
        "action": "transform_data",
        "csv_text": csv_lines.join("\n"),
        "ops": [],
        "output_format": "md_table"
    }))
}

pub(super) fn inline_json_transform_deterministic_plan_result(
    goal: &str,
    state: &AppState,
    loop_state: &LoopState,
    original_user_text: &str,
    route_result: Option<&RouteResult>,
) -> Option<PlanResult> {
    if loop_state.round_no > 1
        || loop_state.has_tool_or_skill_output
        || !transform_skill_enabled_for_planning(state)
    {
        return None;
    }
    let args = inline_json_transform_args_from_text(original_user_text)
        .or_else(|| inline_json_transform_args_from_text(goal))
        .or_else(|| inline_json_scalar_count_args_from_contract(original_user_text, route_result))
        .or_else(|| inline_json_scalar_count_args_from_contract(goal, route_result))
        .or_else(|| inline_json_transform_args_from_candidate(original_user_text, route_result))
        .or_else(|| inline_json_transform_args_from_candidate(goal, route_result))
        .or_else(|| {
            contextual_inline_structured_transform_args_from_payload(
                original_user_text,
                route_result,
            )
        })
        .or_else(|| contextual_inline_structured_transform_args_from_payload(goal, route_result))
        .or_else(|| inline_csv_transform_args_from_text(original_user_text, route_result))
        .or_else(|| inline_csv_transform_args_from_text(goal, route_result))?;
    let actions = vec![AgentAction::CallSkill {
        skill: "transform".to_string(),
        args,
    }];
    let raw_plan_text = serde_json::to_string(&serde_json::json!({ "steps": actions }))
        .unwrap_or_else(|_| "{\"steps\":[]}".to_string());
    Some(build_plan_result(
        goal,
        &raw_plan_text,
        PlanKind::Single,
        &actions,
    ))
}

pub(super) fn replace_scalar_path_respond_only_with_auto_locator_observation(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || is_plain_respond_only_plan(&actions).is_none() {
        return actions;
    }
    let auto_locator_path = auto_locator_path.or_else(|| {
        loop_state
            .output_vars
            .get("auto_locator_path")
            .map(String::as_str)
    });
    if let Some(observation) =
        scalar_path_auto_locator_observation_plan(route_result, auto_locator_path)
    {
        info!("plan_replace_scalar_path_respond_only_with_auto_locator_observation");
        observation
    } else {
        actions
    }
}

pub(super) fn file_delivery_respond_only_observation_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if !route.wants_file_delivery
        && !route.output_contract.delivery_required
        && route.output_contract.response_shape != crate::OutputResponseShape::FileToken
    {
        return None;
    }
    let content = is_plain_respond_only_plan(actions)?;
    let parsed_file_token = crate::finalize::parse_delivery_file_token(content);
    let path = parsed_file_token
        .as_ref()
        .map(|(_kind, raw_path)| raw_path.trim())
        .filter(|path| !path.is_empty())
        .unwrap_or_else(|| route.output_contract.locator_hint.trim());
    file_delivery_path_observation_plan(state, path)
}

fn file_delivery_path_observation_plan(state: &AppState, path: &str) -> Option<Vec<AgentAction>> {
    let path = path.trim();
    if path.is_empty() || path.contains('\n') {
        return None;
    }
    let candidate = Path::new(path);
    let resolved = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    };
    let stat_action = AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args: serde_json::json!({
            "action": "stat_paths",
            "paths": [resolved.display().to_string()],
            "include_missing": true,
        }),
    };
    if resolved.is_file() {
        let token = format!("FILE:{}", resolved.display());
        return Some(vec![stat_action, AgentAction::Respond { content: token }]);
    }
    Some(vec![stat_action])
}

pub(super) fn replace_file_delivery_respond_only_with_path_observation(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output || is_plain_respond_only_plan(&actions).is_none() {
        return actions;
    }
    if let Some(observation) =
        file_delivery_respond_only_observation_plan(state, route_result, &actions)
    {
        info!("plan_replace_file_delivery_respond_only_with_path_observation");
        observation
    } else {
        actions
    }
}

fn file_delivery_empty_write_path<'a>(
    state: &AppState,
    action: &'a AgentAction,
) -> Option<&'a str> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    if canonical != "fs_basic" {
        return None;
    }
    let action_name = args.get("action").and_then(Value::as_str)?.trim();
    if action_name != "write_text" {
        return None;
    }
    let content = args.get("content").and_then(Value::as_str).unwrap_or("");
    if !content.is_empty() {
        return None;
    }
    args.get("path")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|path| !path.is_empty())
}

fn file_delivery_empty_write_targets_locator(
    state: &AppState,
    route: &RouteResult,
    action: &AgentAction,
) -> bool {
    let Some(path) = file_delivery_empty_write_path(state, action) else {
        return false;
    };
    let locator = route.output_contract.locator_hint.trim();
    if locator.is_empty() {
        return false;
    }
    let locator_path = resolve_delivery_token_path(state, locator);
    let action_path = resolve_delivery_token_path(state, path);
    same_existing_or_display_path(&locator_path, &action_path)
}

fn file_delivery_empty_write_placeholder_observation_plan(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: &[AgentAction],
) -> Option<Vec<AgentAction>> {
    let route = route_result?;
    if !file_delivery_contract_requires_file_token(route)
        || !matches!(
            route.output_contract.locator_kind,
            crate::OutputLocatorKind::Path | crate::OutputLocatorKind::Filename
        )
    {
        return None;
    }
    let mut has_empty_locator_write = false;
    for action in actions {
        if file_delivery_empty_write_targets_locator(state, route, action) {
            has_empty_locator_write = true;
            continue;
        }
        if action_is_likely_mutating(state, action) {
            return None;
        }
    }
    if !has_empty_locator_write {
        return None;
    }
    file_delivery_path_observation_plan(state, route.output_contract.locator_hint.trim())
}

pub(super) fn replace_file_delivery_empty_write_with_path_observation(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    if let Some(observation) =
        file_delivery_empty_write_placeholder_observation_plan(state, route_result, &actions)
    {
        info!("plan_replace_file_delivery_empty_write_with_path_observation");
        observation
    } else {
        actions
    }
}

pub(super) fn action_make_dir_path(state: &AppState, action: &AgentAction) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    let obj = args.as_object()?;
    match canonical.as_str() {
        "make_dir" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if action == "make_dir" {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

pub(super) fn file_delivery_contract_requires_file_token(route: &RouteResult) -> bool {
    route.wants_file_delivery
        || route.output_contract.delivery_required
        || route.output_contract.response_shape == crate::OutputResponseShape::FileToken
}

pub(super) fn file_delivery_contract_is_token_only(route: &RouteResult) -> bool {
    route.output_contract.response_shape == crate::OutputResponseShape::FileToken
        && matches!(
            route.output_contract.semantic_kind,
            crate::OutputSemanticKind::None | crate::OutputSemanticKind::GeneratedFileDelivery
        )
}

pub(super) fn generated_file_write_action_path(
    state: &AppState,
    action: &AgentAction,
) -> Option<String> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    let obj = args.as_object()?;
    match canonical.as_str() {
        "write_file" => obj.get("path").and_then(Value::as_str),
        "fs_basic" => {
            let action = obj.get("action").and_then(Value::as_str)?.trim();
            if matches!(action, "write_text" | "append_text") {
                obj.get("path").and_then(Value::as_str)
            } else {
                None
            }
        }
        _ => None,
    }
    .map(str::trim)
    .filter(|path| !path.is_empty())
    .map(ToString::to_string)
}

pub(super) fn resolve_delivery_token_path(state: &AppState, path: &str) -> PathBuf {
    let candidate = Path::new(path.trim());
    if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        state.skill_rt.workspace_root.join(candidate)
    }
}

pub(super) fn delivery_write_parent_matches_make_dir(
    state: &AppState,
    write_path: &str,
    make_dir_path: &str,
) -> bool {
    let write_path = resolve_delivery_token_path(state, write_path);
    let make_dir_path = resolve_delivery_token_path(state, make_dir_path);
    write_path
        .parent()
        .is_some_and(|parent| same_existing_or_display_path(parent, &make_dir_path))
}

pub(super) fn strip_redundant_make_dir_before_file_delivery_write(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    let write_paths = actions
        .iter()
        .filter_map(|action| generated_file_write_action_path(state, action))
        .collect::<Vec<_>>();
    if write_paths.is_empty() {
        return actions;
    }
    let original_len = actions.len();
    let stripped = actions
        .into_iter()
        .filter(|action| {
            let Some(make_dir_path) = action_make_dir_path(state, action) else {
                return true;
            };
            !write_paths.iter().any(|write_path| {
                delivery_write_parent_matches_make_dir(state, write_path, &make_dir_path)
            })
        })
        .collect::<Vec<_>>();
    if stripped.len() != original_len {
        info!(
            "plan_strip_redundant_make_dir_before_file_delivery_write removed={}",
            original_len.saturating_sub(stripped.len())
        );
    }
    stripped
}

pub(super) fn append_file_token_after_generated_file_write_delivery(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::Respond { content }
                if crate::finalize::parse_delivery_file_token(content.trim()).is_some()
        )
    }) {
        return actions;
    }
    let Some(path) = actions
        .iter()
        .rev()
        .find_map(|action| generated_file_write_action_path(state, action))
    else {
        return actions;
    };
    let resolved = resolve_delivery_token_path(state, &path);
    let token = format!("FILE:{}", resolved.display());
    let mut rewritten = actions;
    match rewritten.last_mut() {
        Some(AgentAction::Respond { content }) if file_delivery_contract_is_token_only(route) => {
            *content = token;
        }
        _ => rewritten.push(AgentAction::Respond { content: token }),
    }
    info!("plan_append_file_token_after_generated_file_write_delivery");
    rewritten
}

pub(super) fn existing_file_delivery_observation_path(
    state: &AppState,
    action: &AgentAction,
) -> Option<PathBuf> {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return None,
    };
    if state.resolve_canonical_skill_name(skill) != "fs_basic" {
        return None;
    }
    let obj = args.as_object()?;
    let action = obj.get("action").and_then(Value::as_str).map(str::trim);
    if !matches!(action, Some("stat_paths" | "path_batch_facts")) {
        return None;
    }
    let path = obj
        .get("paths")
        .and_then(Value::as_array)
        .and_then(|paths| paths.iter().find_map(Value::as_str))
        .or_else(|| obj.get("path").and_then(Value::as_str))
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    let resolved = resolve_delivery_token_path(state, path);
    resolved.is_file().then_some(resolved)
}

pub(super) fn append_file_token_after_existing_file_delivery_observation(
    state: &AppState,
    route_result: Option<&RouteResult>,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    let Some(route) = route_result else {
        return actions;
    };
    if !file_delivery_contract_requires_file_token(route) {
        return actions;
    }
    if !file_delivery_contract_is_token_only(route) {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::Respond { content }
                if crate::finalize::parse_delivery_file_token(content.trim()).is_some()
        )
    }) {
        return actions;
    }
    let Some(path) = actions
        .iter()
        .rev()
        .find_map(|action| existing_file_delivery_observation_path(state, action))
        .or_else(|| {
            let hint = route.output_contract.locator_hint.trim();
            if hint.is_empty() {
                return None;
            }
            let resolved = resolve_delivery_token_path(state, hint);
            resolved.is_file().then_some(resolved)
        })
    else {
        return actions;
    };
    let token = format!("FILE:{}", path.display());
    let mut rewritten = actions;
    match rewritten.last_mut() {
        Some(AgentAction::Respond { content }) => *content = token,
        _ => rewritten.push(AgentAction::Respond { content: token }),
    }
    info!("plan_append_file_token_after_existing_file_delivery_observation");
    rewritten
}

pub(super) fn route_is_existing_file_content_delivery(
    state: &AppState,
    route_result: Option<&RouteResult>,
) -> bool {
    let Some(route) = route_result else {
        return false;
    };
    if !route.output_contract.delivery_required
        || route.output_contract.delivery_intent != crate::OutputDeliveryIntent::FileSingle
    {
        return false;
    }
    match route.output_contract.semantic_kind {
        crate::OutputSemanticKind::GeneratedFileDelivery => {
            if route.output_contract.response_shape != crate::OutputResponseShape::FileToken {
                return false;
            }
        }
        kind if kind.is_content_excerpt_summary() => {}
        _ => return false,
    }
    let hint = route.output_contract.locator_hint.trim();
    if hint.is_empty() {
        return false;
    }
    let resolved = resolve_delivery_token_path(state, hint);
    resolved.is_file()
}

pub(super) fn action_is_existing_file_content_read(state: &AppState, action: &AgentAction) -> bool {
    let (skill, args) = match action {
        AgentAction::CallSkill { skill, args } => (skill, args),
        AgentAction::CallTool { tool, args } => (tool, args),
        _ => return false,
    };
    let canonical = state.resolve_canonical_skill_name(skill);
    if canonical == "doc_parse" || canonical == "read_file" {
        return true;
    }
    if canonical != "fs_basic" {
        return false;
    }
    args.as_object()
        .and_then(|obj| obj.get("action"))
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|action| matches!(action, "read_text_range" | "read_range"))
}

pub(super) fn respond_content_has_file_token_line_and_prose(content: &str) -> bool {
    let mut has_token = false;
    let mut has_non_token = false;
    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        if crate::finalize::parse_delivery_file_token(line).is_some() {
            has_token = true;
        } else {
            has_non_token = true;
        }
    }
    has_token && has_non_token
}

pub(super) fn rewrite_mixed_file_token_prose_respond_to_synthesize_answer(
    state: &AppState,
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if !route_is_existing_file_content_delivery(state, route_result)
        || (!actions
            .iter()
            .any(|action| action_is_existing_file_content_read(state, action))
            && !loop_state
                .executed_step_results
                .iter()
                .any(executed_step_is_successful_text_read))
    {
        return actions;
    }
    let mut rewritten = actions;
    let Some(AgentAction::Respond { content }) = rewritten.last() else {
        return rewritten;
    };
    if !respond_content_has_file_token_line_and_prose(content) {
        return rewritten;
    }
    *rewritten.last_mut().expect("last action exists") = AgentAction::SynthesizeAnswer {
        evidence_refs: vec!["last_output".to_string()],
    };
    info!("plan_rewrite_mixed_file_token_prose_respond_to_synthesize_answer");
    rewritten
}

pub(super) fn scalar_count_locator_path(
    route_result: Option<&RouteResult>,
    auto_locator_path: Option<&str>,
) -> Option<String> {
    let route = route_result?;
    if route.needs_clarify
        || !route.output_contract.requires_content_evidence
        || route.output_contract.delivery_required
        || !scalar_count_contract_allows_count_shape(route)
        || route.output_contract.semantic_kind != crate::OutputSemanticKind::ScalarCount
        || route_requests_hidden_entries_count(route)
    {
        return None;
    }
    route_directory_locator_path(route, auto_locator_path)
}

pub(super) fn scalar_count_contract_allows_count_shape(route: &RouteResult) -> bool {
    matches!(
        route.output_contract.response_shape,
        crate::OutputResponseShape::Scalar | crate::OutputResponseShape::OneSentence
    ) || (route.output_contract.response_shape == crate::OutputResponseShape::Strict
        && route.output_contract.exact_sentence_count == Some(1))
}

pub(super) fn replace_scalar_count_plan_with_count_inventory(
    route_result: Option<&RouteResult>,
    loop_state: &LoopState,
    auto_locator_path: Option<&str>,
    user_text: &str,
    actions: Vec<AgentAction>,
) -> Vec<AgentAction> {
    if loop_state.has_tool_or_skill_output {
        return actions;
    }
    if actions.iter().any(|action| {
        matches!(
            action,
            AgentAction::CallSkill { skill, .. } | AgentAction::CallTool { tool: skill, .. }
                if skill.eq_ignore_ascii_case("run_cmd")
        )
    }) {
        return actions;
    }
    let Some(path) = scalar_count_explicit_count_path_from_actions(&actions)
        .or_else(|| scalar_count_locator_path(route_result, auto_locator_path))
    else {
        return actions;
    };
    if !Path::new(&path).is_dir() {
        info!("plan_replace_scalar_count_missing_locator_with_path_facts path={path}");
        let answer = if crate::language_policy::request_language_hint(user_text)
            .to_ascii_lowercase()
            .starts_with("en")
        {
            format!("{path} does not exist, so the matching item count cannot be computed.")
        } else {
            format!("{path} 不存在，无法统计匹配项数量。")
        };
        return vec![
            AgentAction::CallTool {
                tool: "fs_basic".to_string(),
                args: serde_json::json!({
                    "action": "stat_paths",
                    "paths": [path],
                    "include_missing": true,
                }),
            },
            AgentAction::Respond { content: answer },
        ];
    }
    info!("plan_replace_scalar_count_plan_with_count_inventory");
    if scalar_count_actions_include_listing(&actions) {
        info!("plan_scalar_count_listing_requires_structured_count_repair");
        return actions;
    }
    let inventory_kind = scalar_count_inventory_kind_from_actions(&actions);
    let mut args = serde_json::json!({
        "action": "count_entries",
        "path": path,
    });
    if let Some(obj) = args.as_object_mut() {
        apply_scalar_count_inventory_filters_from_actions(obj, &actions);
        match inventory_kind {
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
        if let Some(hint) = route_result.and_then(scalar_count_filter_hint_from_route) {
            apply_scalar_count_filter_hint(obj, &hint);
        }
    }
    vec![AgentAction::CallTool {
        tool: "fs_basic".to_string(),
        args,
    }]
}

pub(super) fn scalar_count_actions_include_listing(actions: &[AgentAction]) -> bool {
    actions.iter().any(|action| {
        let (skill, args) = match action {
            AgentAction::CallSkill { skill, args }
            | AgentAction::CallTool { tool: skill, args } => (skill.as_str(), args),
            _ => return false,
        };
        let action_name = args
            .get("action")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_ascii_lowercase());
        skill.eq_ignore_ascii_case("list_dir")
            || (skill.eq_ignore_ascii_case("fs_basic")
                && matches!(action_name.as_deref(), Some("list_dir")))
            || (skill.eq_ignore_ascii_case("system_basic")
                && matches!(action_name.as_deref(), Some("inventory_dir")))
    })
}
