use super::*;

pub(super) fn detect_format_from_path(path: &Path) -> String {
    match path
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => "json",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        _ => "json",
    }
    .to_string()
}

pub(super) fn parse_structured_root(
    path: &Path,
    format_hint: Option<&str>,
) -> SkillResult<(String, Value)> {
    let format = format_hint
        .map(|v| v.to_ascii_lowercase())
        .unwrap_or_else(|| detect_format_from_path(path));
    let meta = std::fs::metadata(path).map_err(|err| SkillError::io("metadata", path, err))?;
    if meta.is_dir() {
        return Err(SkillError::is_directory(format!(
            "structured document parsing requires a file, but target is a directory: {}",
            path.display()
        )));
    }
    let raw =
        std::fs::read_to_string(path).map_err(|err| SkillError::io("read_file", path, err))?;
    let root_value = match format.as_str() {
        "json" => serde_json::from_str::<Value>(&raw)
            .map_err(|err| SkillError::invalid_data(format!("json parse failed: {err}")))?,
        "toml" => {
            let value = raw
                .parse::<toml::Value>()
                .map_err(|err| SkillError::invalid_data(format!("toml parse failed: {err}")))?;
            serde_json::to_value(value)
                .map_err(|err| SkillError::invalid_data(format!("toml convert failed: {err}")))?
        }
        "yaml" | "yml" => serde_yaml::from_str::<Value>(&raw)
            .map_err(|err| SkillError::invalid_data(format!("yaml parse failed: {err}")))?,
        other => {
            return Err(SkillError::invalid_input(format!(
                "unsupported format: {other}; use json|toml|yaml"
            )));
        }
    };
    Ok((format, root_value))
}

pub(super) fn collect_dir_signatures(
    root: &Path,
    include_hidden: bool,
    recursive: bool,
    max_entries: usize,
) -> SkillResult<std::collections::BTreeMap<String, String>> {
    let mut out = std::collections::BTreeMap::new();
    walk_inventory(root, recursive, &mut |entry_path, meta, depth| {
        if depth == 0 {
            return Ok(false);
        }
        let rel = entry_path
            .strip_prefix(root)
            .unwrap_or(entry_path)
            .to_string_lossy()
            .to_string();
        let name = entry_path.file_name().and_then(OsStr::to_str).unwrap_or("");
        if !include_hidden && name.starts_with('.') {
            return Ok(meta.is_dir() && recursive);
        }
        if out.len() < max_entries {
            out.insert(rel, path_kind(meta).to_string());
        }
        Ok(meta.is_dir() && recursive)
    })?;
    Ok(out)
}

pub(super) fn lookup_field_value<'a>(value: &'a Value, field_path: &str) -> Option<&'a Value> {
    let mut current = value;
    for seg in split_field_path(field_path)? {
        if seg.is_empty() {
            return None;
        }
        current = lookup_field_segment(current, seg)?;
    }
    Some(current)
}

pub(super) struct FieldLookup<'a> {
    pub(super) value: Option<&'a Value>,
    pub(super) resolved_field_path: Option<String>,
    pub(super) match_strategy: &'static str,
    pub(super) match_count: usize,
}

pub(super) fn lookup_field_value_with_resolution<'a>(
    value: &'a Value,
    field_path: &str,
) -> FieldLookup<'a> {
    if let Some(found) = lookup_field_value(value, field_path) {
        return FieldLookup {
            value: Some(found),
            resolved_field_path: Some(field_path.to_string()),
            match_strategy: "exact_path",
            match_count: 1,
        };
    }

    if let Some(found) = lookup_array_item_key_path(value, field_path) {
        return found;
    }

    if let Some(found) = lookup_array_item_identity(value, field_path) {
        return found;
    }

    if let Some(found) = lookup_parent_scoped_suffix_field(value, field_path) {
        return found;
    }

    if let Some(found) = lookup_missing_parent_leaf_suffix_field(value, field_path) {
        return found;
    }

    if let Some(found) = lookup_json_schema_properties_path(value, field_path) {
        return found;
    }

    let Some(key) = bare_field_key_selector(field_path) else {
        return FieldLookup {
            value: None,
            resolved_field_path: None,
            match_strategy: "exact_path",
            match_count: 0,
        };
    };

    let mut matches = Vec::new();
    collect_bare_key_matches(value, key, "", &mut matches);
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "unique_bare_key",
            match_count: 1,
        };
    }

    if matches.is_empty() {
        collect_bare_key_suffix_matches(value, key, "", &mut matches);
        if matches.len() == 1 {
            let (resolved_field_path, found) = matches.remove(0);
            return FieldLookup {
                value: Some(found),
                resolved_field_path: Some(resolved_field_path),
                match_strategy: "unique_bare_key_suffix",
                match_count: 1,
            };
        }
    }

    FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "unique_bare_key",
        match_count: matches.len(),
    }
}

