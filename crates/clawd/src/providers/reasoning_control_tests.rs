use super::*;

#[test]
fn structured_verification_uses_the_documented_m3_toggle_with_the_same_model_and_output_cap() {
    let mut body = Map::from_iter([
        ("model".into(), json!("MiniMax-M3")),
        ("max_tokens".into(), json!(4096)),
    ]);
    apply_openai_reasoning(
        &mut body,
        "MiniMax-M3",
        &ChatRequestHints {
            structured_verification: true,
            reasoning_effort: Some("low".into()),
            ..Default::default()
        },
    );
    assert_eq!(body["thinking"], json!({"type":"disabled"}));
    assert!(body.get("reasoning_effort").is_none());
    assert_eq!(body["model"], "MiniMax-M3");
    assert_eq!(body["max_tokens"], 4096);
}

#[test]
fn ordinary_m3_planning_keeps_adaptive_thinking_and_does_not_claim_unsupported_effort_control() {
    for effort in [None, Some("low"), Some("high")] {
        let mut body = Map::new();
        apply_openai_reasoning(
            &mut body,
            "MiniMax-M3",
            &ChatRequestHints {
                reasoning_effort: effort.map(str::to_owned),
                ..Default::default()
            },
        );
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }
}

#[test]
fn m31_and_other_models_keep_their_effort_and_never_inherit_the_m3_thinking_toggle() {
    for model in ["MiniMax-M3.1-Flash", "gpt-5", "fixture-model"] {
        let mut body = Map::new();
        apply_openai_reasoning(
            &mut body,
            model,
            &ChatRequestHints {
                structured_verification: true,
                reasoning_effort: Some("low".into()),
                ..Default::default()
            },
        );
        assert_eq!(body["reasoning_effort"], "low");
        assert!(body.get("thinking").is_none());
    }
}

#[test]
fn anthropic_compatibility_does_not_send_claude_manual_thinking_to_m3_or_m31() {
    let hints = ChatRequestHints {
        structured_verification: true,
        reasoning_effort: Some("low".into()),
        ..Default::default()
    };
    let mut m3 = Map::new();
    assert!(apply_anthropic_reasoning(&mut m3, "MiniMax-M3", &hints));
    assert_eq!(m3["thinking"], json!({"type":"disabled"}));
    assert!(m3.get("output_config").is_none());
    let mut m31 = Map::new();
    assert!(apply_anthropic_reasoning(
        &mut m31,
        "MiniMax-M3.1-Flash",
        &hints
    ));
    assert_eq!(m31["output_config"], json!({"effort":"low"}));
    assert!(m31.get("thinking").is_none());
    let mut claude = Map::new();
    assert!(!apply_anthropic_reasoning(
        &mut claude,
        "claude-sonnet-4-6",
        &hints
    ));
    assert!(claude.is_empty());
    let mut ordinary = Map::new();
    assert!(apply_anthropic_reasoning(
        &mut ordinary,
        "MiniMax-M3",
        &ChatRequestHints::default()
    ));
    assert!(ordinary.is_empty());
}
