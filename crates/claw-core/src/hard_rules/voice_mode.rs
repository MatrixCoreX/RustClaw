use serde_json::Value as JsonValue;
use std::sync::OnceLock;

pub const VOICE_MODE_INTENT_CONFIDENCE_THRESHOLD: f64 = 0.55;
const VOICE_MODE_INTENT_SCHEMA_RAW: &str =
    include_str!("../../../../prompts/schemas/voice_mode_intent.schema.json");

static VOICE_MODE_INTENT_SCHEMA: OnceLock<JsonValue> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceModeIntentDecision {
    pub mode: &'static str,
    pub confidence: Option<f64>,
    pub parser_path: &'static str,
}

fn voice_mode_intent_schema() -> &'static JsonValue {
    VOICE_MODE_INTENT_SCHEMA.get_or_init(|| {
        serde_json::from_str::<JsonValue>(VOICE_MODE_INTENT_SCHEMA_RAW)
            .expect("voice_mode_intent schema must be valid JSON")
    })
}

fn parse_mode_token(text: &str) -> Option<&'static str> {
    match text.trim().to_ascii_lowercase().as_str() {
        "voice" => Some("voice"),
        "text" => Some("text"),
        "both" => Some("both"),
        "reset" => Some("reset"),
        "show" => Some("show"),
        "none" => Some("none"),
        _ => None,
    }
}

fn schema_property<'a>(schema: &'a JsonValue, name: &str) -> Option<&'a JsonValue> {
    schema.get("properties")?.get(name)
}

fn schema_declared_fields(schema: &JsonValue) -> Option<&serde_json::Map<String, JsonValue>> {
    schema.get("properties")?.as_object()
}

fn schema_requires_field(schema: &JsonValue, name: &str) -> bool {
    schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|fields| fields.iter().any(|field| field.as_str() == Some(name)))
        .unwrap_or(false)
}

fn schema_allows_additional_properties(schema: &JsonValue) -> bool {
    schema
        .get("additionalProperties")
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

fn schema_enum_contains(schema: &JsonValue, name: &str, candidate: &str) -> bool {
    schema_property(schema, name)
        .and_then(|property| property.get("enum"))
        .and_then(|v| v.as_array())
        .map(|values| values.iter().any(|value| value.as_str() == Some(candidate)))
        .unwrap_or(false)
}

fn schema_number_in_range(schema: &JsonValue, name: &str, value: f64) -> bool {
    let property = match schema_property(schema, name) {
        Some(property) => property,
        None => return false,
    };
    let minimum = property
        .get("minimum")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::NEG_INFINITY);
    let maximum = property
        .get("maximum")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::INFINITY);
    value >= minimum && value <= maximum
}

fn schema_string_is_valid(schema: &JsonValue, name: &str, value: &str) -> bool {
    let property = match schema_property(schema, name) {
        Some(property) => property,
        None => return false,
    };
    if property.get("type").and_then(|v| v.as_str()) != Some("string") {
        return false;
    }
    let min_length = property
        .get("minLength")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    !value.trim().is_empty() && value.chars().count() >= min_length
}

fn parse_from_json_value(
    value: &JsonValue,
    parser_path: &'static str,
) -> Option<VoiceModeIntentDecision> {
    let schema = voice_mode_intent_schema();
    let object = value.as_object()?;
    if !schema_allows_additional_properties(schema) {
        let declared_fields = schema_declared_fields(schema)?;
        if object.keys().any(|key| !declared_fields.contains_key(key)) {
            return None;
        }
    }
    for required in ["mode", "confidence", "reason"] {
        if schema_requires_field(schema, required) && !object.contains_key(required) {
            return None;
        }
    }
    let mode_raw = object.get("mode")?.as_str()?;
    if !schema_enum_contains(schema, "mode", mode_raw) {
        return None;
    }
    let mode = parse_mode_token(mode_raw)?;
    let confidence = object.get("confidence")?.as_f64()?;
    if !schema_number_in_range(schema, "confidence", confidence) {
        return None;
    }
    let reason = object.get("reason")?.as_str()?;
    if !schema_string_is_valid(schema, "reason", reason) {
        return None;
    }
    Some(VoiceModeIntentDecision {
        mode,
        confidence: Some(confidence),
        parser_path,
    })
}

fn parse_json_mode_and_confidence(raw: &str) -> Option<VoiceModeIntentDecision> {
    if let Ok(v) = serde_json::from_str::<JsonValue>(raw) {
        if let Some(out) = parse_from_json_value(&v, "strict_json") {
            return Some(out);
        }
    }
    if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) {
        if start < end {
            let part = &raw[start..=end];
            if let Ok(v) = serde_json::from_str::<JsonValue>(part) {
                if let Some(out) = parse_from_json_value(&v, "extracted_json") {
                    return Some(out);
                }
            }
        }
    }
    None
}

pub fn parse_voice_mode_intent_decision(raw: &str) -> Option<VoiceModeIntentDecision> {
    let normalized = raw.trim();
    if normalized.is_empty() {
        return None;
    }

    let decision = parse_json_mode_and_confidence(normalized)?;
    let score = decision.confidence?;
    if score < VOICE_MODE_INTENT_CONFIDENCE_THRESHOLD {
        return None;
    }
    Some(decision)
}

pub fn parse_voice_mode_intent_label(raw: &str) -> Option<&'static str> {
    parse_voice_mode_intent_decision(raw).map(|d| d.mode)
}

#[cfg(test)]
#[path = "voice_mode_tests.rs"]
mod tests;