pub(super) fn bare_field_key_selector(field_path: &str) -> Option<&str> {
    let key = field_path.trim();
    if key.is_empty()
        || key
            .chars()
            .any(|ch| ch == '.' || ch == '[' || ch == ']' || ch.is_whitespace())
    {
        return None;
    }
    Some(key)
}

pub(super) fn collect_bare_key_matches<'a>(
    value: &'a Value,
    target_key: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                if key == target_key {
                    out.push((child_path.clone(), child));
                }
                collect_bare_key_matches(child, target_key, &child_path, out);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let child_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                collect_bare_key_matches(child, target_key, &child_path, out);
            }
        }
        _ => {}
    }
}

pub(super) fn collect_bare_key_suffix_matches<'a>(
    value: &'a Value,
    target_key: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                if bare_key_suffix_matches(key, target_key) && is_safe_suffix_field_value(child) {
                    out.push((child_path.clone(), child));
                }
                collect_bare_key_suffix_matches(child, target_key, &child_path, out);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let child_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                collect_bare_key_suffix_matches(child, target_key, &child_path, out);
            }
        }
        _ => {}
    }
}

pub(super) fn is_safe_suffix_field_value(value: &Value) -> bool {
    matches!(
        value,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

pub(super) fn bare_key_suffix_matches(key: &str, target_key: &str) -> bool {
    let key = key.trim();
    let target_key = target_key.trim();
    if target_key.len() < 3 || key.eq_ignore_ascii_case(target_key) {
        return false;
    }
    let key_lower = key.to_ascii_lowercase();
    let target_lower = target_key.to_ascii_lowercase();
    let Some(prefix) = key_lower.strip_suffix(&target_lower) else {
        return false;
    };
    prefix.ends_with(['_', '-'])
}

pub(super) fn lookup_parent_scoped_suffix_field<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<FieldLookup<'a>> {
    let segments = split_field_path(field_path)?;
    if segments.len() < 2 {
        return None;
    }
    let leaf = segments.last()?.trim();
    let target_key = bare_field_key_selector(leaf)?;
    let parent_path = segments[..segments.len() - 1].join(".");
    if parent_path.trim().is_empty() {
        return None;
    }
    let parent = lookup_field_value(value, &parent_path)?;
    let mut matches = Vec::new();
    collect_bare_key_suffix_matches(parent, target_key, &parent_path, &mut matches);
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "parent_scoped_key_suffix",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "parent_scoped_key_suffix",
        match_count: matches.len(),
    })
}

pub(super) fn lookup_missing_parent_leaf_suffix_field<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<FieldLookup<'a>> {
    let segments = split_field_path(field_path)?;
    if segments.len() < 2 {
        return None;
    }
    let leaf = segments.last()?.trim();
    let target_key = bare_field_key_selector(leaf)?;
    let parent_path = segments[..segments.len() - 1].join(".");
    if parent_path.trim().is_empty() || lookup_field_value(value, &parent_path).is_some() {
        return None;
    }

    let mut matches = Vec::new();
    collect_bare_key_suffix_matches(value, target_key, "", &mut matches);
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "missing_parent_leaf_key_suffix",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "missing_parent_leaf_key_suffix",
        match_count: matches.len(),
    })
}

pub(super) fn lookup_json_schema_properties_path<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<FieldLookup<'a>> {
    let segments = split_field_path(field_path)?;
    if segments.len() < 3 || segments.first()? != &"properties" {
        return None;
    }
    let property_key = segments.get(1)?.trim();
    if property_key.is_empty() {
        return None;
    }
    let nested_field_path = segments[2..].join(".");
    if nested_field_path.trim().is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    collect_json_schema_properties_path_matches(
        value,
        property_key,
        &nested_field_path,
        "",
        &mut matches,
    );
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "json_schema_properties_path",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "json_schema_properties_path",
        match_count: matches.len(),
    })
}

