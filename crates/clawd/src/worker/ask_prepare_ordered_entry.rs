use serde_json::Value;

fn json_usize(value: &Value) -> Option<usize> {
    value
        .as_u64()
        .and_then(|raw| usize::try_from(raw).ok())
        .or_else(|| value.as_i64().and_then(|raw| usize::try_from(raw).ok()))
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
}

fn ordered_entry_index_from_state_patch(
    state_patch: Option<&Value>,
    frame: &crate::followup_frame::FollowupFrame,
) -> Option<usize> {
    let len = frame.ordered_entries.len();
    if len == 0 {
        return None;
    }
    let reference = state_patch?
        .get("ordered_entry_ref")
        .or_else(|| state_patch?.get("ordered_entry_reference"))?;
    let reference = reference.as_object()?;
    if let Some(index_value) = reference.get("index") {
        let index = json_usize(index_value)?;
        let index_base = reference
            .get("index_base")
            .and_then(json_usize)
            .unwrap_or(1);
        let zero_based_index = index.checked_sub(index_base)?;
        return (zero_based_index < len).then_some(zero_based_index);
    }

    let offset = reference
        .get("relative_offset")
        .or_else(|| reference.get("offset_from_selected"))
        .and_then(json_i64)?;
    let selected = i64::try_from(frame.selected_entry_index?).ok()?;
    let target = selected.checked_add(offset)?;
    usize::try_from(target).ok().filter(|index| *index < len)
}

fn ordered_entry_state_patch(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> Option<&Value> {
    turn_analysis.and_then(|analysis| analysis.state_patch.as_ref())
}

fn ordered_entry_reference_from_active_frame_index(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    index: usize,
) -> bool {
    let Some(frame) = session_snapshot.active_followup_frame.as_ref() else {
        return false;
    };
    let Some(target) = crate::followup_frame::ordered_entry_target_at(frame, index) else {
        return false;
    };
    if target.trim().is_empty() {
        return false;
    }
    route_result.output_contract.locator_kind = crate::OutputLocatorKind::Path;
    route_result.output_contract.locator_hint = target.clone();
    if route_result.route_reason.trim().is_empty() {
        route_result.route_reason = "ordered_entry_reference_bound_from_active_frame".to_string();
    } else if !route_result
        .route_reason
        .contains("ordered_entry_reference_bound_from_active_frame")
    {
        route_result
            .route_reason
            .push_str("; ordered_entry_reference_bound_from_active_frame");
    }
    if route_result.resolved_intent.trim().is_empty() {
        route_result.resolved_intent = format!("Use ordered entry {}: {target}", index + 1);
    } else if !route_result.resolved_intent.contains(&target) {
        route_result
            .resolved_intent
            .push_str(&format!("\nordered_entry_target: {target}"));
    }
    true
}

fn normalize_ordered_entry_path_token(token: &str) -> String {
    let mut normalized = token
        .trim()
        .trim_matches('`')
        .trim_matches('"')
        .trim_matches('\'')
        .trim_matches(|ch: char| matches!(ch, ',' | '，' | ';' | '；' | '。' | ')' | ']' | '}'))
        .trim_start_matches("FILE:")
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string();
    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }
    normalized
}

fn push_unique_ordered_entry_path_token(candidates: &mut Vec<String>, token: String) {
    if !candidates
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&token))
    {
        candidates.push(token);
    }
}

fn ordered_entry_path_token_candidates(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    for token in text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，' | ';' | '；' | '。' | '(' | ')' | '[' | ']' | '{' | '}' | '"' | '\''
            )
    }) {
        let token = normalize_ordered_entry_path_token(token);
        if token.is_empty() || token.contains("{{") || token.contains("}}") {
            continue;
        }
        let path = std::path::Path::new(&token);
        let has_path_shape = path.components().count() > 1 || token.contains('/');
        let has_file_extension = path
            .file_name()
            .and_then(|name| name.to_str())
            .and_then(|name| std::path::Path::new(name).extension())
            .is_some();
        if has_path_shape || has_file_extension {
            push_unique_ordered_entry_path_token(&mut candidates, token);
        }
    }
    for filename in crate::delivery_utils::extract_filename_candidates(text) {
        let filename = normalize_ordered_entry_path_token(&filename);
        if !filename.is_empty() {
            push_unique_ordered_entry_path_token(&mut candidates, filename);
        }
    }
    candidates
}

fn ordered_entry_path_token_matches_target(token: &str, target: &str) -> bool {
    let token = normalize_ordered_entry_path_token(token);
    let target = normalize_ordered_entry_path_token(target);
    if token.is_empty() || target.is_empty() {
        return false;
    }
    if token.eq_ignore_ascii_case(&target) {
        return true;
    }
    let token_lower = token.to_ascii_lowercase();
    let target_lower = target.to_ascii_lowercase();
    if token_lower.ends_with(&format!("/{target_lower}"))
        || target_lower.ends_with(&format!("/{token_lower}"))
    {
        return true;
    }
    let token_base = std::path::Path::new(&token)
        .file_name()
        .and_then(|name| name.to_str());
    let target_base = std::path::Path::new(&target)
        .file_name()
        .and_then(|name| name.to_str());
    matches!((token_base, target_base), (Some(left), Some(right)) if left.eq_ignore_ascii_case(right))
}

