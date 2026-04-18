use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::AppState;

pub(crate) fn render_prompt_template(template: &str, replacements: &[(&str, &str)]) -> String {
    let mut rendered = template.to_string();
    for (key, value) in replacements {
        rendered = rendered.replace(key, value);
    }
    rendered
}

pub(crate) fn log_prompt_render(
    state: &AppState,
    task_id: &str,
    prompt_name: &str,
    prompt_source: &str,
    round: Option<usize>,
) {
    log_prompt_render_with_version(state, task_id, prompt_name, prompt_source, None, round);
}

/// §3.5a: 带 prompt_version 字段的版本，给已迁移到 with_meta API 的关键审计点用。
///
/// `prompt_version` 缺失时记 `prompt_version=none`；存在时记 `prompt_version=...`。
/// 该字段会进 model_io / task_journal payload，为 prompt 改动追溯提供锚点。
pub(crate) fn log_prompt_render_with_version(
    state: &AppState,
    task_id: &str,
    prompt_name: &str,
    prompt_source: &str,
    prompt_version: Option<&str>,
    round: Option<usize>,
) {
    if !state.policy.routing.debug_log_prompt {
        return;
    }
    let version = prompt_version.unwrap_or("none");
    match round {
        Some(round) => {
            tracing::info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_source={} prompt_version={} prompt_dynamic=true note=dynamic_built_prompt round={}",
                crate::highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_source,
                version,
                round
            );
        }
        None => {
            tracing::info!(
                "{} prompt_invocation task_id={} prompt_name={} prompt_source={} prompt_version={} prompt_dynamic=true note=dynamic_built_prompt",
                crate::highlight_tag("prompt"),
                task_id,
                prompt_name,
                prompt_source,
                version
            );
        }
    }
}

pub(crate) fn parse_llm_json_extract_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    extract_json_object(raw)
        .or_else(|| extract_first_json_object_any(raw))
        .and_then(|s| serde_json::from_str::<T>(&s).ok())
}

pub(crate) fn parse_llm_json_raw_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str::<T>(raw.trim()).ok().or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| serde_json::from_str::<T>(&s).ok())
    })
}

pub(crate) fn parse_llm_json_raw_or_any_with_repair<T: DeserializeOwned>(raw: &str) -> Option<T> {
    parse_json_with_repair(raw.trim()).or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| parse_json_with_repair::<T>(&s))
    })
}

pub(crate) fn extract_first_json_value_any(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let opener = bytes[i];
        if opener != b'{' && opener != b'[' {
            i += 1;
            continue;
        }
        let start = i;
        let mut stack = vec![opener];
        let mut in_string = false;
        let mut escaped = false;
        let mut j = i + 1;
        while j < bytes.len() {
            let c = bytes[j];
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == b'\\' {
                    escaped = true;
                } else if c == b'"' {
                    in_string = false;
                }
                j += 1;
                continue;
            }
            match c {
                b'"' => in_string = true,
                b'{' | b'[' => stack.push(c),
                b'}' | b']' => {
                    let Some(last) = stack.pop() else {
                        break;
                    };
                    let matched = matches!((last, c), (b'{', b'}') | (b'[', b']'));
                    if !matched {
                        break;
                    }
                    if stack.is_empty() {
                        let candidate = &text[start..=j];
                        if serde_json::from_str::<Value>(candidate).is_ok() {
                            return Some(candidate.to_string());
                        }
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        i = start + 1;
    }
    None
}

pub(crate) fn extract_first_json_object_any(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;
            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else if c == b'"' {
                    in_string = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return Some(text[start..=j].to_string());
                    }
                }
                j += 1;
            }
            i = j;
        }
        i += 1;
    }
    None
}

pub(crate) fn extract_json_object(text: &str) -> Option<String> {
    extract_agent_action_objects(text).into_iter().next()
}

