use std::collections::HashSet;
use std::hash::{Hash, Hasher};

pub(super) fn dedupe_terminal_messages(items: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in items {
        if seen.insert(item.clone()) {
            out.push(item);
        }
    }
    out
}

pub(super) fn terminal_text_fingerprint_hex(text: &str) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

pub(super) fn terminal_text_preview_for_log(text: &str, max_chars: usize) -> String {
    let normalized = text
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if normalized.chars().count() <= max_chars {
        return normalized;
    }
    normalized.chars().take(max_chars).collect::<String>() + "...(truncated)"
}

pub(super) fn terminal_tts_text(answer: &str) -> String {
    claw_core::channel_delivery_tokens::strip_legacy_delivery_lines(answer)
        .lines()
        .filter(|line| !line.trim_start().starts_with("EPHEMERAL:"))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tts_projection_omits_local_and_remote_delivery_tokens() {
        assert_eq!(
            terminal_tts_text(
                "spoken\nIMAGE_FILE:/tmp/image.png\nVIDEO_URL:https://example.test/video.mp4"
            ),
            "spoken"
        );
    }
}
