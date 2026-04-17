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
    if crate::canonical_skill_name(skill_name) == "chat" {
        return Value::Object(obj);
    }
    if obj.contains_key("_memory") {
        return Value::Object(obj);
    }
    let anchor = skill_memory_anchor(skill_name, &obj);
    let structured = crate::memory::service::recall_structured_memory_context(
        state,
        task.user_key.as_deref(),
        task.user_id,
        task.chat_id,
        &anchor,
        state.policy.memory.recall_limit.max(1),
        true,
        true,
    );
    let memory_context = crate::memory::service::structured_memory_context_block(
        &structured,
        crate::memory::retrieval::MemoryContextMode::Skill,
        state.policy.memory.skill_memory_max_chars.max(384),
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
    let lang_hint = state.policy.command_intent.default_locale.clone();
    obj.insert(
        "_memory".to_string(),
        serde_json::json!({
            "context": memory_context,
            "long_term_summary": structured.long_term_summary.clone().unwrap_or_default(),
            "preferences": Value::Object(pref_map),
            "knowledge_docs": knowledge_docs,
            "lang_hint": lang_hint
        }),
    );
    Value::Object(obj)
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
