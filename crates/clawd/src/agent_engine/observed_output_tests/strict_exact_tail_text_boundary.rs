#[test]
fn strict_exact_tail_read_accepts_machine_extra_payload() {
    let output = serde_json::json!({
        "status": "ok",
        "text": "tail read completed",
        "extra": {
            "action": "read_range",
            "mode": "tail",
            "requested_n": 2,
            "excerpt": "10|alpha\n11|beta",
            "path": "/tmp/clawd.log"
        }
    })
    .to_string();

    assert_eq!(
        super::strict_exact_tail_read_answer_from_output(&output).as_deref(),
        Some("alpha\nbeta")
    );
}

#[test]
fn strict_exact_tail_read_ignores_json_hidden_in_visible_text() {
    let hidden_payload = serde_json::json!({
        "action": "read_range",
        "mode": "tail",
        "requested_n": 2,
        "excerpt": "10|alpha\n11|beta",
        "path": "/tmp/clawd.log"
    })
    .to_string();
    let output = serde_json::json!({
        "status": "ok",
        "text": hidden_payload
    })
    .to_string();

    assert_eq!(super::strict_exact_tail_read_answer_from_output(&output), None);
}
