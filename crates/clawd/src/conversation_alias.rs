use serde_json::Value;

use super::{ConversationState, SessionAliasBinding, MAX_SESSION_ALIAS_BINDINGS};

fn normalize_alias_target(raw_target: &str) -> Option<String> {
    let trimmed = raw_target
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(trimmed);
    crate::intent::locator_extractor::extract_explicit_locator_for_fallback(trimmed)
        .map(|locator| locator.locator_hint)
        .or_else(|| surface.single_filename_candidate().map(ToString::to_string))
        .or_else(|| Some(trimmed.to_string()))
}

fn normalize_explicit_alias_target(raw_target: &str) -> Option<String> {
    let trimmed = raw_target
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(trimmed);
    crate::intent::locator_extractor::extract_explicit_locator_for_fallback(trimmed)
        .map(|locator| locator.locator_hint)
        .or_else(|| surface.single_filename_candidate().map(ToString::to_string))
}

fn normalized_alias_surface_for_match(raw: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    for ch in raw.trim().chars() {
        let mapped = if matches!(ch, '_' | '-') { ' ' } else { ch };
        if mapped.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space && !out.ends_with(' ') {
            out.push(' ');
        }
        for lower in mapped.to_lowercase() {
            out.push(lower);
        }
        pending_space = false;
    }
    out.trim().to_string()
}

pub(crate) fn alias_surface_matches_prompt(prompt: &str, alias: &str) -> bool {
    let alias = normalized_alias_surface_for_match(alias);
    if alias.is_empty() {
        return false;
    }
    normalized_alias_surface_for_match(prompt).contains(&alias)
}

#[cfg(test)]
pub(crate) fn single_alias_binding_mentioned_in_prompt<'a>(
    bindings: &'a [SessionAliasBinding],
    prompt: &str,
) -> Option<&'a SessionAliasBinding> {
    let mut matches = alias_bindings_mentioned_in_prompt(bindings, prompt);
    if matches.is_empty() {
        return None;
    }
    let target = matches[0].target.trim();
    if matches.len() == 1
        || matches
            .iter()
            .all(|binding| binding.target.trim() == target)
    {
        matches.sort_by_key(|binding| {
            std::cmp::Reverse(
                normalized_alias_surface_for_match(&binding.alias)
                    .chars()
                    .count(),
            )
        });
        return Some(matches.remove(0));
    }
    None
}

pub(crate) fn alias_bindings_mentioned_in_prompt<'a>(
    bindings: &'a [SessionAliasBinding],
    prompt: &str,
) -> Vec<&'a SessionAliasBinding> {
    let mut matches = bindings
        .iter()
        .filter(|binding| alias_surface_matches_prompt(prompt, &binding.alias))
        .collect::<Vec<_>>();
    matches.dedup_by(|left, right| left.alias == right.alias && left.target == right.target);
    matches
}