pub(crate) fn extract_agent_action_objects(text: &str) -> Vec<String> {
    fn push_candidate_if_action(out: &mut Vec<String>, candidate: String) {
        if is_agent_action_candidate(&candidate) {
            out.push(candidate);
        }
    }

    let bytes = text.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' {
            let start = i;
            let mut depth = 0usize;
            let mut in_string = false;
            let mut escaped = false;
            let mut j = i;
            let mut closed = false;

            while j < bytes.len() {
                let c = bytes[j];
                if in_string {
                    if escaped {
                        escaped = false;
                    } else if c == b'\\' {
                        escaped = true;
                    } else if c == b'"' {
                        in_string = false;
                    }
                } else if c == b'"' {
                    in_string = true;
                } else if c == b'{' {
                    depth += 1;
                } else if c == b'}' {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        closed = true;
                        push_candidate_if_action(&mut out, text[start..=j].to_string());
                        break;
                    }
                } else if c == b']' && depth == 1 {
                    // Recover the trailing inner object when a wrapper array closes before the
                    // final action object emitted its own `}`.
                    let mut repaired = text[start..j].to_string();
                    repaired.push('}');
                    if serde_json::from_str::<Value>(&repaired).is_ok() {
                        closed = true;
                        push_candidate_if_action(&mut out, repaired);
                        break;
                    }
                }
                j += 1;
            }
            if closed {
                i = j;
            } else {
                i = start;
            }
        }
        i += 1;
    }
    out
}

pub(crate) fn parse_agent_action_json_with_repair(
    raw: &str,
    state: &AppState,
) -> Result<Value, String> {
    let parsed = match serde_json::from_str::<Value>(raw) {
        Ok(v) => Ok(v),
        Err(first_err) => {
            let repaired = repair_invalid_json_escapes(raw);
            match serde_json::from_str::<Value>(&repaired) {
                Ok(v) => Ok(v),
                Err(second_err) => Err(format!(
                    "initial parse error: {first_err}; repaired parse error: {second_err}"
                )),
            }
        }
    }?;
    Ok(normalize_agent_action_shape(parsed, state))
}

fn is_agent_action_candidate(candidate: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<Value>(candidate) {
        return v.get("type").is_some()
            || v.get("action").is_some()
            || v.get("tool").is_some()
            || v.get("skill").is_some();
    }
    candidate.contains("\"type\"")
        || candidate.contains("\"action\"")
        || candidate.contains("\"tool\"")
        || candidate.contains("\"skill\"")
}

fn repair_invalid_json_escapes(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;

    for ch in raw.chars() {
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            continue;
        }

        if escaped {
            let valid = matches!(ch, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't' | 'u');
            if !valid {
                out.push('\\');
            }
            out.push(ch);
            escaped = false;
            continue;
        }

        match ch {
            '\\' => {
                out.push(ch);
                escaped = true;
            }
            '"' => {
                out.push(ch);
                in_string = false;
            }
            _ => out.push(ch),
        }
    }

    out
}

fn repair_unescaped_inner_quotes(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len() + 16);
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        if !in_string {
            if ch == '"' {
                in_string = true;
            }
            out.push(ch);
            i += 1;
            continue;
        }

        if escaped {
            out.push(ch);
            escaped = false;
            i += 1;
            continue;
        }

        match ch {
            '\\' => {
                out.push(ch);
                escaped = true;
            }
            '"' => {
                let mut j = i + 1;
                while j < chars.len() && chars[j].is_whitespace() {
                    j += 1;
                }
                let looks_like_string_end =
                    j >= chars.len() || matches!(chars[j], ',' | '}' | ']' | ':');
                if looks_like_string_end {
                    out.push(ch);
                    in_string = false;
                } else {
                    out.push('\\');
                    out.push('"');
                }
            }
            _ => out.push(ch),
        }
        i += 1;
    }

    out
}

