use super::local_whisper_effectively_enabled;

#[test]
fn local_whisper_auto_follows_the_saved_setting() {
    assert!(local_whisper_effectively_enabled(true, None));
    assert!(!local_whisper_effectively_enabled(false, None));
    assert!(local_whisper_effectively_enabled(true, Some("auto")));
    assert!(!local_whisper_effectively_enabled(false, Some("auto")));
}

#[test]
fn local_whisper_runtime_override_takes_precedence() {
    assert!(local_whisper_effectively_enabled(false, Some("true")));
    assert!(local_whisper_effectively_enabled(false, Some("1")));
    assert!(!local_whisper_effectively_enabled(true, Some("false")));
    assert!(!local_whisper_effectively_enabled(true, Some("0")));
}

#[test]
fn invalid_local_whisper_override_fails_closed() {
    assert!(!local_whisper_effectively_enabled(true, Some("invalid")));
}
