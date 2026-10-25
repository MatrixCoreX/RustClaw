use super::*;

fn message(media: JsonValue) -> Message {
    // Untagged MessageKind can accept an invalid media fixture as Empty.
    // Validate these payloads directly so fixture errors retain their cause.
    if let Some(voice) = media.get("voice") {
        serde_json::from_value::<teloxide::types::Voice>(voice.clone())
            .expect("typed voice fixture");
    }
    if let Some(video) = media.get("video") {
        serde_json::from_value::<teloxide::types::Video>(video.clone())
            .expect("typed video fixture");
    }
    let mut value = json!({
        "message_id": 7,
        "date": 1,
        "chat": {"id": 42, "type": "private", "first_name": "Fixture"}
    });
    value
        .as_object_mut()
        .unwrap()
        .extend(media.as_object().unwrap().clone());
    serde_json::from_value(value).expect("Telegram media fixture")
}

#[test]
fn upstream_message_types_preserve_photo_and_voice_attachment_delivery() {
    let photo = message(json!({"caption":"fixture caption", "photo": [
        {"file_id":"small", "file_unique_id":"small-unique", "width":1, "height":1},
        {"file_id":"large", "file_unique_id":"large-unique", "width":2, "height":2}
    ]}));
    assert_eq!(
        extract_image_attachment(&photo),
        Some(("large".into(), "jpg".into()))
    );
    assert_eq!(photo.id.0, 7);
    assert_eq!(photo.chat.id.0, 42);
    assert_eq!(
        photo.text().or_else(|| photo.caption()),
        Some("fixture caption")
    );
    let voice = message(json!({"voice": {
        "file_id":"voice", "file_unique_id":"voice-unique", "duration":23,
        "mime_type":"audio/ogg"
    }}));
    assert_eq!(
        extract_audio_attachment(&voice),
        Some(("voice".into(), "ogg".into()))
    );
}

#[test]
fn upstream_message_types_preserve_video_and_document_attachment_delivery() {
    let video = message(json!({"video": {
        "file_id":"video", "file_unique_id":"video-unique", "duration":1,
        "width":2, "height":2, "file_name":"clip.MP4", "mime_type":"video/mp4"
    }}));
    assert_eq!(
        extract_video_attachment(&video),
        Some(("video".into(), "mp4".into()))
    );
    let document = message(json!({"document": {
        "file_id":"document", "file_unique_id":"document-unique",
        "file_name":"report.PDF", "mime_type":"application/pdf"
    }}));
    assert_eq!(
        extract_file_attachment(&document),
        Some(("document".into(), "pdf".into()))
    );
}

#[test]
fn explicit_null_mime_preserves_voice_and_video_attachment_delivery() {
    let voice = message(json!({"voice": {
        "file_id":"voice-null", "file_unique_id":"voice-null-unique",
        "duration":23, "mime_type":null
    }}));
    assert!(voice.voice().expect("voice variant").mime_type.is_none());
    assert_eq!(
        extract_audio_attachment(&voice),
        Some(("voice-null".into(), "ogg".into()))
    );
    let video = message(json!({"video": {
        "file_id":"video-null", "file_unique_id":"video-null-unique",
        "duration":1, "width":2, "height":2, "mime_type":null
    }}));
    assert!(video.video().expect("video variant").mime_type.is_none());
    assert_eq!(
        extract_video_attachment(&video),
        Some(("video-null".into(), "mp4".into()))
    );
}

#[test]
fn upstream_document_types_keep_media_out_of_generic_file_delivery() {
    let image = message(json!({"document": {
        "file_id":"image", "file_unique_id":"image-unique", "mime_type":"image/png"
    }}));
    assert_eq!(
        extract_image_attachment(&image),
        Some(("image".into(), "png".into()))
    );
    assert!(extract_file_attachment(&image).is_none());
    let audio = message(json!({"document": {
        "file_id":"audio", "file_unique_id":"audio-unique", "mime_type":"audio/ogg"
    }}));
    assert!(extract_file_attachment(&audio).is_none());
}
