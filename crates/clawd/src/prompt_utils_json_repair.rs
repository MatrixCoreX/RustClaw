use serde::de::DeserializeOwned;

use serde_json::Value;

pub(crate) fn parse_llm_json_raw_or_any<T: DeserializeOwned>(raw: &str) -> Option<T> {
    serde_json::from_str::<T>(raw.trim()).ok().or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| serde_json::from_str::<T>(&s).ok())
    })
}

pub(crate) fn parse_llm_json_raw_or_any_with_repair<T: DeserializeOwned>(raw: &str) -> Option<T> {
    // F11: minimax / 部分模型偏好把 JSON plan 包在 ```json ... ``` 代码围栏里，
    // 围栏前/后还会带 prose。原生
    // `extract_first_json_object_any` 是 byte-level brace balancer，少量 raw
    // 在含中文宽括号 / 引号 / `\n` 转义 + `{{template}}` 占位时会过早终止，
    // 抓出的 candidate 不完整 → 解析后 step_count < 真实 step 数（典型现象：
    // plan 实际 4 步 [read, read, chat, respond] 被解析成只剩 [read, read]，
    // 后续 chat/respond 全丢，执行落入 observed_answer_fallback）。
    // 这里先用 codefence 显式提取一遍，命中则跳过 prose 干扰，保证 brace
    // balancer 从 envelope 第一个真正的 `{` 起走，否则回退原行为不破坏其它
    // 已经直接吐 JSON 的路径。
    if let Some(stripped) = strip_first_json_codefence(raw) {
        if let Some(value) = parse_json_with_repair::<T>(stripped.trim()) {
            return Some(value);
        }
        if let Some(value) =
            extract_first_json_object_any(&stripped).and_then(|s| parse_json_with_repair::<T>(&s))
        {
            return Some(value);
        }
    }
    parse_json_with_repair(raw.trim()).or_else(|| {
        extract_first_json_object_any(raw).and_then(|s| parse_json_with_repair::<T>(&s))
    })
}

/// 提取 raw 里第一个 ```json``` / ``` 代码围栏的内容；命中返回 fence 内文本，
/// 未命中返回 None。围栏类型容忍：` ```json `, ` ```JSON `, ` ``` ` 三种。
fn strip_first_json_codefence(raw: &str) -> Option<String> {
    let trimmed = raw.trim_start();
    // 找开 fence
    let fence_start = trimmed.find("```")?;
    let after_fence = &trimmed[fence_start + 3..];
    // 跳过可选语言标签 (json / JSON / 任意非换行串) + 一个换行
    let lang_end = after_fence.find('\n')?;
    let body_start = lang_end + 1;
    let body_and_rest = &after_fence[body_start..];
    // Prefer the first complete JSON value inside the fence. A planner may put
    // markdown fences inside a JSON string field such as `respond.content`; a
    // plain `find("```")` would treat that inner content as the closing fence
    // and truncate the plan.
    if let Some(json) = extract_first_json_value_any(body_and_rest) {
        return Some(json);
    }
    // 找闭 fence
    let close = body_and_rest.find("```")?;
    Some(body_and_rest[..close].to_string())
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

pub(crate) fn is_agent_action_type_token(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "think"
            | "call_tool"
            | "call_skill"
            | "call_capability"
            | "synthesize_answer"
            | "respond"
            | "run_cmd"
            | "reply"
            | "answer"
            | "final"
    )
}

fn is_agent_action_candidate(candidate: &str) -> bool {
    if let Ok(v) = serde_json::from_str::<Value>(candidate) {
        let Some(obj) = v.as_object() else {
            return false;
        };
        let has_args_shape = ["args", "arguments", "parameters", "input", "action_input"]
            .into_iter()
            .any(|key| obj.get(key).is_some());
        return obj.get("steps").and_then(Value::as_array).is_some()
            || ["type", "action_type"].into_iter().any(|key| {
                obj.get(key)
                    .and_then(Value::as_str)
                    .is_some_and(is_agent_action_type_token)
            })
            || obj
                .get("action")
                .and_then(Value::as_str)
                .is_some_and(|value| is_agent_action_type_token(value) || has_args_shape)
            || (["tool", "skill", "capability"]
                .into_iter()
                .any(|key| obj.get(key).is_some())
                && has_args_shape);
    }
    candidate.contains("\"type\"")
        || candidate.contains("\"action_type\"")
        || candidate.contains("\"action\"")
        || candidate.contains("\"steps\"")
        || ((candidate.contains("\"tool\"")
            || candidate.contains("\"skill\"")
            || candidate.contains("\"capability\""))
            && (candidate.contains("\"args\"")
                || candidate.contains("\"arguments\"")
                || candidate.contains("\"parameters\"")
                || candidate.contains("\"input\"")
                || candidate.contains("\"action_input\"")))
}

