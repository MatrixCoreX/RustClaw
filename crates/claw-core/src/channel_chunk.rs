//! Shared text chunking for channel-safe message sending (Telegram, WhatsApp, etc.).
//! Used by clawd, telegramd, whatsappd so segment logic and prefix budget stay in one place.

/// Reserve for segment prefix "（i/N）\n" so that prefix + body does not exceed channel limit.
/// Callers should chunk with (channel_limit - SEGMENT_PREFIX_MAX_CHARS).
pub const SEGMENT_PREFIX_MAX_CHARS: usize = 16;

/// Split text into segments for channel-safe sending.
/// Prefer newline boundaries; then UTF-8 char boundary. Empty input -> [].
/// Each segment is trimmed; non-empty only. Does not add segment prefix; callers add "（i/N）\n" when n > 1.
pub fn chunk_text_for_channel(text: &str, max_chars: usize) -> Vec<String> {
    let s = text.trim();
    if s.is_empty() || max_chars == 0 {
        return vec![];
    }
    if s.len() <= max_chars {
        return vec![s.to_string()];
    }
    let mut out = Vec::new();
    let mut start = 0;
    while start < s.len() {
        let mut chunk_end = (start + max_chars).min(s.len());
        while chunk_end > start && !s.is_char_boundary(chunk_end) {
            chunk_end -= 1;
        }
        if chunk_end == start {
            chunk_end = (start + 1..=s.len())
                .find(|&i| s.is_char_boundary(i))
                .unwrap_or(s.len());
        }
        let window = &s[start..chunk_end];
        let segment_end = if let Some(rel) = window.rfind('\n') {
            start + rel + 1
        } else {
            chunk_end
        };
        let segment = s[start..segment_end].trim();
        if !segment.is_empty() {
            out.push(segment.to_string());
        }
        start = segment_end;
    }
    out
}
