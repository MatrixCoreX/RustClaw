use super::*;

#[test]
fn compatibility_decoder_parses_every_legacy_prefix_once() {
    let text = concat!(
        "caption\n",
        "IMAGE_FILE:`/tmp/image.png`\n",
        "VIDEO_FILE:/tmp/video.mp4\n",
        "VOICE_FILE:/tmp/voice.ogg\n",
        "MUSIC_FILE:/tmp/music.mp3\n",
        "FILE_FILE:/tmp/document.pdf\n",
        "FILE:/tmp/auto.bin\n",
        "IMAGE_URL:https://example.invalid/image.png\n",
        "VIDEO_URL:https://example.invalid/video.mp4\n",
        "FILE_URL:https://example.invalid/file.pdf\n",
        "MEDIA_URL:https://example.invalid/auto.bin\n",
    );
    let tokens = legacy_delivery_tokens(text);

    assert_eq!(tokens.len(), 10);
    assert_eq!(tokens[0].kind, LegacyDeliveryKind::Image);
    assert_eq!(tokens[0].reference, "/tmp/image.png");
    assert_eq!(tokens[4].kind, LegacyDeliveryKind::File);
    assert_eq!(tokens[5].kind, LegacyDeliveryKind::Auto);
    assert_eq!(tokens[6].location, LegacyDeliveryLocation::RemoteUrl);
    assert_eq!(strip_legacy_delivery_lines(text), "caption");
    assert_eq!(legacy_delivery_lines(text).lines().count(), 10);
}

#[test]
fn local_projection_does_not_consume_remote_tokens_or_prefix_examples() {
    let text = concat!(
        "caption IMAGE_FILE:/tmp/example.png\n",
        "IMAGE_FILE:/tmp/image.png\n",
        "IMAGE_URL:https://example.invalid/image.png\n",
    );

    assert_eq!(
        strip_legacy_local_delivery_lines(text),
        concat!(
            "caption IMAGE_FILE:/tmp/example.png\n",
            "IMAGE_URL:https://example.invalid/image.png"
        )
    );
    assert_eq!(
        legacy_local_delivery_lines(text),
        "IMAGE_FILE:/tmp/image.png"
    );
}

#[test]
fn empty_tokens_are_not_treated_as_delivery_lines() {
    assert_eq!(parse_legacy_delivery_line("FILE:  `  "), None);
    assert_eq!(
        parse_legacy_delivery_line_ref("FILE:  ").map(|token| token.kind),
        Some(LegacyDeliveryKind::Auto)
    );
    assert_eq!(
        strip_legacy_delivery_lines("before\nFILE:  `  \nafter"),
        "before\nafter"
    );
}

#[test]
fn compatibility_decoder_accepts_delivery_tokens_wrapped_in_list_markers() {
    let text = concat!(
        "caption\n",
        "1. IMAGE_FILE:/tmp/first.webp\n",
        "2) IMAGE_FILE:/tmp/second.webp\n",
        "- FILE:/tmp/article.txt\n",
        "* IMAGE_URL:https://example.invalid/image.webp\n",
    );

    let tokens = legacy_delivery_tokens(text);

    assert_eq!(tokens.len(), 4);
    assert_eq!(tokens[0].reference, "/tmp/first.webp");
    assert_eq!(tokens[1].reference, "/tmp/second.webp");
    assert_eq!(tokens[2].kind, LegacyDeliveryKind::Auto);
    assert_eq!(tokens[3].location, LegacyDeliveryLocation::RemoteUrl);
    assert_eq!(strip_legacy_delivery_lines(text), "caption");
}

#[test]
fn compatibility_decoder_does_not_treat_inline_or_unspaced_examples_as_delivery_tokens() {
    for text in [
        "caption 1. IMAGE_FILE:/tmp/example.webp",
        "1.IMAGE_FILE:/tmp/example.webp",
        "release-1. IMAGE_FILE:/tmp/example.webp",
    ] {
        assert_eq!(parse_legacy_delivery_line(text), None, "{text}");
    }
}