/// 把任意 JSON 文本里的对象重复键去重为「last-wins」。
///
/// 背景：minimax 这类模型偶尔会输出包含重复键的 JSON（例如
/// `{"target_scope":"system","target_scope":"system"}`）。serde_json 自身把
/// `Value::Object` 实现为 BTreeMap/Map（last-wins，不会报错），但
/// `serde::Deserialize` 派生的 struct 反序列化时会触发
/// `Error("duplicate field ...")`，导致整个 JSON 解析失败。
///
/// 这里先把字符串 round-trip 一次：解析为 `Value` 时已经隐式去重，再
/// 序列化回字符串即可作为后续 struct deserialize 的喂入。
fn dedupe_json_object_keys(raw: &str) -> Option<String> {
    let value: Value = serde_json::from_str(raw).ok()?;
    serde_json::to_string(&value).ok()
}

fn parse_json_with_repair<T: DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str::<T>(raw)
        .ok()
        .or_else(|| {
            let repaired = repair_invalid_json_escapes(raw);
            serde_json::from_str::<T>(&repaired).ok()
        })
        .or_else(|| {
            let repaired = repair_unescaped_inner_quotes(raw);
            serde_json::from_str::<T>(&repaired).ok()
        })
        .or_else(|| {
            let repaired = repair_unescaped_inner_quotes(&repair_invalid_json_escapes(raw));
            serde_json::from_str::<T>(&repaired).ok()
        })
        // 最后再尝试一次「对象重复键去重」回退路径：
        // 处理 minimax / 部分模型偶发输出 `{"x":1,"x":1}` 之类 duplicate-field
        // 的合法 JSON 但派生 Deserialize 失败的 case（详见 dedupe_json_object_keys 注释）。
        // 仍然套一层 escape/quote repair，覆盖「重复键 + 转义异常」的复合场景。
        .or_else(|| {
            let deduped = dedupe_json_object_keys(raw)?;
            serde_json::from_str::<T>(&deduped).ok()
        })
        .or_else(|| {
            let deduped = dedupe_json_object_keys(&repair_invalid_json_escapes(raw))?;
            serde_json::from_str::<T>(&deduped).ok()
        })
        .or_else(|| {
            let deduped =
                dedupe_json_object_keys(&repair_unescaped_inner_quotes(raw)).or_else(|| {
                    dedupe_json_object_keys(&repair_unescaped_inner_quotes(
                        &repair_invalid_json_escapes(raw),
                    ))
                })?;
            serde_json::from_str::<T>(&deduped).ok()
        })
}

fn normalize_agent_action_shape(value: Value, state: &AppState) -> Value {
    let Some(obj) = value.as_object() else {
        return value;
    };
    let Some(raw_type) = obj.get("type").and_then(|v| v.as_str()) else {
        if let Some(skill) = obj.get("skill").and_then(|v| v.as_str()) {
            let normalized_skill = state.resolve_canonical_skill_name(skill.trim());
            if state.is_builtin_skill(&normalized_skill) {
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_skill,
                    "args": args,
                });
            }
        }
        if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
            let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
            let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
            return json!({
                "type": "call_skill",
                "skill": normalized_tool,
                "args": args,
            });
        }
        if let Some(content) = obj.get("content").and_then(|v| v.as_str()) {
            return json!({
                "type": "respond",
                "content": content,
            });
        }
        return value;
    };
    let step_type = raw_type.trim().to_ascii_lowercase();
    if matches!(
        step_type.as_str(),
        "think" | "call_tool" | "call_skill" | "respond"
    ) {
        if step_type == "call_tool" {
            if let Some(tool) = obj.get("tool").and_then(|v| v.as_str()) {
                let normalized_tool = state.resolve_canonical_skill_name(tool.trim());
                let args = obj.get("args").cloned().unwrap_or_else(|| json!({}));
                return json!({
                    "type": "call_skill",
                    "skill": normalized_tool,
                    "args": args,
                });
            }
        }
        return value;
    }

    let args = collect_bare_action_args(obj);
    if state.is_builtin_skill(&step_type) {
        return json!({
            "type": "call_skill",
            "skill": step_type,
            "args": args,
        });
    }

    let normalized_skill = state.resolve_canonical_skill_name(&step_type);
    if state.is_builtin_skill(&normalized_skill) {
        return json!({
            "type": "call_skill",
            "skill": normalized_skill,
            "args": args,
        });
    }

    value
}

