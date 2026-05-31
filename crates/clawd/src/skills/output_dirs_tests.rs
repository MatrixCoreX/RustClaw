use super::ensure_default_output_dir_for_skill_args;
use serde_json::json;
use std::path::Path;

#[test]
fn preserves_explicit_image_output_path() {
    let args = json!({
        "prompt": "make a smoke image",
        "output_path": "document/skill_generate_smoke.png"
    });

    let actual = ensure_default_output_dir_for_skill_args(
        Path::new("/tmp/no-such-workspace"),
        "image_generate",
        args,
    );

    assert_eq!(
        actual.get("output_path").and_then(|value| value.as_str()),
        Some("document/skill_generate_smoke.png")
    );
}

#[test]
fn adds_image_output_path_when_missing() {
    let args = json!({ "prompt": "make a smoke image" });

    let actual = ensure_default_output_dir_for_skill_args(
        Path::new("/tmp/no-such-workspace"),
        "image_generate",
        args,
    );

    let output_path = actual
        .get("output_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    assert!(output_path.starts_with("document/gen-"));
    assert!(output_path.ends_with(".png"));
}