pub(crate) fn session_alias_bindings_from_state_patch(
    state_patch: Option<&Value>,
) -> Vec<SessionAliasBinding> {
    let Some(state_patch) = state_patch else {
        return Vec::new();
    };
    let now_ts = crate::now_ts_u64();
    let mut out = Vec::new();
    if let Some(alias_bindings) = state_patch
        .get("alias_bindings")
        .and_then(|value| value.as_array())
    {
        for item in alias_bindings {
            let Some(alias) = item
                .get("alias")
                .or_else(|| item.get("alias_key"))
                .or_else(|| item.get("surface"))
                .or_else(|| item.get("name"))
                .or_else(|| item.get("alias_name"))
                .and_then(|value| value.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                continue;
            };
            let Some(target) = alias_binding_target_value(item).and_then(normalize_alias_target)
            else {
                continue;
            };
            if out
                .iter()
                .any(|existing: &SessionAliasBinding| existing.alias.eq_ignore_ascii_case(alias))
            {
                continue;
            }
            out.push(SessionAliasBinding {
                alias: alias.to_string(),
                target,
                updated_at_ts: now_ts,
            });
            if out.len() >= MAX_SESSION_ALIAS_BINDINGS {
                return out;
            }
        }
    }
    if let Some(alias_bindings) = state_patch
        .get("alias_bindings")
        .and_then(|value| value.as_object())
    {
        if alias_binding_record_map_present(alias_bindings) {
            if let Some((alias, target)) = alias_binding_record_from_map(alias_bindings) {
                if !out.iter().any(|existing: &SessionAliasBinding| {
                    existing.alias.eq_ignore_ascii_case(&alias)
                }) {
                    out.push(SessionAliasBinding {
                        alias,
                        target,
                        updated_at_ts: now_ts,
                    });
                }
            }
            return out;
        }
        if let Some(update_items) = alias_binding_update_items(alias_bindings) {
            for item in update_items {
                let Some(alias) = item
                    .get("alias")
                    .or_else(|| item.get("alias_key"))
                    .or_else(|| item.get("surface"))
                    .or_else(|| item.get("name"))
                    .or_else(|| item.get("alias_name"))
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                else {
                    continue;
                };
                let Some(target) =
                    alias_binding_target_value(item).and_then(normalize_alias_target)
                else {
                    continue;
                };
                if out.iter().any(|existing: &SessionAliasBinding| {
                    existing.alias.eq_ignore_ascii_case(alias)
                }) {
                    continue;
                }
                out.push(SessionAliasBinding {
                    alias: alias.to_string(),
                    target,
                    updated_at_ts: now_ts,
                });
                if out.len() >= MAX_SESSION_ALIAS_BINDINGS {
                    return out;
                }
            }
            if !out.is_empty() {
                return out;
            }
        }
        for (alias, value) in alias_bindings {
            let alias = alias.trim();
            if alias.is_empty() {
                continue;
            }
            let Some(target) = compatibility_alias_target(value).and_then(normalize_alias_target)
            else {
                continue;
            };
            if out
                .iter()
                .any(|existing: &SessionAliasBinding| existing.alias.eq_ignore_ascii_case(alias))
            {
                continue;
            }
            out.push(SessionAliasBinding {
                alias: alias.to_string(),
                target,
                updated_at_ts: now_ts,
            });
            if out.len() >= MAX_SESSION_ALIAS_BINDINGS {
                return out;
            }
        }
    }
    let Some(obj) = state_patch.as_object() else {
        return out;
    };
    for (key, value) in obj {
        let alias_and_target = compatibility_alias_key(key)
            .and_then(|alias| {
                compatibility_alias_target(value)
                    .and_then(normalize_alias_target)
                    .map(|target| (alias, target))
            })
            .or_else(|| {
                direct_alias_map_key(key).and_then(|alias| {
                    compatibility_alias_target(value)
                        .and_then(normalize_explicit_alias_target)
                        .map(|target| (alias, target))
                })
            });
        let Some((alias, target)) = alias_and_target else {
            continue;
        };
        if out
            .iter()
            .any(|existing: &SessionAliasBinding| existing.alias.eq_ignore_ascii_case(&alias))
        {
            continue;
        }
        out.push(SessionAliasBinding {
            alias,
            target,
            updated_at_ts: now_ts,
        });
        if out.len() >= MAX_SESSION_ALIAS_BINDINGS {
            break;
        }
    }
    out
}

pub(crate) fn state_patch_is_alias_bindings_only(state_patch: &Value) -> bool {
    let Some(obj) = state_patch.as_object() else {
        return false;
    };
    !obj.is_empty()
        && obj.iter().all(|(key, value)| {
            if !json_value_is_meaningful(value) {
                return true;
            }
            if key == "alias_bindings" {
                return alias_bindings_value_is_well_formed(value);
            }
            if alias_bindings_metadata_is_ignorable(key, value) {
                return true;
            }
            compatibility_alias_key(key).is_some()
                && compatibility_alias_target(value)
                    .and_then(normalize_alias_target)
                    .is_some()
                || direct_alias_map_key(key).is_some()
                    && compatibility_alias_target(value)
                        .and_then(normalize_explicit_alias_target)
                        .is_some()
        })
}