fn collect_bare_action_args(obj: &serde_json::Map<String, Value>) -> Value {
    let mut args = obj
        .get("args")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    for (key, value) in obj {
        if matches!(key.as_str(), "type" | "args" | "tool" | "skill") {
            continue;
        }
        args.insert(key.clone(), value.clone());
    }
    Value::Object(args)
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_handles_unescaped_quotes() {
        let raw = r#"{"resolved_user_intent":"记住："那玩意README"指向 /home/guagua/test/README.md","reason":"用户定义了"那玩意README"映射","confidence":1.0}"#;
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("should parse repaired json");
        assert_eq!(
            parsed
                .get("resolved_user_intent")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "记住：\"那玩意README\"指向 /home/guagua/test/README.md"
        );
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_dedupes_object_keys_for_struct() {
        use serde::Deserialize;
        #[derive(Debug, Deserialize, PartialEq, Eq)]
        struct ExecutionRecipeProbe {
            kind: String,
            target_scope: String,
        }
        let raw = r#"{"kind":"none","target_scope":"system","target_scope":"system"}"#;
        // Sanity check: 直接 derive Deserialize 在 duplicate field 上会失败。
        assert!(serde_json::from_str::<ExecutionRecipeProbe>(raw).is_err());
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<ExecutionRecipeProbe>(raw)
            .expect("dedup pass should recover duplicate-key object");
        assert_eq!(
            parsed,
            ExecutionRecipeProbe {
                kind: "none".to_string(),
                target_scope: "system".to_string(),
            }
        );
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_dedupes_nested_duplicate_keys() {
        let raw = r#"{"mode":"chat_act","execution_recipe":{"kind":"none","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#;
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("nested duplicate keys should be repaired");
        assert_eq!(
            parsed
                .pointer("/execution_recipe/target_scope")
                .and_then(|v| v.as_str()),
            Some("system")
        );
        assert_eq!(parsed.get("mode").and_then(|v| v.as_str()), Some("chat_act"));
    }

    #[test]
    fn parse_llm_json_raw_or_any_with_repair_keeps_valid_json() {
        let raw = r#"{"mode":"chat","confidence":0.9}"#;
        let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
            .expect("valid json should parse");
        assert_eq!(
            parsed
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or_default(),
            "chat"
        );
    }

    /// §D1：dedupe_json_object_keys 幂等性。任意 JSON dedup 一次和二次结果必须一致。
    /// 防止未来引入「dedup 自身搬动了 key 顺序导致再 dedup 又改」这种回归。
    #[test]
    fn dedupe_json_object_keys_is_idempotent() {
        let corpus = [
            r#"{"a":1}"#,
            r#"{"a":1,"a":2}"#,
            r#"{"a":1,"a":2,"a":3,"a":4}"#,
            r#"{"a":{"b":1,"b":2},"a":{"b":3,"b":4}}"#,
            r#"[{"x":1,"x":2},{"x":3,"x":4}]"#,
            r#"{"mode":"chat_act","execution_recipe":{"kind":"none","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#,
            r#"{"a":[1,2,3],"a":[4,5,6]}"#,
            r#"{}"#,
            r#"[]"#,
            r#""hi""#,
            r#"42"#,
            r#"true"#,
            r#"null"#,
        ];
        for raw in corpus {
            let once =
                super::dedupe_json_object_keys(raw).expect("first dedup pass should succeed");
            let twice =
                super::dedupe_json_object_keys(&once).expect("second dedup pass should succeed");
            assert_eq!(
                once, twice,
                "dedupe_json_object_keys not idempotent on input {}",
                raw
            );
        }
    }

    /// §D1：N-fold 重复键 last-wins 规则覆盖。覆盖 minimax 偶发把同一字段
    /// 重复 2/3/5/10 次的全部观测形态。
    #[test]
    fn dedupe_json_object_keys_last_wins_for_n_fold_duplicates() {
        for n in [2usize, 3, 5, 10] {
            let mut payload = String::from("{");
            for i in 0..n {
                if i > 0 {
                    payload.push(',');
                }
                payload.push_str(&format!(r#""x":"v{}""#, i));
            }
            payload.push('}');
            let deduped = super::dedupe_json_object_keys(&payload)
                .expect("n-fold duplicate input should round-trip through Value");
            let parsed: Value = serde_json::from_str(&deduped)
                .expect("dedup output should still parse as Value");
            assert_eq!(
                parsed.get("x").and_then(|v| v.as_str()),
                Some(format!("v{}", n - 1).as_str()),
                "expected last-wins for n={}, got: {}",
                n,
                deduped
            );
        }
    }

    /// §D1：minimax 实际观测的「病态 JSON 语料库」全部能跑通解析回路 —— 含
    /// duplicate keys / 嵌套 duplicate / 数组里的 object-with-duplicates / 数值与
    /// bool 重复 / null 与字符串混合重复。任何一条 panic 都视为回归。
    ///
    /// 这里**不**断言每一条都能 dedup 成功；只断言不 panic 且能 round-trip：
    /// `parse_llm_json_raw_or_any_with_repair::<Value>(...)` 拿到结果后再 to_string
    /// 然后再 dedup 仍然能 parse。
    #[test]
    fn parse_llm_json_raw_or_any_with_repair_survives_minimax_pathological_corpus() {
        let corpus = [
            // duplicate top-level keys
            r#"{"target_scope":"system","target_scope":"system"}"#,
            // duplicate top + duplicate nested
            r#"{"a":"x","a":"y","b":{"c":1,"c":2,"c":3}}"#,
            // duplicate inside array element
            r#"{"items":[{"k":1,"k":2},{"k":3,"k":4,"k":5}]}"#,
            // duplicate boolean / null mixed
            r#"{"flag":true,"flag":false,"missing":null,"missing":"present"}"#,
            // duplicate keys with mixed value types (str -> obj)
            r#"{"contract":"loose","contract":{"shape":"strict"}}"#,
            // realistic minimax normalizer payload: duplicate target_scope inside
            // execution_recipe nested in IntentNormalizerOut-style envelope.
            r#"{"resolved_user_intent":"check service","mode":"chat_act","needs_clarify":false,"reason":"r","confidence":0.8,"execution_recipe":{"kind":"ops_closed_loop","profile":"ops_service","target_scope":"system","target_scope":"system"}}"#,
        ];
        for raw in corpus {
            let parsed = super::parse_llm_json_raw_or_any_with_repair::<Value>(raw)
                .unwrap_or_else(|| panic!("failed to repair-and-parse: {}", raw));
            let reserialized = serde_json::to_string(&parsed)
                .expect("repaired Value should re-serialize");
            let again = super::parse_llm_json_raw_or_any_with_repair::<Value>(&reserialized)
                .unwrap_or_else(|| panic!("re-parse of normalized form failed: {}", reserialized));
            assert!(again.is_object() || again.is_array() || again.is_string() || again.is_number() || again.is_boolean() || again.is_null());
        }
    }

    #[test]
    fn extract_agent_action_objects_recovers_inner_actions_from_malformed_wrapper() {
        let raw = r#"{"steps":[{"type":"call_skill","skill":"read_file","args":{"path":"README.md"}},{"type":"call_skill","skill":"chat","args":{"text":"summarize","style":"chat"}]}"#;
        let extracted = super::extract_agent_action_objects(raw);
        assert_eq!(extracted.len(), 2);
        let parsed: Value =
            serde_json::from_str(&extracted[0]).expect("first inner action should parse");
        assert_eq!(
            parsed.get("skill").and_then(|v| v.as_str()),
            Some("read_file")
        );
        let parsed_second: Value =
            serde_json::from_str(&extracted[1]).expect("second inner action should parse");
        assert_eq!(
            parsed_second.get("skill").and_then(|v| v.as_str()),
            Some("chat")
        );
    }
}
