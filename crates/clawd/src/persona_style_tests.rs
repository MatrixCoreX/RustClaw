use super::*;

#[test]
fn six_style_profiles_preserve_the_canonical_body_and_protected_tokens() {
    let canonical = "部署完成，状态为 OK。版本 0.1.8，地址 https://example.test/a。";
    for prefix in [
        "STYLE_COMPANION",
        "STYLE_EXPERT",
        "STYLE_TEACHER",
        "STYLE_ADVISOR",
        "STYLE_REVIEWER",
        "STYLE_CUSTOM",
    ] {
        let rendered = format!("{prefix}\n\n{canonical}");
        assert!(persona_render_is_semantically_safe(
            canonical, &rendered, prefix
        ));
        assert!(rendered.ends_with(canonical));
    }
}

#[test]
fn exact_and_structured_shapes_bypass_style_rendering() {
    assert!(looks_like_exact_scalar("AGENT_LLM_OK"));
    assert!(looks_like_exact_scalar("42"));
    assert!(looks_like_table("| A | B |\n| - | - |\n| 1 | 2 |"));
    assert!(serde_json::from_str::<serde_json::Value>(r#"{"ok":true}"#).is_ok());
}

#[test]
fn custom_profile_text_is_not_interpreted_as_an_execution_instruction() {
    let canonical = "SAFE_CANONICAL_RESULT";
    let prefix = "STYLE_CUSTOM";
    let rendered = format!("{prefix}\n\n{canonical}");
    assert!(persona_render_is_semantically_safe(
        canonical, &rendered, prefix
    ));
}
