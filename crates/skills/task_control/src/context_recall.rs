use serde_json::Value;

pub(super) fn decode_response(
    success: bool,
    api: super::ApiResponse,
) -> Result<super::SkillOutput, String> {
    if success && api.ok {
        let value = api.data.ok_or("context_recall_response_invalid")?;
        return Ok(super::SkillOutput::structured(value.to_string(), value));
    }
    let code = api
        .error
        .filter(|code| {
            !code.is_empty()
                && code.len() <= 128
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        })
        .unwrap_or_else(|| "context_recall_api_failed".into());
    let mut extra = super::error_extra(&code);
    if matches!(
        code.as_str(),
        "context_cursor_invalid" | "context_cursor_expired" | "context_snapshot_changed"
    ) {
        extra["retryable"] = Value::Bool(true);
        extra["side_effect_applied"] = Value::Bool(false);
        extra["failure_phase"] = Value::String("pre_dispatch".into());
        extra["recovery_action"] = Value::String("replan_arguments".into());
        extra["recovery_context"] = serde_json::json!({
            "operation":"rediscover_context_index",
            "discard_argument_fields":["cursor", "source_digest"],
            "preserve_argument_fields":["context_ref"],
            "field_paths":["/capabilities"],
            "source_task_reexecution_allowed":false,
            "historical_instruction_authority":"none",
        });
    }
    Ok(super::SkillOutput {
        text: String::new(),
        extra: Some(extra),
        error_text: Some(code),
    })
}

#[derive(Debug, Default)]
pub(super) struct Options {
    pub cursor: Option<String>,
    pub page_size: Option<u64>,
    pub source_digest: Option<String>,
}

impl Options {
    pub(super) fn parse(args: &Value) -> Result<Self, String> {
        let string = |name, limit| -> Result<Option<String>, String> {
            match args.get(name) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(value)) if !value.is_empty() && value.len() <= limit => {
                    Ok(Some(value.clone()))
                }
                _ => Err("context_page_invalid".into()),
            }
        };
        let cursor = string("cursor", 1024)?;
        let source_digest = string("source_digest", 64)?;
        if source_digest
            .as_ref()
            .is_some_and(|value| value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err("context_snapshot_invalid".into());
        }
        let page_size = match args.get("page_size") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_u64()
                    .filter(|size| (1..=64).contains(size))
                    .ok_or("context_page_invalid")?,
            ),
        };
        if (cursor.is_some() || page_size.is_some())
            && !args
                .get("field_paths")
                .and_then(Value::as_array)
                .is_some_and(|paths| paths.iter().any(|path| path == "/capabilities"))
        {
            return Err("context_page_invalid".into());
        }
        Ok(Self {
            cursor,
            page_size,
            source_digest,
        })
    }
}

#[cfg(test)]
#[path = "context_recall_tests.rs"]
mod tests;