pub(super) fn collect_json_schema_properties_path_matches<'a>(
    value: &'a Value,
    property_key: &str,
    nested_field_path: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            if let Some(properties) = map.get("properties").and_then(Value::as_object) {
                if let Some(property_schema) = properties.get(property_key) {
                    if let Some(nested_value) =
                        lookup_field_value(property_schema, nested_field_path)
                    {
                        let base_path = if current_path.is_empty() {
                            format!("properties.{property_key}")
                        } else {
                            format!("{current_path}.properties.{property_key}")
                        };
                        out.push((format!("{base_path}.{nested_field_path}"), nested_value));
                    }
                }
            }
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                collect_json_schema_properties_path_matches(
                    child,
                    property_key,
                    nested_field_path,
                    &child_path,
                    out,
                );
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let child_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                collect_json_schema_properties_path_matches(
                    child,
                    property_key,
                    nested_field_path,
                    &child_path,
                    out,
                );
            }
        }
        _ => {}
    }
}

pub(super) fn lookup_array_item_key_path<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<FieldLookup<'a>> {
    let segments = split_field_path(field_path)?;
    if segments.len() < 2 {
        return None;
    }
    let selector_value = segments[0].trim();
    if selector_value.is_empty() || selector_value.contains('[') || selector_value.contains(']') {
        return None;
    }
    let nested_field_path = segments[1..].join(".");
    if nested_field_path.trim().is_empty() {
        return None;
    }

    let mut matches = Vec::new();
    collect_array_item_key_path_matches(
        value,
        selector_value,
        &nested_field_path,
        "",
        &mut matches,
    );
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "array_item_key_path",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "array_item_key_path",
        match_count: matches.len(),
    })
}

pub(super) fn lookup_array_item_identity<'a>(
    value: &'a Value,
    field_path: &str,
) -> Option<FieldLookup<'a>> {
    let selector_value = bare_field_key_selector(field_path)?;
    let mut matches = Vec::new();
    collect_array_item_identity_matches(value, selector_value, "", &mut matches);
    if matches.len() == 1 {
        let (resolved_field_path, found) = matches.remove(0);
        return Some(FieldLookup {
            value: Some(found),
            resolved_field_path: Some(resolved_field_path),
            match_strategy: "array_item_identity",
            match_count: 1,
        });
    }
    (!matches.is_empty()).then_some(FieldLookup {
        value: None,
        resolved_field_path: None,
        match_strategy: "array_item_identity",
        match_count: matches.len(),
    })
}

pub(super) fn collect_array_item_key_path_matches<'a>(
    value: &'a Value,
    selector_value: &str,
    nested_field_path: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                collect_array_item_key_path_matches(
                    child,
                    selector_value,
                    nested_field_path,
                    &child_path,
                    out,
                );
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let item_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                if let Some((selector_key, nested_value)) =
                    array_item_key_path_match(child, selector_value, nested_field_path)
                {
                    let resolved_path = format!(
                        "{current_path}[{selector_key}={selector_value}].{nested_field_path}"
                    );
                    out.push((resolved_path, nested_value));
                }
                collect_array_item_key_path_matches(
                    child,
                    selector_value,
                    nested_field_path,
                    &item_path,
                    out,
                );
            }
        }
        _ => {}
    }
}

pub(super) fn collect_array_item_identity_matches<'a>(
    value: &'a Value,
    selector_value: &str,
    current_path: &str,
    out: &mut Vec<(String, &'a Value)>,
) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = if current_path.is_empty() {
                    key.clone()
                } else {
                    format!("{current_path}.{key}")
                };
                collect_array_item_identity_matches(child, selector_value, &child_path, out);
            }
        }
        Value::Array(items) => {
            for (idx, child) in items.iter().enumerate() {
                let item_path = if current_path.is_empty() {
                    format!("[{idx}]")
                } else {
                    format!("{current_path}[{idx}]")
                };
                if let Some(selector_key) = array_item_identity_match(child, selector_value) {
                    let resolved_path = if current_path.is_empty() {
                        format!("[{selector_key}={selector_value}]")
                    } else {
                        format!("{current_path}[{selector_key}={selector_value}]")
                    };
                    out.push((resolved_path, child));
                }
                collect_array_item_identity_matches(child, selector_value, &item_path, out);
            }
        }
        _ => {}
    }
}

