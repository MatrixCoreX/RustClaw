pub(super) fn utf8_safe_prefix(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

pub(super) fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

pub(super) fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}