fn alias_bindings_metadata_is_ignorable(key: &str, value: &Value) -> bool {
    match key {
        "forbidden_visible_literals"
        | "required_content_literals"
        | "required_visible_literals" => true,
        "required_machine_fields" => required_machine_fields_are_alias_bindings_only(value),
        "primary_task_update" => {
            primary_task_update_value_is_inactive(value)
                || primary_task_update_value_is_alias_binding_metadata(value)
                || primary_task_update_value_is_alias_ack_projection(value)
        }
        _ => false,
    }
}

fn required_machine_fields_are_alias_bindings_only(value: &Value) -> bool {
    match value {
        Value::String(text) => text.trim() == "alias_bindings",
        Value::Array(items) => {
            !items.is_empty()
                && items.iter().all(|item| {
                    item.as_str()
                        .map(str::trim)
                        .is_some_and(|field| field == "alias_bindings")
                })
        }
        _ => false,
    }
}

fn primary_task_update_value_is_alias_ack_projection(value: &Value) -> bool {
    let Some(map) = value.as_object() else {
        return false;
    };
    !map.is_empty()
        && map.keys().all(|key| {
            matches!(
                key.as_str(),
                "last_primary_task_prompt" | "last_primary_task_output"
            )
        })
}

fn primary_task_update_value_is_alias_binding_metadata(value: &Value) -> bool {
    let Some(action) = value
        .as_object()
        .and_then(|map| map.get("action").or_else(|| map.get("kind")))
        .and_then(Value::as_str)
        .map(str::trim)
        .map(|action| action.to_ascii_lowercase())
    else {
        return false;
    };
    matches!(action.as_str(), "alias_update" | "alias_rebind")
}

fn primary_task_update_value_is_inactive(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(active) => !*active,
        Value::Object(map) => map.values().all(primary_task_update_value_is_inactive),
        Value::Array(items) => items.iter().all(primary_task_update_value_is_inactive),
        Value::String(text) => {
            let normalized = text.trim().to_ascii_lowercase();
            normalized.is_empty() || matches!(normalized.as_str(), "false" | "none" | "null")
        }
        Value::Number(_) => false,
    }
}

