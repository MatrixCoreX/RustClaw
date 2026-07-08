use super::preflight_permission_decision;
use super::tests::{install_test_registry, test_state};

#[test]
fn preflight_permission_decision_marks_media_dry_run_as_low_risk_observe() {
    let state = test_state();
    install_test_registry(
        &state,
        r#"
[[skills]]
name = "image_generate"
enabled = true
kind = "runner"
planner_kind = "tool"
risk_level = "high"
requires_confirmation = true
side_effect = true
planner_capabilities = [
  { name = "image.generate", action = "generate", effect = "external", required = ["prompt"], optional = ["dry_run", "output_path"], risk_level = "high", once_per_task = true, idempotent = false, dedup_scope = "action" },
]
"#,
        &["image_generate"],
    );
    let args = serde_json::json!({
        "action": "generate",
        "prompt": "status card",
        "output_path": "document/media_dry_run/image_status_card.png",
        "dry_run": true
    });

    let permission = preflight_permission_decision(
        &state,
        "image_generate",
        &args,
        "media_dry_run_probe",
        "media_dry_run_probe",
    );

    assert_eq!(permission["risk_level"], serde_json::json!("low"));
    assert_eq!(permission["needs_confirmation"], false);
    assert_eq!(permission["action_effect"], serde_json::json!("observe"));
}