pub(crate) fn repair_invalid_json_escapes(raw: &str) -> String {
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

fn repair_stray_quote_after_primitive(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());
    let mut in_string = false;
    let mut escaped = false;
    let mut i = 0usize;

    while i < chars.len() {
        let ch = chars[i];
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            i += 1;
            continue;
        }

        if ch == '"' {
            let prev = out.chars().rev().find(|c| !c.is_whitespace());
            let next = chars
                .iter()
                .skip(i + 1)
                .copied()
                .find(|c| !c.is_whitespace());
            if prev.is_some_and(|c| c.is_ascii_alphanumeric())
                && next.is_some_and(|c| matches!(c, ',' | '}' | ']'))
            {
                i += 1;
                continue;
            }
            in_string = true;
        }

        out.push(ch);
        i += 1;
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
/// duplicate-field 错误，导致整个 JSON 解析失败。
///
/// 这里先把字符串 round-trip 一次：解析为 `Value` 时已经隐式去重，再
/// 序列化回字符串即可作为后续 struct deserialize 的喂入。
pub(crate) fn dedupe_json_object_keys(raw: &str) -> Option<String> {
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
            let repaired = repair_stray_quote_after_primitive(raw);
            serde_json::from_str::<T>(&repaired).ok()
        })
        .or_else(|| {
            let repaired = repair_stray_quote_after_primitive(&repair_invalid_json_escapes(raw));
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
        // 补齐截断 JSON 末尾未闭合的 `{`/`[`。部分 providers 会在 planner
        // action envelope 的最后漏掉闭合括号；恢复语法后仍由当前 schema 做
        // 字段和动作合同校验。
        .or_else(|| {
            let balanced = balance_unclosed_brackets(raw)?;
            serde_json::from_str::<T>(&balanced).ok()
        })
        .or_else(|| {
            let balanced = balance_unclosed_brackets(&repair_invalid_json_escapes(raw))?;
            serde_json::from_str::<T>(&balanced).ok()
        })
        .or_else(|| {
            let balanced = balance_unclosed_brackets(&repair_unescaped_inner_quotes(raw))?;
            serde_json::from_str::<T>(&balanced).ok()
        })
        .or_else(|| {
            let balanced = balance_unclosed_brackets(&repair_unescaped_inner_quotes(
                &repair_invalid_json_escapes(raw),
            ))?;
            serde_json::from_str::<T>(&balanced).ok()
        })
}

/// §F3-a：在 raw 末尾按未闭合栈顺序补齐 `]` / `}`。
///
/// 实现要点：
/// - 全程感知 JSON 字符串语法（含 `\\` / `\"` 等转义），不会把字面量里的
///   括号当成结构标记；
/// - 只追加，不删除任何字符，保持已有内容字节级稳定，避免破坏其它 repair
///   路径；
/// - 字符串里如果末尾仍未闭合，先补一个 `"` 再补结构括号；
/// - 如果一路扫到末尾 `stack` 已经空了（即已经是平衡 JSON），返回 None
///   表示「无需追加」，让上游继续走原路径，而不是返回一个完全相同的字符串
///   再做一次 `from_str` 浪费一次 CPU。
pub(crate) fn balance_unclosed_brackets(raw: &str) -> Option<String> {
    let trimmed = raw.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    let mut stack: Vec<u8> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;
    for &c in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if c == b'\\' {
                escaped = true;
            } else if c == b'"' {
                in_string = false;
            }
            continue;
        }
        match c {
            b'"' => in_string = true,
            b'{' => stack.push(b'}'),
            b'[' => stack.push(b']'),
            b'}' | b']' => {
                if stack.last() == Some(&c) {
                    stack.pop();
                }
            }
            _ => {}
        }
    }
    if !in_string && stack.is_empty() {
        return None;
    }
    let mut out = trimmed.to_string();
    if in_string {
        out.push('"');
    }
    while let Some(closer) = stack.pop() {
        out.push(closer as char);
    }
    Some(out)
}