fn alias_bindings_value_is_well_formed(value: &Value) -> bool {
    if let Some(items) = value.as_array() {
        return !items.is_empty()
            && items.iter().all(|item| {
                let alias = item
                    .get("alias")
                    .or_else(|| item.get("alias_key"))
                    .or_else(|| item.get("surface"))
                    .or_else(|| item.get("name"))
                    .or_else(|| item.get("alias_name"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|alias| !alias.is_empty());
                let target = alias_binding_target_value(item)
                    .map(str::trim)
                    .filter(|target| !target.is_empty());
                alias.is_some() && target.is_some()
            });
    }
    value.as_object().is_some_and(|bindings| {
        if alias_binding_record_map_present(bindings) {
            return alias_binding_record_from_map(bindings).is_some();
        }
        if alias_binding_update_map_present(bindings) {
            return alias_binding_update_map_is_well_formed(bindings);
        }
        !bindings.is_empty()
            && bindings.iter().all(|(alias, target)| {
                !alias.trim().is_empty()
                    && compatibility_alias_target(target)
                        .map(str::trim)
                        .is_some_and(|target| !target.is_empty())
            })
    })
}

fn alias_binding_target_value(item: &Value) -> Option<&str> {
    item.get("target")
        .or_else(|| item.get("path"))
        .or_else(|| item.get("value"))
        .or_else(|| item.get("locator"))
        .or_else(|| item.get("locator_hint"))
        .or_else(|| item.get("locator_value"))
        .or_else(|| item.get("target_value"))
        .or_else(|| item.get("target_path"))
        .or_else(|| item.get("target_abs"))
        .or_else(|| item.get("alias_target"))
        .or_else(|| item.get("absolute_path"))
        .or_else(|| item.get("alias_target_path"))
        .or_else(|| item.get("alias_target_abs"))
        .and_then(Value::as_str)
}

fn alias_binding_record_map_present(map: &serde_json::Map<String, Value>) -> bool {
    map.contains_key("alias")
        || map.contains_key("alias_key")
        || map.contains_key("surface")
        || map.contains_key("name")
        || map.contains_key("alias_name")
        || map.contains_key("action")
        || map.contains_key("target")
        || map.contains_key("alias_target")
        || map.contains_key("target_abs")
        || map.contains_key("target_path")
        || map.contains_key("alias_target_path")
        || map.contains_key("locator_hint")
}

fn alias_binding_record_from_map(map: &serde_json::Map<String, Value>) -> Option<(String, String)> {
    let alias = map
        .get("alias")
        .or_else(|| map.get("alias_key"))
        .or_else(|| map.get("surface"))
        .or_else(|| map.get("name"))
        .or_else(|| map.get("alias_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|alias| !alias.is_empty())?;
    let target =
        alias_binding_target_value(&Value::Object(map.clone())).and_then(normalize_alias_target)?;
    Some((alias.to_string(), target))
}

fn alias_binding_update_items<'a>(map: &'a serde_json::Map<String, Value>) -> Option<&'a [Value]> {
    ["add_or_update", "upsert", "add", "update"]
        .iter()
        .find_map(|key| {
            map.get(*key)
                .and_then(Value::as_array)
                .filter(|items| !items.is_empty())
                .map(Vec::as_slice)
        })
}

fn alias_binding_update_map_present(map: &serde_json::Map<String, Value>) -> bool {
    alias_binding_update_items(map).is_some() || map.contains_key("remove")
}

fn alias_binding_update_map_is_well_formed(map: &serde_json::Map<String, Value>) -> bool {
    let Some(update_items) = alias_binding_update_items(map) else {
        return false;
    };
    let update_keys = ["add_or_update", "upsert", "add", "update"];
    map.iter().all(|(key, value)| {
        if update_keys.contains(&key.as_str()) {
            return value.as_array().is_some_and(|items| {
                !items.is_empty() && items.iter().all(alias_binding_update_item_is_well_formed)
            });
        }
        if key == "remove" {
            return !json_value_is_meaningful(value);
        }
        false
    }) && update_items
        .iter()
        .all(alias_binding_update_item_is_well_formed)
}

fn alias_binding_update_item_is_well_formed(item: &Value) -> bool {
    let alias = item
        .get("alias")
        .or_else(|| item.get("alias_key"))
        .or_else(|| item.get("surface"))
        .or_else(|| item.get("name"))
        .or_else(|| item.get("alias_name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|alias| !alias.is_empty());
    let target = alias_binding_target_value(item)
        .map(str::trim)
        .filter(|target| !target.is_empty());
    alias.is_some() && target.is_some()
}

fn json_value_is_meaningful(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(text) => !text.trim().is_empty(),
        Value::Array(items) => items.iter().any(json_value_is_meaningful),
        Value::Object(map) => map.values().any(json_value_is_meaningful),
        _ => true,
    }
}

fn compatibility_alias_key(key: &str) -> Option<String> {
    let trimmed = key.trim();
    let alias = trimmed
        .strip_suffix("_alias")
        .or_else(|| trimmed.strip_suffix("Alias"))?
        .trim_matches(|ch: char| ch == '_' || ch == '-' || ch.is_whitespace())
        .trim();
    (!alias.is_empty()).then(|| alias.to_string())
}

fn direct_alias_map_key(key: &str) -> Option<String> {
    let trimmed = key.trim();
    if trimmed.is_empty() || state_patch_schema_key(trimmed) {
        return None;
    }
    Some(trimmed.to_string())
}

fn state_patch_schema_key(key: &str) -> bool {
    matches!(
        key,
        "alias_bindings"
            | "active_task_boundary"
            | "audience"
            | "constraints"
            | "deictic_reference"
            | "deliverable"
            | "filename_only"
            | "format"
            | "ordered_entry_ref"
            | "ordered_entry_reference"
            | "output_format"
            | "forbidden_visible_literals"
            | "primary_task_update"
            | "required_content_literals"
            | "required_visible_literals"
            | "scope"
            | "target"
    )
}

fn compatibility_alias_target(value: &Value) -> Option<&str> {
    if let Some(target) = value.as_str() {
        return Some(target);
    }
    value
        .as_object()
        .and_then(|obj| {
            obj.get("target")
                .or_else(|| obj.get("path"))
                .or_else(|| obj.get("value"))
                .or_else(|| obj.get("locator"))
                .or_else(|| obj.get("locator_hint"))
                .or_else(|| obj.get("locator_value"))
                .or_else(|| obj.get("target_value"))
                .or_else(|| obj.get("target_path"))
                .or_else(|| obj.get("alias_target"))
                .or_else(|| obj.get("target_abs"))
                .or_else(|| obj.get("absolute_path"))
                .or_else(|| obj.get("alias_target_path"))
                .or_else(|| obj.get("alias_target_abs"))
        })
        .and_then(Value::as_str)
}

pub(super) fn merge_alias_bindings(
    prior_state: Option<&ConversationState>,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
) -> Vec<SessionAliasBinding> {
    let mut alias_bindings = prior_state
        .map(|state| state.alias_bindings.clone())
        .unwrap_or_default();
    let parsed = session_alias_bindings_from_state_patch(
        turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()),
    );
    if parsed.is_empty() {
        return alias_bindings;
    }
    for binding in parsed {
        alias_bindings.retain(|existing| existing.alias != binding.alias);
        alias_bindings.push(binding);
    }
    if alias_bindings.len() > MAX_SESSION_ALIAS_BINDINGS {
        let start = alias_bindings.len() - MAX_SESSION_ALIAS_BINDINGS;
        alias_bindings = alias_bindings.split_off(start);
    }
    alias_bindings
}

pub(super) fn merge_alias_bindings_for_turn(
    prior_state: Option<&ConversationState>,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
    prompt: &str,
    route_result: &crate::IntentOutputContract,
    resolved_prompt_for_execution: &str,
) -> Vec<SessionAliasBinding> {
    let mut alias_bindings = merge_alias_bindings(prior_state, turn_analysis);
    if turn_analysis_has_structured_alias_bindings(turn_analysis) {
        return alias_bindings;
    }
    for binding in structural_alias_bindings_from_prompt(
        prior_state,
        turn_analysis,
        prompt,
        route_result,
        resolved_prompt_for_execution,
    ) {
        alias_bindings.retain(|existing| existing.alias != binding.alias);
        alias_bindings.push(binding);
    }
    if alias_bindings.len() > MAX_SESSION_ALIAS_BINDINGS {
        let start = alias_bindings.len() - MAX_SESSION_ALIAS_BINDINGS;
        alias_bindings = alias_bindings.split_off(start);
    }
    alias_bindings
}

fn turn_analysis_has_structured_alias_bindings(
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
) -> bool {
    !session_alias_bindings_from_state_patch(
        turn_analysis.and_then(|analysis| analysis.state_patch.as_ref()),
    )
    .is_empty()
}

pub(super) fn turn_analysis_has_alias_only_state_patch(
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
) -> bool {
    turn_analysis
        .and_then(|analysis| analysis.state_patch.as_ref())
        .is_some_and(state_patch_is_alias_bindings_only)
}

fn structural_alias_bindings_from_prompt(
    prior_state: Option<&ConversationState>,
    turn_analysis: Option<&crate::turn_context::TurnAnalysis>,
    prompt: &str,
    route_result: &crate::IntentOutputContract,
    resolved_prompt_for_execution: &str,
) -> Vec<SessionAliasBinding> {
    let mut out = Vec::new();
    let multi_bindings = structural_quoted_alias_bindings_from_prompt(prompt);
    if !multi_bindings.is_empty() {
        out.extend(multi_bindings);
    } else if let Some(binding) = structural_quoted_alias_binding_from_single_locator_prompt(prompt)
    {
        out.push(binding);
    } else if let Some(binding) =
        structural_alias_binding_from_prompt(prompt, route_result, resolved_prompt_for_execution)
    {
        out.push(binding);
    } else if turn_analysis
        .and_then(|analysis| analysis.turn_type)
        .is_some_and(|turn_type| {
            matches!(turn_type, crate::turn_context::TurnType::PreferenceOrMemory)
        })
    {
        out.extend(structural_alias_bindings_from_single_locator_prefix(prompt));
    }
    let rebinds = structural_alias_rebinds_from_prompt(prior_state, prompt);
    if !rebinds.is_empty() {
        out.extend(rebinds);
    } else if false {
        out.extend(structural_alias_bindings_from_single_locator_prefix(prompt));
    }
    out
}

pub(crate) fn structural_quoted_alias_bindings_from_prompt(
    prompt: &str,
) -> Vec<SessionAliasBinding> {
    let mut locators =
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt);
    locators.dedup_by(|left, right| left.locator_hint == right.locator_hint);
    if locators.len() < 2 {
        return Vec::new();
    }

    let now_ts = crate::now_ts_u64();
    let mut out = Vec::new();
    let mut segment_start = 0usize;
    for locator in locators {
        let Some((locator_start, locator_end)) =
            find_locator_span_after(prompt, &locator.locator_hint, segment_start)
        else {
            continue;
        };
        if let Some(alias) =
            last_structural_quoted_alias_in_range(prompt, segment_start, locator_start)
        {
            if let Some(target) = normalize_alias_target(&locator.locator_hint) {
                if !out.iter().any(|existing: &SessionAliasBinding| {
                    existing.alias.eq_ignore_ascii_case(&alias) || existing.target == target
                }) {
                    out.push(SessionAliasBinding {
                        alias,
                        target,
                        updated_at_ts: now_ts,
                    });
                }
            }
        }
        segment_start = locator_end;
    }
    (out.len() >= 2).then_some(out).unwrap_or_default()
}

