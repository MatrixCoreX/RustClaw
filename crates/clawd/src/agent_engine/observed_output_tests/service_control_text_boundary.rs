#[test]
fn service_control_status_direct_answer_ignores_visible_text_json_payload() {
    let value = serde_json::json!({
        "status": "ok",
        "text": serde_json::json!({
            "target": "clawd",
            "status": "running",
            "manager_type": "systemd"
        })
        .to_string()
    });

    assert!(super::service_control_status_direct_answer_candidate(
        &value,
        Some(crate::OutputResponseShape::Strict)
    )
    .is_none());
}

#[test]
fn service_control_status_direct_answer_reads_extra_payload() {
    let value = serde_json::json!({
        "status": "ok",
        "extra": {
            "target": "clawd",
            "status": "running",
            "manager_type": "systemd"
        }
    });

    assert_eq!(
        super::service_control_status_direct_answer_candidate(
            &value,
            Some(crate::OutputResponseShape::Strict)
        )
        .as_deref(),
        Some("target=clawd status=running manager_type=systemd source=service_control")
    );
}
