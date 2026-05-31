use serde_json::Value;

use crate::{AppState, ClaimedTask};

pub(crate) fn inject_skill_memory_context(
    state: &AppState,
    task: &ClaimedTask,
    skill_name: &str,
    args: Value,
) -> Value {
    if !state.policy.memory.skill_memory_enabled {
        return args;
    }
    let mut obj = match args {
        Value::Object(map) => map,
        other => return other,
    };
    if obj.contains_key("_memory") {
        return Value::Object(obj);
    }
    let anchor = skill_memory_anchor(skill_name, &obj);
    let decision = crate::memory::use_policy::decide_skill_memory_use_policy(state, skill_name);
    if matches!(
        decision.profile,
        crate::memory::use_policy::MemoryUseProfile::Disabled
    ) {
        return Value::Object(obj);
    }
    let recent_limit = if decision.needs_recent_recall() {
        state.policy.memory.recall_limit.max(1)
    } else {
        0
    };
    let structured = crate::memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        &anchor,
        recent_limit,
        decision.include_long_term_summary,
        decision.include_preferences,
    );
    let structured =
        crate::memory::use_policy::filter_structured_memory_context(structured, &decision);
    let memory_context = crate::memory::service::structured_memory_context_block(
        &structured,
        decision.mode,
        decision.max_chars,
    );
    let mut pref_map = serde_json::Map::new();
    for (k, v) in &structured.preferences {
        pref_map.insert(k.clone(), Value::String(v.clone()));
    }
    let knowledge_docs = structured
        .knowledge_docs
        .iter()
        .map(|item| {
            serde_json::json!({
                "text": item.text,
                "score": item.score,
                "source_label": item.source_label,
            })
        })
        .collect::<Vec<_>>();
    let lang_hint = skill_memory_language_hint(state, &obj);
    obj.insert(
        "_memory".to_string(),
        serde_json::json!({
            "context": memory_context,
            "long_term_summary": structured.long_term_summary.clone().unwrap_or_default(),
            "preferences": Value::Object(pref_map),
            "knowledge_docs": knowledge_docs,
            "lang_hint": lang_hint,
            "use_policy": {
                "profile": decision.profile.as_str(),
                "reason": decision.reason,
            }
        }),
    );
    Value::Object(obj)
}

fn skill_memory_language_hint(
    state: &AppState,
    args_obj: &serde_json::Map<String, Value>,
) -> String {
    for key in [
        "text",
        "query",
        "instruction",
        "goal",
        "prompt",
        "message",
        "content",
    ] {
        let Some(trimmed) = args_obj
            .get(key)
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            continue;
        };
        let hint = crate::language_policy::preferred_response_language_hint(trimmed, None);
        if hint != "config_default" {
            return hint;
        }
    }
    state.policy.command_intent.default_locale.clone()
}

fn skill_memory_anchor(skill_name: &str, args_obj: &serde_json::Map<String, Value>) -> String {
    let mut parts = vec![format!("skill={skill_name}")];
    for key in [
        "text",
        "query",
        "instruction",
        "goal",
        "prompt",
        "message",
        "content",
        "action",
    ] {
        if let Some(val) = args_obj.get(key).and_then(|v| v.as_str()) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_string());
            }
        }
    }
    parts.join(" | ")
}

#[cfg(test)]
#[path = "memory_context_tests.rs"]
mod tests;