fn find_locator_span_after(prompt: &str, locator: &str, start: usize) -> Option<(usize, usize)> {
    if locator.trim().is_empty() || start >= prompt.len() {
        return None;
    }
    prompt
        .get(start..)
        .and_then(|tail| tail.find(locator).map(|offset| start + offset))
        .or_else(|| prompt.find(locator))
        .map(|idx| (idx, idx + locator.len()))
}

fn last_structural_quoted_alias_in_range(prompt: &str, start: usize, end: usize) -> Option<String> {
    if start >= end || end > prompt.len() {
        return None;
    }
    structural_quoted_aliases_with_spans(prompt)
        .into_iter()
        .filter(|(alias_start, alias_end, _)| *alias_start >= start && *alias_end <= end)
        .max_by_key(|(_, alias_end, _)| *alias_end)
        .map(|(_, _, alias)| alias)
}

fn structural_quoted_aliases_with_spans(text: &str) -> Vec<(usize, usize, String)> {
    let mut aliases = Vec::new();
    for (open, close) in [('“', '”'), ('"', '"'), ('\'', '\''), ('`', '`')] {
        let mut inside = false;
        let mut start = 0usize;
        for (idx, ch) in text.char_indices() {
            if !inside && ch == open {
                inside = true;
                start = idx + ch.len_utf8();
                continue;
            }
            if inside && ch == close {
                if let Some(alias) = text
                    .get(start..idx)
                    .map(str::trim)
                    .filter(|candidate| structural_alias_candidate_is_safe(candidate))
                {
                    aliases.push((start, idx, alias.to_string()));
                }
                inside = false;
            }
        }
    }
    aliases.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    aliases.dedup_by(|left, right| left.0 == right.0 && left.1 == right.1 && left.2 == right.2);
    aliases
}

