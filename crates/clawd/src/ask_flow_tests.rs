use super::{audio_failure_fields, audio_failure_planner_text};

#[test]
fn structured_audio_failure_keeps_machine_fields_without_raw_marker() {
    let raw = concat!(
        "__RC_SKILL_ERROR__:",
        r#"{"skill":"audio_transcribe","error_kind":"provider_request_failed","error_text":"private provider detail","extra":{"error_code":"provider_request_failed","message_key":"skill.audio_transcribe.provider_request_failed","retryable":true}}"#
    );

    let (error_code, message_key, retryable) = audio_failure_fields(raw);
    let prompt = audio_failure_planner_text(
        &error_code,
        &message_key,
        retryable,
        "Please also answer the typed part.",
    );

    assert_eq!(error_code, "provider_request_failed");
    assert_eq!(
        message_key,
        "skill.audio_transcribe.provider_request_failed"
    );
    assert!(retryable);
    assert!(prompt.contains("\"transcript_available\":false"));
    assert!(prompt.contains("Please also answer the typed part."));
    assert!(!prompt.contains("__RC_SKILL_ERROR__"));
    assert!(!prompt.contains("private provider detail"));
}

#[test]
fn unstructured_audio_failure_uses_stable_generic_contract() {
    let (error_code, message_key, retryable) = audio_failure_fields("transport closed");

    assert_eq!(error_code, "transcription_unavailable");
    assert_eq!(
        message_key,
        "skill.audio_transcribe.transcription_unavailable"
    );
    assert!(retryable);
}
