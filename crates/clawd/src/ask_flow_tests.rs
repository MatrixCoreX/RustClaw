use super::{
    attached_image_analysis_context, attached_image_inputs, audio_failure_fields,
    audio_failure_planner_text,
};
use serde_json::json;

#[test]
fn attached_image_context_preserves_description_and_visible_text() {
    let outcome = crate::skills::SkillRunOutcome {
        text: "一张写有营业时间的店铺招牌。".to_string(),
        notify: None,
        validation: None,
        extra: Some(json!({
            "provider": "fixture",
            "structured": {
                "summary": "一张写有营业时间的店铺招牌。",
                "objects": ["店铺招牌"],
                "visible_text": ["营业时间", "09:00-18:00"],
                "uncertainties": []
            }
        })),
    };

    let context = attached_image_analysis_context(1, false, &outcome).expect("image context");
    let parsed: serde_json::Value = serde_json::from_str(&context).expect("image context json");

    assert_eq!(parsed["image_count"], 1);
    assert_eq!(parsed["typed_instruction_present"], false);
    assert_eq!(parsed["analysis_text"], outcome.text);
    assert_eq!(
        parsed["structured"]["visible_text"],
        json!(["营业时间", "09:00-18:00"])
    );
    assert!(parsed.get("provider").is_none());
    assert_eq!(parsed["instruction_authority"], "none");
}

#[test]
fn attached_image_context_keeps_empty_visible_text_empty() {
    let outcome = crate::skills::SkillRunOutcome {
        text: "一只猫坐在窗边。".to_string(),
        notify: None,
        validation: None,
        extra: Some(json!({
            "structured": {
                "summary": "一只猫坐在窗边。",
                "objects": ["猫", "窗户"],
                "visible_text": [],
                "uncertainties": []
            }
        })),
    };

    let context = attached_image_analysis_context(1, false, &outcome).expect("image context");
    let parsed: serde_json::Value = serde_json::from_str(&context).expect("image context json");

    assert_eq!(parsed["structured"]["visible_text"], json!([]));
}

#[test]
fn attached_image_context_rejects_unstructured_analysis() {
    let outcome = crate::skills::SkillRunOutcome {
        text: "图像分析结果".to_string(),
        notify: None,
        validation: None,
        extra: None,
    };

    let error = attached_image_analysis_context(1, true, &outcome)
        .expect_err("unstructured image analysis must not enter planner context");

    assert!(error
        .to_string()
        .contains("image_vision_describe_structured_output_missing"));
}

#[test]
fn attached_image_inputs_accept_channel_attachments_and_strip_transport_fields() {
    let payload = json!({
        "attachments": [
            {
                "kind": "image",
                "path": "data/inbox/photo.jpg",
                "mime_type": "image/jpeg",
                "size": 42
            },
            {
                "kind": "file",
                "path": "data/inbox/report.pdf",
                "mime_type": "application/pdf"
            }
        ]
    });

    assert_eq!(
        attached_image_inputs(&payload),
        vec![json!({"path": "data/inbox/photo.jpg"})]
    );
}

#[test]
fn attached_image_inputs_prefer_explicit_images_and_preserve_order() {
    let payload = json!({
        "images": [
            {"path": "data/inbox/first.png", "kind": "image", "size": 10},
            {"url": "https://example.test/second.webp"}
        ],
        "attachments": [
            {"kind": "image", "path": "data/inbox/ignored.jpg"}
        ]
    });

    assert_eq!(
        attached_image_inputs(&payload),
        vec![
            json!({"path": "data/inbox/first.png"}),
            json!({"url": "https://example.test/second.webp"})
        ]
    );
}

#[test]
fn structured_audio_failure_keeps_machine_fields_without_raw_marker() {
    let raw = concat!(
        "__RC_SKILL_ERROR__:",
        r#"{"skill":"audio_transcribe","error_code":"provider_request_failed","error_text":"private provider detail","extra":{"error_code":"provider_request_failed","message_key":"skill.audio_transcribe.provider_request_failed","retryable":true}}"#
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