pub(crate) fn structural_quoted_alias_binding_from_single_locator_prompt(
    prompt: &str,
) -> Option<SessionAliasBinding> {
    let Some((surface, target)) = single_current_prompt_locator_surface_and_target(prompt) else {
        return None;
    };
    let Some(idx) = prompt.find(&surface) else {
        return None;
    };
    let prefix = prompt[..idx].trim();
    let alias = single_structural_quoted_alias(prefix)?;
    Some(SessionAliasBinding {
        alias,
        target,
        updated_at_ts: crate::now_ts_u64(),
    })
}

pub(crate) fn structural_alias_rebinds_from_prompt(
    prior_state: Option<&ConversationState>,
    prompt: &str,
) -> Vec<SessionAliasBinding> {
    let Some(prior) = prior_state else {
        return Vec::new();
    };
    let target = match single_current_prompt_locator_target(prompt) {
        Some(target) if !target.trim().is_empty() => target,
        _ => return Vec::new(),
    };
    let now_ts = crate::now_ts_u64();
    alias_bindings_mentioned_in_prompt(&prior.alias_bindings, prompt)
        .into_iter()
        .filter(|existing| existing.target != target)
        .map(|existing| SessionAliasBinding {
            alias: existing.alias.clone(),
            target: target.clone(),
            updated_at_ts: now_ts,
        })
        .collect()
}

