use super::skill_memory_language_hint;
use crate::AppState;
use serde_json::json;

fn object(value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    value.as_object().cloned().expect("object")
}

#[test]
fn skill_memory_language_hint_prefers_skill_args_over_config() {
    let mut state = AppState::test_default_with_fixture_provider();
    state.policy.command_intent.default_locale = "en-US".to_string();

    assert_eq!(
        skill_memory_language_hint(&state, &object(json!({"text": "请记住这个编号"}))),
        "zh-CN"
    );
    assert_eq!(
        skill_memory_language_hint(&state, &object(json!({"query": "remember this id"}))),
        "en"
    );
    assert_eq!(
        skill_memory_language_hint(&state, &object(json!({"action": "read"}))),
        "en-US"
    );
}
