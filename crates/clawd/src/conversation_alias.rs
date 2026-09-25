use serde_json::Value;

use super::{SessionAliasBinding, MAX_SESSION_ALIAS_BINDINGS};

fn normalize_alias_target(raw_target: &str) -> Option<String> {
    let trimmed = raw_target
        .trim()
        .trim_matches(|ch: char| matches!(ch, '"' | '\'' | '`' | '“' | '”' | '‘' | '’'))
        .trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
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

pub(super) fn merge_alias_bindings_from_capability_results(
    mut alias_bindings: Vec<SessionAliasBinding>,
    results: &[claw_core::capability_result::CapabilityResultEnvelope],
) -> Vec<SessionAliasBinding> {
    let now_ts = crate::now_ts_u64();
    for result in results {
        if result.status != claw_core::capability_result::CapabilityResultStatus::Ok
            || result.capability != "session.bind_alias"
            || result.action.as_deref() != Some("bind_session_alias")
            || result
                .data
                .pointer("/extra/execution_binding/skill_name")
                .and_then(Value::as_str)
                != Some("task_control")
        {
            continue;
        }
        let Some(items) = result
            .data
            .pointer("/extra/session_alias_bindings")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for item in items {
            let Some(alias) = item
                .get("alias")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty() && value.chars().count() <= 256)
            else {
                continue;
            };
            let Some(target) = item
                .get("target")
                .and_then(Value::as_str)
                .filter(|value| value.chars().count() <= 4096)
                .and_then(normalize_alias_target)
            else {
                continue;
            };
            let Some(target_kind) = item
                .get("target_kind")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|kind| session_alias_target_shape_is_valid(&target, kind))
            else {
                continue;
            };
            if alias == target || target_kind.is_empty() {
                continue;
            }
            let normalized_alias = normalized_alias_surface_for_match(alias);
            alias_bindings.retain(|existing| {
                normalized_alias_surface_for_match(&existing.alias) != normalized_alias
            });
            alias_bindings.push(SessionAliasBinding {
                alias: alias.to_string(),
                target,
                updated_at_ts: now_ts,
            });
        }
    }
    if alias_bindings.len() > MAX_SESSION_ALIAS_BINDINGS {
        let start = alias_bindings.len() - MAX_SESSION_ALIAS_BINDINGS;
        alias_bindings = alias_bindings.split_off(start);
    }
    alias_bindings
}

pub(crate) fn session_alias_target_shape_is_valid(target: &str, target_kind: &str) -> bool {
    match target_kind {
        "path" => {
            !target.contains(['\n', '\r', '\0'])
                && (target.contains('/')
                    || target.contains('\\')
                    || std::path::Path::new(target).extension().is_some())
        }
        "url" => target.starts_with("https://") || target.starts_with("http://"),
        "task" => uuid::Uuid::parse_str(target).is_ok(),
        "artifact" => claw_core::capability_result::task_artifact_reference_owner(target).is_some(),
        "resource" => target
            .split_once(':')
            .is_some_and(|(namespace, identifier)| {
                !namespace.is_empty()
                    && !identifier.is_empty()
                    && namespace
                        .chars()
                        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
                    && !target.chars().any(char::is_whitespace)
            }),
        _ => false,
    }
}