fn structural_alias_bindings_from_single_locator_prefix(prompt: &str) -> Vec<SessionAliasBinding> {
    let Some((surface, target)) = single_current_prompt_locator_surface_and_target(prompt) else {
        return Vec::new();
    };
    let Some(idx) = prompt.find(&surface) else {
        return Vec::new();
    };
    let prefix = prompt[..idx].trim();
    let aliases = alias_suffix_candidates_from_prefix(prefix);
    let now_ts = crate::now_ts_u64();
    aliases
        .into_iter()
        .map(|alias| SessionAliasBinding {
            alias,
            target: target.clone(),
            updated_at_ts: now_ts,
        })
        .collect()
}

fn single_current_prompt_locator_target(prompt: &str) -> Option<String> {
    single_current_prompt_locator_surface_and_target(prompt).map(|(_, target)| target)
}

fn single_current_prompt_locator_surface_and_target(prompt: &str) -> Option<(String, String)> {
    let mut locators =
        crate::intent::locator_extractor::extract_explicit_locator_candidates_for_fallback(prompt);
    locators.dedup_by(|left, right| left.locator_hint == right.locator_hint);
    if locators.len() != 1 {
        return None;
    }
    let surface = locators.remove(0).locator_hint;
    let target = normalize_alias_target(&surface)?;
    Some((surface, target))
}

fn alias_suffix_candidates_from_prefix(prefix: &str) -> Vec<String> {
    if let Some(candidate) = machine_alias_suffix_candidate_from_prefix(prefix) {
        return vec![candidate];
    }
    let tokens = prefix
        .split_whitespace()
        .map(|token| {
            token
                .trim_matches(|ch: char| {
                    ch.is_ascii_punctuation()
                        || matches!(ch, '，' | '。' | '；' | '：' | '“' | '”' | '‘' | '’')
                })
                .trim()
        })
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    let mut out = Vec::new();
    if tokens.len() >= 3 {
        let base = &tokens[..tokens.len() - 1];
        for len in 2..=base.len().min(4) {
            let candidate = base[base.len() - len..].join(" ");
            if structural_alias_candidate_is_safe(&candidate)
                && !out
                    .iter()
                    .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
            {
                out.push(candidate);
            }
        }
    }
    for candidate in compact_alias_suffix_candidates_from_prefix(prefix) {
        if structural_alias_candidate_is_safe(&candidate)
            && !out
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&candidate))
        {
            out.push(candidate);
        }
    }
    out
}

fn machine_alias_suffix_candidate_from_prefix(prefix: &str) -> Option<String> {
    prefix
        .split_whitespace()
        .rev()
        .map(trim_alias_token)
        .find(|token| machine_alias_token_is_safe(token))
        .map(ToString::to_string)
}

fn trim_alias_token(token: &str) -> &str {
    token
        .trim_matches(|ch: char| {
            ch.is_ascii_punctuation()
                || matches!(
                    ch,
                    '，' | '。'
                        | '；'
                        | '：'
                        | '、'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '（'
                        | '）'
                        | '【'
                        | '】'
                        | '《'
                        | '》'
                )
        })
        .trim()
}

fn machine_alias_token_is_safe(token: &str) -> bool {
    let char_count = token.chars().count();
    if !(2..=64).contains(&char_count) || !structural_alias_candidate_is_safe(token) {
        return false;
    }
    let mut has_alpha = false;
    let mut has_digit = false;
    let mut has_separator = false;
    let mut alpha_count = 0usize;
    let mut uppercase_alpha_count = 0usize;
    for ch in token.chars() {
        if ch.is_ascii_alphabetic() {
            has_alpha = true;
            alpha_count += 1;
            if ch.is_ascii_uppercase() {
                uppercase_alpha_count += 1;
            }
            continue;
        }
        if ch.is_ascii_digit() {
            has_digit = true;
            continue;
        }
        if matches!(ch, '_' | '-') {
            has_separator = true;
            continue;
        }
        return false;
    }
    has_alpha
        && (has_separator
            || has_digit
            || (alpha_count >= 2 && alpha_count == uppercase_alpha_count))
}