pub(super) fn array_item_key_path_match<'a>(
    item: &'a Value,
    selector_value: &str,
    nested_field_path: &str,
) -> Option<(&'static str, &'a Value)> {
    let map = item.as_object()?;
    for selector_key in ["name", "id", "key"] {
        if map
            .get(selector_key)
            .and_then(Value::as_str)
            .is_some_and(|value| value == selector_value)
        {
            let nested_value = lookup_field_value(item, nested_field_path)?;
            return Some((selector_key, nested_value));
        }
    }
    None
}

pub(super) fn array_item_identity_match(
    item: &Value,
    selector_value: &str,
) -> Option<&'static str> {
    let map = item.as_object()?;
    for selector_key in ["name", "id", "key"] {
        if map
            .get(selector_key)
            .and_then(Value::as_str)
            .is_some_and(|value| value == selector_value)
        {
            return Some(selector_key);
        }
    }
    None
}

pub(super) fn split_field_path(field_path: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut bracket_depth = 0usize;
    let mut quote: Option<char> = None;
    for (idx, ch) in field_path.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.checked_sub(1)?,
            '.' if bracket_depth == 0 => {
                out.push(&field_path[start..idx]);
                start = idx + ch.len_utf8();
            }
            _ => {}
        }
    }
    if quote.is_some() || bracket_depth != 0 {
        return None;
    }
    out.push(&field_path[start..]);
    Some(out)
}

pub(super) fn lookup_field_segment<'a>(mut current: &'a Value, segment: &str) -> Option<&'a Value> {
    if let Ok(idx) = segment.parse::<usize>() {
        return current.as_array()?.get(idx);
    }

    let Some(first_bracket) = segment.find('[') else {
        return current.get(segment);
    };
    let key = &segment[..first_bracket];
    if !key.is_empty() {
        current = current.get(key)?;
    }

    let mut rest = &segment[first_bracket..];
    while !rest.is_empty() {
        let inner_start = rest.strip_prefix('[')?;
        let end = find_selector_end(inner_start)?;
        let selector = &inner_start[..end];
        current = lookup_field_selector(current, selector)?;
        rest = &inner_start[end + 1..];
    }
    Some(current)
}

pub(super) fn find_selector_end(selector_and_tail: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (idx, ch) in selector_and_tail.char_indices() {
        if let Some(active_quote) = quote {
            if ch == active_quote {
                quote = None;
            }
            continue;
        }
        match ch {
            '\'' | '"' => quote = Some(ch),
            ']' => return Some(idx),
            _ => {}
        }
    }
    None
}

pub(super) fn lookup_field_selector<'a>(value: &'a Value, selector: &str) -> Option<&'a Value> {
    let selector = selector.trim();
    if let Ok(idx) = selector.parse::<usize>() {
        return value.as_array()?.get(idx);
    }
    let condition = selector
        .strip_prefix("?(")
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(selector);
    let (field_path, expected) = parse_field_filter_condition(condition)?;
    value.as_array()?.iter().find(|item| {
        lookup_field_value(item, field_path)
            .is_some_and(|found| json_value_matches_text(found, expected))
    })
}

pub(super) fn parse_field_filter_condition(condition: &str) -> Option<(&str, &str)> {
    let (left, right) = condition
        .split_once("==")
        .or_else(|| condition.split_once('='))?;
    let left = left.trim();
    let field_path = left
        .strip_prefix("@.")
        .or_else(|| left.strip_prefix('@'))
        .unwrap_or(left)
        .trim();
    if field_path.is_empty() {
        return None;
    }
    let expected = strip_matching_quotes(right.trim())?;
    Some((field_path, expected))
}

pub(super) fn strip_matching_quotes(value: &str) -> Option<&str> {
    if value.len() >= 2 {
        let bytes = value.as_bytes();
        let first = bytes[0];
        let last = bytes[value.len() - 1];
        if (first == b'\'' && last == b'\'') || (first == b'"' && last == b'"') {
            return Some(&value[1..value.len() - 1]);
        }
    }
    Some(value)
}

pub(super) fn json_value_matches_text(value: &Value, expected: &str) -> bool {
    match value {
        Value::String(text) => text == expected,
        Value::Bool(flag) => flag.to_string() == expected,
        Value::Number(number) => number.to_string() == expected,
        Value::Null => expected.eq_ignore_ascii_case("null"),
        Value::Array(_) | Value::Object(_) => json_value_to_text(value) == expected,
    }
}

pub(super) fn json_value_type(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

pub(super) fn json_value_to_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(v).unwrap_or_default(),
    }
}
