fn normalize_run_cmd_command_for_fingerprint(command: &str) -> String {
    let tokens = command
        .split_whitespace()
        .map(normalize_command_token_for_fingerprint)
        .collect::<Vec<_>>();
    tokens.join(" ")
}

fn normalize_command_token_for_fingerprint(token: &str) -> String {
    if token.is_empty() {
        return String::new();
    }
    if token.starts_with('-') || token.contains('$') || token.contains('*') {
        return token.to_string();
    }
    if token.starts_with("./") || token.contains("/./") || token.contains("//") {
        return normalize_path_string_for_fingerprint(token);
    }
    token.to_string()
}

fn normalize_path_string_for_fingerprint(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    let mut quote_prefix = String::new();
    let mut quote_suffix = String::new();
    if (s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')) {
        quote_prefix = s[..1].to_string();
        quote_suffix = s[s.len() - 1..].to_string();
        s = s[1..s.len().saturating_sub(1)].to_string();
    }

    while s.starts_with("./") {
        s = s[2..].to_string();
    }
    while s.contains("//") {
        s = s.replace("//", "/");
    }
    s = s.replace("/./", "/");

    let path = Path::new(&s);
    let mut parts = Vec::new();
    let mut absolute = false;
    for comp in path.components() {
        match comp {
            Component::RootDir => absolute = true,
            Component::CurDir => {}
            Component::Normal(p) => parts.push(p.to_string_lossy().to_string()),
            Component::ParentDir => parts.push("..".to_string()),
            Component::Prefix(_) => {}
        }
    }
    let mut out = if absolute {
        format!("/{}", parts.join("/"))
    } else {
        parts.join("/")
    };
    if out.is_empty() {
        out = ".".to_string();
    }
    format!("{quote_prefix}{out}{quote_suffix}")
}

fn normalize_args_for_fingerprint(state: &AppState, action_name: &str, args: &Value) -> Value {
    if action_name == "subagent"
        && args.get("action").and_then(Value::as_str) == Some("inline_readonly")
    {
        return normalize_inline_subagent_args_for_fingerprint(state, args);
    }
    let mut out = args.clone();
    if action_name == "run_cmd" {
        if let Some(obj) = out.as_object_mut() {
            if let Some(cmd) = obj.get("command").and_then(|v| v.as_str()) {
                obj.insert(
                    "command".to_string(),
                    Value::String(normalize_run_cmd_command_for_fingerprint(cmd)),
                );
            }
            if let Some(cwd) = obj.get("cwd").and_then(|v| v.as_str()) {
                obj.insert(
                    "cwd".to_string(),
                    Value::String(normalize_path_string_for_fingerprint(cwd)),
                );
            }
        }
    }
    out
}

fn normalize_inline_subagent_args_for_fingerprint(state: &AppState, args: &Value) -> Value {
    let config = super::subagent_runtime::load_subagent_runtime_config(state);
    let role_family = args
        .get("role")
        .and_then(Value::as_str)
        .and_then(|token| config.resolve_role(token))
        .map(|role| role.family.as_str())
        .unwrap_or("unresolved");
    let mut context_refs = args
        .get("context_refs")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(normalize_path_string_for_fingerprint)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    context_refs.sort();
    context_refs.dedup();
    json!({
        "action": "inline_readonly",
        "role_family": role_family,
        "context_refs": context_refs,
    })
}

fn canonicalize_json_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut out = serde_json::Map::new();
            for key in keys {
                if let Some(v) = map.get(&key) {
                    out.insert(key, canonicalize_json_value(v));
                }
            }
            Value::Object(out)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(canonicalize_json_value).collect()),
        Value::Number(num) => canonicalize_json_number(num),
        _ => value.clone(),
    }
}

fn canonicalize_json_number(num: &serde_json::Number) -> Value {
    if num.is_i64() || num.is_u64() {
        return Value::Number(num.clone());
    }
    let Some(float_value) = num.as_f64() else {
        return Value::Number(num.clone());
    };
    if !float_value.is_finite() {
        return Value::Number(num.clone());
    }
    let rounded = float_value.round();
    if (float_value - rounded).abs() <= 1e-12 {
        if rounded >= 0.0 && rounded <= u64::MAX as f64 {
            return Value::Number(serde_json::Number::from(rounded as u64));
        }
        if rounded >= i64::MIN as f64 && rounded <= i64::MAX as f64 {
            return Value::Number(serde_json::Number::from(rounded as i64));
        }
    }
    Value::Number(num.clone())
}

fn canonical_json_string(value: &Value) -> String {
    serde_json::to_string(&canonicalize_json_value(value)).unwrap_or_else(|_| value.to_string())
}