fn compact_alias_suffix_candidates_from_prefix(prefix: &str) -> Vec<String> {
    let segment = prefix
        .rsplit(|ch: char| {
            ch.is_whitespace()
                || ch.is_ascii_punctuation()
                || matches!(
                    ch,
                    '，' | '。'
                        | '；'
                        | '：'
                        | '、'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '（'
                        | '）'
                        | '【'
                        | '】'
                        | '《'
                        | '》'
                )
        })
        .map(str::trim)
        .find(|segment| !segment.is_empty())
        .unwrap_or_default();
    if segment.is_empty() || segment.is_ascii() {
        return Vec::new();
    }
    let chars = segment.chars().collect::<Vec<_>>();
    if chars.len() < 3 || chars.len() > 16 {
        return Vec::new();
    }

    let mut stems = Vec::new();
    push_unique_alias_candidate(&mut stems, chars.iter().collect::<String>());
    if chars.len() >= 4 {
        push_unique_alias_candidate(&mut stems, chars[..chars.len() - 1].iter().collect());
    }

    stems
        .into_iter()
        .filter(|candidate| {
            let len = candidate.chars().count();
            (3..=16).contains(&len)
        })
        .collect()
}

fn push_unique_alias_candidate(out: &mut Vec<String>, candidate: String) {
    let candidate = candidate.trim();
    if candidate.is_empty()
        || out
            .iter()
            .any(|existing| existing.eq_ignore_ascii_case(candidate))
    {
        return;
    }
    out.push(candidate.to_string());
}

pub(crate) fn structural_alias_binding_from_prompt(
    prompt: &str,
    route_result: &crate::IntentOutputContract,
    resolved_prompt_for_execution: &str,
) -> Option<SessionAliasBinding> {
    if route_result.requires_content_evidence {
        return None;
    }
    let alias = single_structural_quoted_alias(prompt)?;
    let target = single_structural_locator_target([
        prompt,
        resolved_prompt_for_execution,
        "",
        route_result.locator_hint.as_str(),
    ])?;
    Some(SessionAliasBinding {
        alias,
        target,
        updated_at_ts: crate::now_ts_u64(),
    })
}

fn single_structural_quoted_alias(text: &str) -> Option<String> {
    let mut candidates = Vec::new();
    for (open, close) in [('“', '”'), ('"', '"'), ('\'', '\''), ('`', '`')] {
        let mut inside = false;
        let mut start = 0usize;
        for (idx, ch) in text.char_indices() {
            if !inside && ch == open {
                inside = true;
                start = idx + ch.len_utf8();
                continue;
            }
            if inside && ch == close {
                if let Some(candidate) = text
                    .get(start..idx)
                    .map(str::trim)
                    .filter(|candidate| structural_alias_candidate_is_safe(candidate))
                {
                    candidates.push(candidate.to_string());
                }
                inside = false;
            }
        }
    }
    candidates.sort();
    candidates.dedup();
    (candidates.len() == 1).then(|| candidates.remove(0))
}

fn structural_alias_candidate_is_safe(candidate: &str) -> bool {
    let char_count = candidate.chars().count();
    if !(1..=80).contains(&char_count) {
        return false;
    }
    let surface = crate::intent::surface_signals::analyze_prompt_surface(candidate);
    !surface.has_concrete_locator_hint()
        && crate::intent::locator_extractor::extract_explicit_locator_for_fallback(candidate)
            .is_none()
}

fn single_structural_locator_target<'a>(
    sources: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    let mut targets = Vec::new();
    for source in sources {
        let Some(target) =
            crate::intent::locator_extractor::extract_explicit_locator_for_fallback(source)
                .map(|locator| locator.locator_hint)
                .and_then(|target| normalize_alias_target(&target))
        else {
            continue;
        };
        if !targets.iter().any(|existing| existing == &target) {
            targets.push(target);
        }
    }
    (targets.len() == 1).then(|| targets.remove(0))
}
