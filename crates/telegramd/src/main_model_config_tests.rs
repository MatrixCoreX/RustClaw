use super::*;

#[test]
fn apply_model_config_populates_custom_vendor_only_in_llm_section() {
    let mut v: TomlValue = toml::from_str(
        r#"
[llm]
selected_vendor = "openai"
selected_model = "gpt-4o-mini"
"#,
    )
    .expect("parse");
    apply_model_config_value(&mut v, "custom", "my-custom-model").expect("apply");

    let llm = v.get("llm").and_then(|x| x.as_table()).expect("llm");
    assert_eq!(
        llm.get("selected_vendor").and_then(|x| x.as_str()),
        Some("custom")
    );
    assert_eq!(
        llm.get("selected_model").and_then(|x| x.as_str()),
        Some("my-custom-model")
    );
    let custom = llm
        .get("custom")
        .and_then(|x| x.as_table())
        .expect("custom");
    assert_eq!(
        custom.get("base_url").and_then(|x| x.as_str()),
        Some("https://api.example.com/v1")
    );
    assert_eq!(
        custom.get("api_key").and_then(|x| x.as_str()),
        Some("REPLACE_ME_CUSTOM_API_KEY")
    );
    assert_eq!(
        custom.get("model").and_then(|x| x.as_str()),
        Some("my-custom-model")
    );
    assert!(v.get("audio_synthesize").is_none());
    assert!(v.get("audio_transcribe").is_none());
}

#[test]
fn apply_model_config_qwen_uses_expected_default_base_url() {
    let mut v: TomlValue = toml::from_str("[llm]\n").expect("parse");
    apply_model_config_value(&mut v, "qwen", "qwen-max-latest").expect("apply");
    let qwen = v
        .get("llm")
        .and_then(|x| x.get("qwen"))
        .and_then(|x| x.as_table())
        .expect("qwen");
    assert_eq!(
        qwen.get("base_url").and_then(|x| x.as_str()),
        Some("https://dashscope.aliyuncs.com/compatible-mode/v1")
    );
}

#[test]
fn apply_model_config_minimax_uses_expected_default_base_url() {
    let mut v: TomlValue = toml::from_str("[llm]\n").expect("parse");
    apply_model_config_value(&mut v, "minimax", "MiniMax-M2.5").expect("apply");
    let minimax = v
        .get("llm")
        .and_then(|x| x.get("minimax"))
        .and_then(|x| x.as_table())
        .expect("minimax");
    assert_eq!(
        minimax.get("base_url").and_then(|x| x.as_str()),
        Some("https://api.minimaxi.com/v1")
    );
}

#[test]
fn apply_model_config_mimo_uses_expected_default_base_url() {
    let mut v: TomlValue = toml::from_str("[llm]\n").expect("parse");
    apply_model_config_value(&mut v, "mimo", "mimo-v2.5-pro").expect("apply");
    let mimo = v
        .get("llm")
        .and_then(|x| x.get("mimo"))
        .and_then(|x| x.as_table())
        .expect("mimo");
    assert_eq!(
        mimo.get("base_url").and_then(|x| x.as_str()),
        Some("https://token-plan-cn.xiaomimimo.com/v1")
    );
}
