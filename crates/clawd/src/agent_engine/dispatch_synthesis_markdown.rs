use serde_json::Value;

pub(super) fn selected_markdown_title_from_read_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    if !read_value_requests_title_field_selector(&value) {
        return None;
    }
    scalar_string_field_from_read_value(&value, "field_value")
        .or_else(|| scalar_string_field_from_read_value(&value, "value_text"))
        .or_else(|| scalar_string_field_from_read_value(&value, "value"))
        .or_else(|| first_markdown_heading_from_read_value(&value))
}

pub(super) fn markdown_heading_from_read_output(output: &str) -> Option<String> {
    let value = serde_json::from_str::<Value>(output.trim()).ok()?;
    let text = markdown_text_from_read_value(&value)?;
    standalone_markdown_heading_from_text(&text)
}

pub(super) fn strip_markdown_read_line_prefix(line: &str) -> &str {
    let trimmed = line.trim();
    if let Some((prefix, rest)) = trimmed.split_once('|') {
        if !prefix.is_empty() && prefix.chars().all(|ch| ch.is_ascii_digit()) {
            return rest.trim();
        }
    }
    line
}

fn read_value_requests_title_field_selector(value: &Value) -> bool {
    value
        .get("field_selector")
        .and_then(Value::as_str)
        .map(str::trim)
        .is_some_and(|selector| selector == "title")
        || value
            .get("extra")
            .filter(|extra| extra.is_object())
            .is_some_and(read_value_requests_title_field_selector)
}

fn scalar_string_field_from_read_value(value: &Value, key: &str) -> Option<String> {
    if let Some(text) = value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        return Some(text.to_string());
    }
    value
        .get("extra")
        .filter(|extra| extra.is_object())
        .and_then(|extra| scalar_string_field_from_read_value(extra, key))
}

fn first_markdown_heading_from_read_value(value: &Value) -> Option<String> {
    let text = markdown_text_from_read_value(value)?;
    text.lines().find_map(markdown_heading_from_line)
}

fn markdown_text_from_read_value(value: &Value) -> Option<String> {
    let text = value
        .get("content")
        .or_else(|| value.get("excerpt"))
        .and_then(Value::as_str);
    if text.is_some() {
        return text.map(ToString::to_string);
    }
    value
        .get("extra")
        .filter(|extra| extra.is_object())
        .and_then(markdown_text_from_read_value)
}

fn standalone_markdown_heading_from_text(text: &str) -> Option<String> {
    let mut heading: Option<String> = None;
    for line in text.lines() {
        let stripped = strip_markdown_read_line_prefix(line).trim();
        if stripped.is_empty() {
            continue;
        }
        if let Some(candidate) = markdown_heading_from_line(stripped) {
            if heading.is_some() {
                return None;
            }
            heading = Some(candidate);
            continue;
        }
        if markdown_line_is_non_answer_separator_heading(stripped) {
            continue;
        }
        return None;
    }
    heading
}

fn markdown_heading_from_line(line: &str) -> Option<String> {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return None;
    }
    let rest = trimmed.get(hashes..)?.trim();
    (!rest.is_empty()).then(|| rest.to_string())
}

fn markdown_line_is_non_answer_separator_heading(line: &str) -> bool {
    let trimmed = strip_markdown_read_line_prefix(line).trim();
    let hashes = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&hashes) {
        return false;
    }
    trimmed.get(hashes..).map(str::trim).is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .chars()
                .all(|ch| matches!(ch, '=' | '-' | '_' | '*' | '#'))
    })
}
