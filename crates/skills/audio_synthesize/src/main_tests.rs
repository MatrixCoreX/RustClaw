use super::*;

#[test]
fn parse_vendor_aliases() {
    assert!(matches!(parse_vendor("openai"), Some(VendorKind::OpenAI)));
    assert!(matches!(parse_vendor("gemini"), Some(VendorKind::Google)));
    assert!(matches!(
        parse_vendor("claude"),
        Some(VendorKind::Anthropic)
    ));
    assert!(matches!(parse_vendor("xai"), Some(VendorKind::Grok)));
}

#[test]
fn normalize_and_ext() {
    assert_eq!(normalize_format("mp3"), "mp3");
    assert_eq!(normalize_format("unknown"), "opus");
    assert_eq!(mimo_audio_format("mp3"), "mp3");
    assert_eq!(google_audio_encoding("mp3"), "MP3");
    assert_eq!(output_ext("opus"), "ogg");
}
