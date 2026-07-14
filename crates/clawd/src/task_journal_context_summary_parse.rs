use serde_json::Value;

pub(super) fn json_value_after_assignment(summary: Option<&str>, key: &str) -> Option<Value> {
    let summary = summary?.trim();
    let (_, tail) = summary.split_once(key)?;
    parse_first_json_value(tail.trim_start())
}

fn parse_first_json_value(text: &str) -> Option<Value> {
    let (start, _) = text
        .char_indices()
        .find(|(_, ch)| matches!(ch, '{' | '['))?;
    let mut stack = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for (offset, ch) in text[start..].char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => stack.push('}'),
            '[' => stack.push(']'),
            '}' | ']' => {
                if stack.pop() != Some(ch) {
                    return None;
                }
                if stack.is_empty() {
                    let end = start + offset + ch.len_utf8();
                    return serde_json::from_str(&text[start..end]).ok();
                }
            }
            _ => {}
        }
    }
    None
}
