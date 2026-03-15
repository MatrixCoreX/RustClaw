use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

use crate::hard_rules::loader::read_toml_text;
use crate::hard_rules::types::VoiceModeIntentAliases;

pub const VOICE_MODE_INTENT_CONFIDENCE_THRESHOLD: f64 = 0.55;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoiceModeIntentDecision {
    pub mode: &'static str,
    pub confidence: Option<f64>,
    pub parser_path: &'static str,
}

fn parse_alias_list(value: &TomlValue, key: &str, fallback: &[String]) -> Vec<String> {
    value
        .get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|s| s.trim().to_ascii_lowercase())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| fallback.to_vec())
}

fn contains_any_alias(normalized: &str, aliases: &[String]) -> bool {
    aliases.iter().any(|x| normalized.contains(x))
}

fn parse_mode_token(text: &str) -> Option<&'static str> {
    match text.trim() {
        "voice" => Some("voice"),
        "text" => Some("text"),
        "both" => Some("both"),
        "reset" => Some("reset"),
        "show" => Some("show"),
        "none" => Some("none"),
        _ => None,
    }
}

fn parse_json_mode_and_confidence(raw: &str) -> Option<(&'static str, Option<f64>)> {
    let parse_from_json_value = |v: &JsonValue| {
        let mode = v
            .get("mode")
            .and_then(|x| x.as_str())
            .and_then(parse_mode_token)?;
        let confidence = v
            .get("confidence")
            .and_then(|x| x.as_f64())
            .map(|c| c.clamp(0.0, 1.0));
        Some((mode, confidence))
    };

    if let Ok(v) = serde_json::from_str::<JsonValue>(raw) {
        if let Some(out) = parse_from_json_value(&v) {
            return Some(out);
        }
    }
    if let (Some(start), Some(end)) = (raw.find('{'), raw.rfind('}')) {
        if start < end {
            let part = &raw[start..=end];
            if let Ok(v) = serde_json::from_str::<JsonValue>(part) {
                if let Some(out) = parse_from_json_value(&v) {
                    return Some(out);
                }
            }
        }
    }
    None
}

fn parse_voice_mode_intent_label_fallback(
    normalized: &str,
    aliases: &VoiceModeIntentAliases,
) -> Option<&'static str> {
    for token in ["voice", "text", "both", "reset", "show", "none"] {
        if normalized == token {
            return Some(token);
        }
    }

    let first = normalized
        .split(|c: char| !c.is_ascii_alphabetic())
        .find(|p| !p.is_empty())
        .unwrap_or("");
    match first {
        "voice" => Some("voice"),
        "text" => Some("text"),
        "both" => Some("both"),
        "reset" => Some("reset"),
        "show" => Some("show"),
        "none" => Some("none"),
        _ => {
            if contains_any_alias(normalized, &aliases.none) {
                return Some("none");
            }
            if contains_any_alias(normalized, &aliases.reset) {
                return Some("reset");
            }
            if contains_any_alias(normalized, &aliases.show) {
                return Some("show");
            }
            if contains_any_alias(normalized, &aliases.both) {
                return Some("both");
            }
            if contains_any_alias(normalized, &aliases.voice) {
                return Some("voice");
            }
            if contains_any_alias(normalized, &aliases.text) {
                return Some("text");
            }
            if normalized.contains("voice") || normalized.contains("语音") {
                return Some("voice");
            }
            if normalized.contains("text")
                || normalized.contains("文字")
                || normalized.contains("文本")
                || normalized.contains("打字")
            {
                return Some("text");
            }
            None
        }
    }
}

pub fn load_voice_mode_intent_aliases(path: &str) -> VoiceModeIntentAliases {
    let defaults = VoiceModeIntentAliases::defaults();
    let Some(raw) = read_toml_text(path) else {
        return defaults;
    };
    let Ok(value) = toml::from_str::<TomlValue>(&raw) else {
        return defaults;
    };

    VoiceModeIntentAliases {
        voice: parse_alias_list(&value, "voice_aliases", &defaults.voice),
        text: parse_alias_list(&value, "text_aliases", &defaults.text),
        both: parse_alias_list(&value, "both_aliases", &defaults.both),
        reset: parse_alias_list(&value, "reset_aliases", &defaults.reset),
        show: parse_alias_list(&value, "show_aliases", &defaults.show),
        none: parse_alias_list(&value, "none_aliases", &defaults.none),
    }
}

pub fn parse_voice_mode_intent_decision(
    raw: &str,
    aliases: &VoiceModeIntentAliases,
) -> Option<VoiceModeIntentDecision> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }

    if let Some((mode, confidence)) = parse_json_mode_and_confidence(&normalized) {
        if let Some(score) = confidence {
            if score >= VOICE_MODE_INTENT_CONFIDENCE_THRESHOLD {
                return Some(VoiceModeIntentDecision {
                    mode,
                    confidence: Some(score),
                    parser_path: "strict_json",
                });
            }
        }
    }

    parse_voice_mode_intent_label_fallback(&normalized, aliases).map(|mode| {
        VoiceModeIntentDecision {
            mode,
            confidence: None,
            parser_path: "fallback",
        }
    })
}

pub fn parse_voice_mode_intent_label(
    raw: &str,
    aliases: &VoiceModeIntentAliases,
) -> Option<&'static str> {
    parse_voice_mode_intent_decision(raw, aliases).map(|d| d.mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_alias_and_keyword_fallback() {
        let aliases = VoiceModeIntentAliases::defaults();
        assert_eq!(
            parse_voice_mode_intent_label("请切到语音回复", &aliases),
            Some("voice")
        );
        assert_eq!(
            parse_voice_mode_intent_label("just text please", &aliases),
            Some("text")
        );
    }

    #[test]
    fn prefers_strict_json_when_confident() {
        let aliases = VoiceModeIntentAliases::defaults();
        let out = parse_voice_mode_intent_decision(
            r#"{"mode":"text","confidence":0.96,"reason":"explicit switch"}"#,
            &aliases,
        )
        .expect("decision");
        assert_eq!(out.mode, "text");
        assert_eq!(out.parser_path, "strict_json");
    }

    #[test]
    fn low_confidence_json_falls_back() {
        let aliases = VoiceModeIntentAliases::defaults();
        let out = parse_voice_mode_intent_decision(
            r#"{"mode":"voice","confidence":0.20,"reason":"uncertain"}"#,
            &aliases,
        )
        .expect("decision");
        assert_eq!(out.mode, "voice");
        assert_eq!(out.parser_path, "fallback");
        assert_eq!(out.confidence, None);
    }

    #[test]
    fn invalid_json_mode_uses_fallback_from_text() {
        let aliases = VoiceModeIntentAliases::defaults();
        let out = parse_voice_mode_intent_decision(
            r#"{"mode":"chat","confidence":0.99} 切回文字回复"#,
            &aliases,
        )
        .expect("decision");
        assert_eq!(out.mode, "text");
        assert_eq!(out.parser_path, "fallback");
    }

    #[test]
    fn malformed_output_without_signal_returns_none() {
        let aliases = VoiceModeIntentAliases::defaults();
        let out = parse_voice_mode_intent_decision("n/a ???", &aliases);
        assert_eq!(out, None);
    }
}