fn ordered_entry_index_from_route_path_tokens(
    route_result: &crate::RouteResult,
    frame: &crate::followup_frame::FollowupFrame,
) -> Option<usize> {
    for source in [&route_result.resolved_intent, &route_result.route_reason] {
        if let Some(index) =
            ordered_entry_index_from_tokens(ordered_entry_path_token_candidates(source), frame)
        {
            return Some(index);
        }
    }
    None
}

fn ordered_entry_plain_token_candidates(text: &str) -> Vec<String> {
    let mut candidates = ordered_entry_path_token_candidates(text);
    for token in text.split(|ch: char| {
        ch.is_whitespace()
            || matches!(
                ch,
                ',' | '，'
                    | ';'
                    | '；'
                    | '。'
                    | '('
                    | ')'
                    | '['
                    | ']'
                    | '{'
                    | '}'
                    | '"'
                    | '\''
                    | '`'
            )
    }) {
        let token = normalize_ordered_entry_path_token(token);
        if token.is_empty()
            || token.len() > 128
            || token.contains("{{")
            || token.contains("}}")
            || token.contains(char::is_whitespace)
            || token.chars().any(char::is_control)
        {
            continue;
        }
        if token.chars().all(|ch| {
            ch.is_alphanumeric()
                || matches!(
                    ch,
                    '.' | '_'
                        | '-'
                        | '/'
                        | '\\'
                        | '~'
                        | '@'
                        | '+'
                        | '='
                        | '['
                        | ']'
                        | '('
                        | ')'
                        | '%'
                        | ':'
                )
        }) {
            push_unique_ordered_entry_path_token(&mut candidates, token);
        }
    }
    candidates
}

fn ordered_entry_index_from_tokens(
    tokens: Vec<String>,
    frame: &crate::followup_frame::FollowupFrame,
) -> Option<usize> {
    let mut matches = Vec::new();
    for token in tokens {
        for index in 0..frame.ordered_entries.len() {
            let Some(target) = crate::followup_frame::ordered_entry_target_at(frame, index) else {
                continue;
            };
            if ordered_entry_path_token_matches_target(&token, &target) && !matches.contains(&index)
            {
                matches.push(index);
            }
        }
    }
    (matches.len() == 1).then(|| matches.remove(0))
}

fn ordered_entry_index_from_current_prompt_token(
    current_prompt: Option<&str>,
    frame: &crate::followup_frame::FollowupFrame,
) -> Option<usize> {
    ordered_entry_index_from_tokens(ordered_entry_plain_token_candidates(current_prompt?), frame)
}

pub(super) fn bind_ordered_entry_reference_from_active_frame(
    route_result: &mut crate::RouteResult,
    session_snapshot: &crate::conversation_state::ActiveSessionSnapshot,
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
    current_prompt: Option<&str>,
) -> bool {
    let supported_ordered_entry_contract = route_result.output_contract.requires_content_evidence
        || route_result.output_contract.delivery_required
        || (route_result.output_contract.response_shape == crate::OutputResponseShape::Scalar
            && route_result.output_contract.semantic_kind
                == crate::OutputSemanticKind::ScalarPathOnly);
    if route_result.needs_clarify
        || !route_result.is_execute_gate()
        || !supported_ordered_entry_contract
    {
        return false;
    }
    let Some(frame) = session_snapshot.active_followup_frame.as_ref() else {
        return false;
    };
    let state_patch_index =
        ordered_entry_index_from_state_patch(ordered_entry_state_patch(turn_analysis), frame);
    let route_path_index = ordered_entry_index_from_route_path_tokens(route_result, frame);
    let current_prompt_index = ordered_entry_index_from_current_prompt_token(current_prompt, frame);
    let Some(index) = route_path_index
        .or(state_patch_index)
        .or(current_prompt_index)
    else {
        return false;
    };
    if let Some(state_patch_index) = state_patch_index {
        if route_path_index.is_some_and(|route_path_index| route_path_index != state_patch_index)
            && !route_result
                .route_reason
                .contains("ordered_entry_reference_index_repaired_from_route_path")
        {
            route_result
                .route_reason
                .push_str("; ordered_entry_reference_index_repaired_from_route_path");
        }
    } else if current_prompt_index.is_some()
        && !route_result
            .route_reason
            .contains("ordered_entry_reference_inferred_from_current_prompt_token")
    {
        route_result
            .route_reason
            .push_str("; ordered_entry_reference_inferred_from_current_prompt_token");
    }
    ordered_entry_reference_from_active_frame_index(route_result, session_snapshot, index)
}

pub(super) fn has_ordered_entry_state_patch(
    turn_analysis: Option<&crate::intent_router::TurnAnalysis>,
) -> bool {
    ordered_entry_state_patch(turn_analysis).is_some()
}
