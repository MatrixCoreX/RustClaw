use claw_core::{
    channel_ingress::ChannelIngressAttachment, conversation_input::ConversationInputSource,
};

use super::*;

#[test]
fn attachment_ids_are_stable_opaque_tokens_and_never_paths() {
    let root = std::env::temp_dir().join(format!(
        "conversation-input-attachment-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(root.join("data/inbox")).expect("inbox");
    std::fs::write(root.join("data/inbox/report.txt"), b"fixture").expect("file");
    let mut request = crate::http::conversation_inputs::client_task::tests::request(
        "rk-test",
        "message-attachment",
        "Summarize the attachment.",
    );
    let ingress = request.task.ingress.as_mut().expect("ingress");
    ingress.attachments.push(ChannelIngressAttachment {
        kind: "file".to_string(),
        path: "data/inbox/report.txt".to_string(),
        mime_type: Some("text/plain".to_string()),
        size: Some(7),
    });
    request.input.source = ConversationInputSource::default();
    let first =
        prepare_attachment_bindings(&root, "principal-1", &request).expect("first bindings");
    let second =
        prepare_attachment_bindings(&root, "principal-1", &request).expect("second bindings");
    assert_eq!(first, second);
    assert_eq!(first.len(), 1);
    assert!(first[0].attachment_id.starts_with("channel_attachment:"));
    assert!(!first[0].attachment_id.contains("report.txt"));
    assert_eq!(first[0].workspace_rel_path, "data/inbox/report.txt");
    std::fs::remove_dir_all(root).expect("cleanup root");
}
