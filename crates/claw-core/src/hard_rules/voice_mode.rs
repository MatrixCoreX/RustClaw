use serde_json::Value as JsonValue;
use toml::Value as TomlValue;

use crate::hard_rules::loader::read_toml_text;
use crate::hard_rules::types::VoiceModeIntentAliases;

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

pub fn parse_voice_mode_intent_label(raw: &str, aliases: &VoiceModeIntentAliases) -> Option<&'static str> {
    let normalized = raw.trim().to_ascii_lowercase();
    if normalized.is_empty() {
        return None;
    }
    if let Ok(v) = serde_json::from_str::<JsonValue>(&normalized) {
        if let Some(mode) = v.get("mode").and_then(|x| x.as_str()) {
            return parse_voice_mode_intent_label(mode, aliases);
        }
    }
    if let (Some(start), Some(end)) = (normalized.find('{'), normalized.rfind('}')) {
        if start < end {
            let part = &normalized[start..=end];
            if let Ok(v) = serde_json::from_str::<JsonValue>(part) {
                if let Some(mode) = v.get("mode").and_then(|x| x.as_str()) {
                    return parse_voice_mode_intent_label(mode, aliases);
                }
            }
        }
    }
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
            if contains_any_alias(&normalized, &aliases.none) {
                return Some("none");
            }
            if contains_any_alias(&normalized, &aliases.reset) {
                return Some("reset");
            }
            if contains_any_alias(&normalized, &aliases.show) {
                return Some("show");
            }
            if contains_any_alias(&normalized, &aliases.both) {
                return Some("both");
            }
            if contains_any_alias(&normalized, &aliases.voice) {
                return Some("voice");
            }
            if contains_any_alias(&normalized, &aliases.text) {
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
}
