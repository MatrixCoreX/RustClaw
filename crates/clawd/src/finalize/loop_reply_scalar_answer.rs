pub(super) fn scalar_answer_from_json(value: &serde_json::Value) -> Option<String> {
    if let Some(answer) = value.get("extra").and_then(scalar_answer_from_json) {
        return Some(answer);
    }
    for key in ["value_text", "value", "count", "total"] {
        let Some(child) = value.get(key) else {
            continue;
        };
        if let Some(raw) = child.as_str() {
            let text = raw.trim();
            if !text.is_empty() {
                return Some(text.to_string());
            }
            if key == "value" {
                return serde_json::to_string(raw).ok();
            }
        }
        if child.is_number() || child.is_boolean() {
            return Some(child.to_string());
        }
    }
    None
}

#[cfg(test)]
#[path = "loop_reply_scalar_answer_tests.rs"]
mod tests;
