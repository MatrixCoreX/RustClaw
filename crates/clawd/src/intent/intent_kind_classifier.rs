use super::surface_signals::{
    analyze_prompt_surface, DeliveryPromptShape, InlineTransformPromptShape, PromptSurfaceSignals,
};
use crate::{OutputResponseShape, RoutedMode};
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FallbackIntentKindClassification {
    pub(crate) routed_mode: RoutedMode,
    pub(crate) wants_file_delivery: bool,
    pub(crate) reason: &'static str,
    pub(crate) source: IntentKindDecisionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntentKindDecisionSource {
    Structured,
    Default,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BarePathOnlyClassification {
    pub(crate) is_bare_path_only: bool,
    pub(crate) source: IntentKindDecisionSource,
}

#[allow(dead_code)]
pub(crate) fn classify_inline_structured_transform(
    user_request: &str,
) -> Option<FallbackIntentKindClassification> {
    let surface = analyze_prompt_surface(user_request);
    classify_inline_structured_transform_with_surface(user_request, &surface)
}

pub(crate) fn classify_inline_structured_transform_with_surface(
    user_request: &str,
    surface: &PromptSurfaceSignals,
) -> Option<FallbackIntentKindClassification> {
    request_looks_like_inline_structured_transform_with_surface(user_request, surface).then_some(
        FallbackIntentKindClassification {
            routed_mode: RoutedMode::Act,
            wants_file_delivery: false,
            reason: "surface_inline_structured_transform",
            source: IntentKindDecisionSource::Structured,
        },
    )
}

#[allow(dead_code)]
pub(crate) fn classify_explicit_locator_fallback(
    user_request: &str,
    response_shape: OutputResponseShape,
) -> FallbackIntentKindClassification {
    let surface = analyze_prompt_surface(user_request);
    classify_explicit_locator_fallback_with_surface(response_shape, &surface)
}

pub(crate) fn classify_explicit_locator_fallback_with_surface(
    response_shape: OutputResponseShape,
    surface: &PromptSurfaceSignals,
) -> FallbackIntentKindClassification {
    let (wants_file_delivery, source) = if let Some(structured) =
        classify_structured_file_delivery_intent(response_shape, &surface)
    {
        (structured, IntentKindDecisionSource::Structured)
    } else if let Some(signal_based) = classify_signal_file_delivery_intent(&surface) {
        (signal_based, IntentKindDecisionSource::Structured)
    } else {
        (false, IntentKindDecisionSource::Default)
    };
    let routed_mode = if wants_file_delivery
        || matches!(
            response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::FileToken
        ) {
        RoutedMode::Act
    } else {
        RoutedMode::ChatAct
    };
    FallbackIntentKindClassification {
        routed_mode,
        wants_file_delivery,
        reason: "surface_explicit_locator_fallback",
        source,
    }
}

fn classify_structured_file_delivery_intent(
    response_shape: OutputResponseShape,
    surface: &PromptSurfaceSignals,
) -> Option<bool> {
    if matches!(response_shape, OutputResponseShape::FileToken) {
        return Some(true);
    }
    surface.has_delivery_token_reference().then_some(true)
}

fn classify_signal_file_delivery_intent(surface: &PromptSurfaceSignals) -> Option<bool> {
    request_wants_file_delivery_with_surface(surface).then_some(true)
}

#[cfg_attr(not(test), allow(dead_code))]
#[allow(dead_code)]
pub(crate) fn request_looks_like_inline_structured_transform(user_request: &str) -> bool {
    let surface = analyze_prompt_surface(user_request);
    request_looks_like_inline_structured_transform_with_surface(user_request, &surface)
}

#[allow(dead_code)]
pub(crate) fn request_wants_file_delivery(user_request: &str) -> bool {
    let surface = analyze_prompt_surface(user_request);
    request_wants_file_delivery_with_surface(&surface)
}

pub(crate) fn request_wants_file_delivery_with_surface(surface: &PromptSurfaceSignals) -> bool {
    matches!(
        surface.delivery_prompt_shape,
        Some(DeliveryPromptShape::PhraseWithTarget)
    )
}

pub(crate) fn request_looks_like_inline_structured_transform_with_surface(
    _user_request: &str,
    surface: &PromptSurfaceSignals,
) -> bool {
    matches!(
        surface.inline_transform_prompt_shape,
        Some(InlineTransformPromptShape::ActionWithTarget)
    )
}

#[allow(dead_code)]
pub(crate) fn is_bare_path_only_input_no_verb(text: &str) -> bool {
    classify_bare_path_only_input(text).is_bare_path_only
}

pub(crate) fn classify_bare_path_only_input(text: &str) -> BarePathOnlyClassification {
    let surface = analyze_prompt_surface(text.trim());
    classify_bare_path_only_input_with_surface(text, &surface)
}

pub(crate) fn classify_bare_path_only_input_with_surface(
    text: &str,
    surface: &PromptSurfaceSignals,
) -> BarePathOnlyClassification {
    if let Some(structured) = classify_structured_bare_path_only_input_with_surface(text, surface) {
        return BarePathOnlyClassification {
            is_bare_path_only: structured,
            source: IntentKindDecisionSource::Structured,
        };
    }
    BarePathOnlyClassification {
        is_bare_path_only: false,
        source: IntentKindDecisionSource::Default,
    }
}

fn classify_structured_bare_path_only_input_with_surface(
    text: &str,
    surface: &PromptSurfaceSignals,
) -> Option<bool> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.len() > 60 {
        return None;
    }
    if trimmed.contains('?')
        || trimmed.contains('？')
        || trimmed.contains('!')
        || trimmed.contains('！')
    {
        return None;
    }
    let token_count = trimmed.split_whitespace().count();
    if token_count == 1 {
        if surface.has_single_filename_candidate() || token_looks_like_pathish_filename(trimmed) {
            return Some(true);
        }
    }
    if surface.inline_json_shape.is_some() || surface.has_structured_target_refinement() {
        return None;
    }
    if token_count != 1 {
        return None;
    }
    if surface.has_explicit_path_or_url() || surface.has_workspace_single_token_hint() {
        return Some(true);
    }
    None
}

fn token_looks_like_pathish_filename(token: &str) -> bool {
    let trimmed = token.trim();
    if trimmed.is_empty()
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.starts_with('.')
    {
        return false;
    }
    let Some((stem, ext)) = trimmed.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !ext.is_empty()
        && ext.len() <= 8
        && stem
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        && ext
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

pub(crate) fn bare_path_clarify_question_for(text: &str) -> String {
    let path = text.trim();
    format!(
        "你想对 `{path}` 做什么？比如列出内容、读取某个文件、还是发给我？ / What do you want to do with `{path}`? e.g. list its contents, read a file, or send it to me?",
        path = path
    )
}

#[cfg(test)]
mod tests {
    use super::{
        bare_path_clarify_question_for, classify_bare_path_only_input,
        classify_explicit_locator_fallback, classify_inline_structured_transform,
        is_bare_path_only_input_no_verb, request_wants_file_delivery, IntentKindDecisionSource,
    };
    use crate::{OutputResponseShape, RoutedMode};

    #[test]
    fn inline_structured_transform_is_act() {
        let out = classify_inline_structured_transform(
            r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7}]"#,
        )
        .expect("inline structured transform should classify");
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.reason, "surface_inline_structured_transform");
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(!out.wants_file_delivery);
    }

    #[test]
    fn inline_json_prompt_without_transform_instruction_does_not_classify() {
        let out = classify_inline_structured_transform(
            r#"explain this JSON briefly: [{"name":"alpha","score":7}]"#,
        );
        assert!(out.is_none());
    }

    #[test]
    fn scalar_explicit_locator_prefers_act() {
        let out = classify_explicit_locator_fallback(
            "read scripts/nl_tests/fixtures/device_local/package.json and output only the name field",
            OutputResponseShape::Scalar,
        );
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Default);
        assert!(!out.wants_file_delivery);
    }

    #[test]
    fn one_sentence_explicit_locator_prefers_chat_act() {
        let out = classify_explicit_locator_fallback(
            "看一下 scripts/nl_tests/fixtures/device_local/configs/app_config.toml，然后用一句大白话说它主要配置了什么",
            OutputResponseShape::OneSentence,
        );
        assert_eq!(out.routed_mode, RoutedMode::ChatAct);
        assert_eq!(out.source, IntentKindDecisionSource::Default);
        assert!(!out.wants_file_delivery);
    }

    #[test]
    fn delivery_request_prefers_act() {
        let out = classify_explicit_locator_fallback(
            "把 document/report.md 发给我",
            OutputResponseShape::Free,
        );
        assert!(request_wants_file_delivery("把 document/report.md 发给我"));
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(out.wants_file_delivery);
    }

    #[test]
    fn bare_stem_delivery_request_prefers_act() {
        let out = classify_explicit_locator_fallback("把 readme 发给我", OutputResponseShape::Free);
        assert!(request_wants_file_delivery("把 readme 发给我"));
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(out.wants_file_delivery);
    }

    #[test]
    fn file_token_contract_prefers_act_without_lexical_delivery_hint() {
        let out =
            classify_explicit_locator_fallback("继续用当前目标", OutputResponseShape::FileToken);
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(out.wants_file_delivery);
    }

    #[test]
    fn explicit_delivery_token_prefers_act_without_lexical_delivery_hint() {
        let out = classify_explicit_locator_fallback(
            "继续处理这个目标 FILE:/tmp/report.md",
            OutputResponseShape::Free,
        );
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(out.wants_file_delivery);
    }

    #[test]
    fn delivery_phrase_without_target_shape_does_not_trigger_legacy_delivery() {
        assert!(!request_wants_file_delivery("请直接发给我"));
        let out = classify_explicit_locator_fallback("请直接发给我", OutputResponseShape::Free);
        assert_eq!(out.routed_mode, RoutedMode::ChatAct);
        assert_eq!(out.source, IntentKindDecisionSource::Default);
        assert!(!out.wants_file_delivery);
    }

    #[test]
    fn generic_file_delivery_phrase_can_still_trigger_delivery_clarify() {
        assert!(request_wants_file_delivery("把文件发给我"));
        assert!(request_wants_file_delivery("send me the file"));
        let out = classify_explicit_locator_fallback("把文件发给我", OutputResponseShape::Free);
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(out.wants_file_delivery);
    }

    #[test]
    fn colloquial_config_delivery_phrase_still_triggers_delivery_clarify() {
        assert!(request_wants_file_delivery(
            "把那份本地配置直接甩给我，别贴正文"
        ));
        let out = classify_explicit_locator_fallback(
            "把那份本地配置直接甩给我，别贴正文",
            OutputResponseShape::Free,
        );
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(out.wants_file_delivery);
    }

    #[test]
    fn deictic_delivery_phrase_still_triggers_legacy_delivery() {
        assert!(request_wants_file_delivery("把这个发给我"));
        let out = classify_explicit_locator_fallback("把这个发给我", OutputResponseShape::Free);
        assert_eq!(out.routed_mode, RoutedMode::Act);
        assert_eq!(out.source, IntentKindDecisionSource::Structured);
        assert!(out.wants_file_delivery);
    }

    #[test]
    fn bare_path_classifier_preserves_existing_behavior() {
        let dir = classify_bare_path_only_input("document/");
        assert!(dir.is_bare_path_only);
        assert_eq!(dir.source, IntentKindDecisionSource::Structured);
        let logs = classify_bare_path_only_input("logs");
        assert!(logs.is_bare_path_only);
        assert_eq!(logs.source, IntentKindDecisionSource::Structured);
        let read = classify_bare_path_only_input("read src/main.rs");
        assert!(!read.is_bare_path_only);
        assert_eq!(read.source, IntentKindDecisionSource::Default);
        assert!(is_bare_path_only_input_no_verb("document/"));
        assert!(is_bare_path_only_input_no_verb("logs"));
        assert!(!is_bare_path_only_input_no_verb("read src/main.rs"));
        let q = bare_path_clarify_question_for("document/");
        assert!(q.contains("`document/`"));
    }
}
