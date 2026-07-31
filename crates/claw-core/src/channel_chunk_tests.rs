use super::*;

#[test]
fn chunks_by_unicode_scalar_count_instead_of_utf8_bytes() {
    let chunks = chunk_text_for_channel("甲乙丙丁戊己", 3);
    assert_eq!(chunks, vec!["甲乙丙", "丁戊己"]);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 3));
}

#[test]
fn never_splits_emoji_or_combined_utf8_encoding() {
    let chunks = chunk_text_for_channel("🙂🚀🦀🌏", 2);
    assert_eq!(chunks, vec!["🙂🚀", "🦀🌏"]);
    assert_eq!(chunks.concat(), "🙂🚀🦀🌏");
}

#[test]
fn prefers_newline_without_exceeding_character_limit() {
    let chunks = chunk_text_for_channel("甲乙\n丙丁戊\n己庚", 5);
    assert_eq!(chunks, vec!["甲乙", "丙丁戊", "己庚"]);
    assert!(chunks.iter().all(|chunk| chunk.chars().count() <= 5));
}

#[test]
fn empty_or_zero_limit_has_no_segments() {
    assert!(chunk_text_for_channel("  \n ", 10).is_empty());
    assert!(chunk_text_for_channel("content", 0).is_empty());
}
