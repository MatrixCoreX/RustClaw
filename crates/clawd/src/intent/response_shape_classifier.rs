use super::surface_signals::{
    analyze_prompt_surface, FieldReadPromptShape, OutputCompressionShape, PathOutputPromptShape,
    PromptSurfaceSignals,
};
use crate::OutputResponseShape;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResponseShapeClassification {
    pub(crate) response_shape: OutputResponseShape,
    pub(crate) reason: &'static str,
    pub(crate) source: ResponseShapeDecisionSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ResponseShapeDecisionSource {
    Structured,
    Default,
}

#[allow(dead_code)]
pub(crate) fn classify_fallback_response_shape(
    user_request: &str,
    delivery_required: bool,
) -> ResponseShapeClassification {
    let trimmed = user_request.trim();
    let surface_signals = analyze_prompt_surface(trimmed);
    classify_fallback_response_shape_with_surface(trimmed, delivery_required, &surface_signals)
}

pub(crate) fn classify_fallback_response_shape_with_surface(
    user_request: &str,
    delivery_required: bool,
    surface_signals: &PromptSurfaceSignals,
) -> ResponseShapeClassification {
    if let Some((shape, reason)) =
        classify_structured_response_shape(user_request, delivery_required, surface_signals)
    {
        return ResponseShapeClassification {
            response_shape: shape,
            reason,
            source: ResponseShapeDecisionSource::Structured,
        };
    }
    let (response_shape, reason, source) = if let Some((shape, reason)) =
        classify_signal_response_shape(delivery_required, surface_signals)
    {
        (shape, reason, ResponseShapeDecisionSource::Structured)
    } else {
        let reason = if surface_signals.inline_json_shape.is_some() {
            "inline_json_default_free"
        } else {
            "default_free"
        };
        (
            OutputResponseShape::Free,
            reason,
            ResponseShapeDecisionSource::Default,
        )
    };
    ResponseShapeClassification {
        response_shape,
        reason,
        source,
    }
}

fn classify_structured_response_shape(
    user_request: &str,
    delivery_required: bool,
    surface_signals: &PromptSurfaceSignals,
) -> Option<(OutputResponseShape, &'static str)> {
    if delivery_required {
        return Some((OutputResponseShape::FileToken, "delivery_required"));
    }
    if surface_signals.inline_json_shape.is_some() {
        return Some((OutputResponseShape::Free, "inline_json_default_free"));
    }
    if matches!(
        surface_signals.path_output_prompt_shape,
        Some(PathOutputPromptShape::ScalarOnly)
    ) {
        return Some((OutputResponseShape::Scalar, "structured_path_output_scalar"));
    }
    if matches!(
        surface_signals.field_read_prompt_shape,
        Some(FieldReadPromptShape::SimpleExplicitScalar)
    ) {
        return Some((
            OutputResponseShape::Scalar,
            "structured_field_selector_scalar",
        ));
    }
    if crate::intent::deterministic_gate::text_looks_like_git_scalar_query(user_request) {
        return Some((OutputResponseShape::Scalar, "structured_git_scalar_query"));
    }
    if surface_signals.requested_sentence_count == Some(1) {
        return Some((
            OutputResponseShape::OneSentence,
            "structured_exact_sentence_count",
        ));
    }
    None
}

fn classify_signal_response_shape(
    delivery_required: bool,
    surface_signals: &PromptSurfaceSignals,
) -> Option<(OutputResponseShape, &'static str)> {
    if delivery_required {
        return Some((OutputResponseShape::FileToken, "delivery_required"));
    }
    match surface_signals.output_compression_shape {
        Some(OutputCompressionShape::ScalarOnly) => {
            return Some((OutputResponseShape::Scalar, "surface_output_only_scalar"));
        }
        Some(OutputCompressionShape::Brief) => {
            return Some((
                OutputResponseShape::OneSentence,
                "surface_brief_one_sentence",
            ));
        }
        None => {}
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{classify_fallback_response_shape, ResponseShapeDecisionSource};
    use crate::intent::surface_signals::{analyze_prompt_surface, InlineJsonShape};
    use crate::OutputResponseShape;

    #[test]
    fn delivery_required_forces_file_token() {
        let out = classify_fallback_response_shape("把 document/report.md 发给我", true);
        assert_eq!(out.response_shape, OutputResponseShape::FileToken);
        assert_eq!(out.reason, "delivery_required");
    }

    #[test]
    fn output_only_request_prefers_scalar() {
        let out =
            classify_fallback_response_shape("读取 package.json 里的 name 字段，只输出值", false);
        assert_eq!(out.response_shape, OutputResponseShape::Scalar);
        assert_eq!(out.reason, "structured_field_selector_scalar");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
        assert!(
            analyze_prompt_surface("读取 package.json 里的 name 字段，只输出值")
                .has_concrete_locator_hint()
        );
    }

    #[test]
    fn only_give_me_the_value_request_prefers_scalar() {
        let out = classify_fallback_response_shape("去 package.json 里找 name，只把值给我", false);
        assert_eq!(out.response_shape, OutputResponseShape::Scalar);
        assert_eq!(out.reason, "structured_field_selector_scalar");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn simple_field_read_without_value_wording_prefers_structured_scalar() {
        let out = classify_fallback_response_shape(
            "读取 /home/guagua/rustclaw/configs/config.toml 里的 tools.allow_sudo",
            false,
        );
        assert_eq!(out.response_shape, OutputResponseShape::Scalar);
        assert_eq!(out.reason, "structured_field_selector_scalar");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn nothing_else_hostname_style_request_prefers_scalar() {
        let out = classify_fallback_response_shape("只告诉我这台机器的 hostname，别补别的", false);
        assert_eq!(out.response_shape, OutputResponseShape::Scalar);
        assert_eq!(out.reason, "surface_output_only_scalar");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn directory_scoped_where_is_path_request_prefers_structured_scalar() {
        let out = classify_fallback_response_shape(
            "在 scripts/nl_tests/fixtures/locator_smart/case_only 里查一下 report.md 在哪，路径就行",
            false,
        );
        assert_eq!(out.response_shape, OutputResponseShape::Scalar);
        assert_eq!(out.reason, "structured_path_output_scalar");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn git_branch_query_prefers_structured_scalar() {
        let out =
            classify_fallback_response_shape("output only the current git branch name", false);
        assert_eq!(out.response_shape, OutputResponseShape::Scalar);
        assert_eq!(out.reason, "structured_git_scalar_query");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn git_recent_commit_query_prefers_structured_scalar() {
        let out = classify_fallback_response_shape(
            "tell me only the title of the most recent git commit",
            false,
        );
        assert_eq!(out.response_shape, OutputResponseShape::Scalar);
        assert_eq!(out.reason, "structured_git_scalar_query");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn locator_followed_by_non_field_summary_text_does_not_turn_scalar() {
        let out = classify_fallback_response_shape(
            "看一下 package.json，然后用一句话总结它主要配置了什么",
            false,
        );
        assert_eq!(out.response_shape, OutputResponseShape::OneSentence);
        assert_eq!(out.reason, "structured_exact_sentence_count");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn numeric_line_count_after_locator_does_not_turn_scalar() {
        let out = classify_fallback_response_shape(
            "把 /home/guagua/rustclaw/README.md 开头读 10 行",
            false,
        );
        assert_ne!(out.response_shape, OutputResponseShape::Scalar);
    }

    #[test]
    fn one_sentence_request_prefers_one_sentence() {
        let out = classify_fallback_response_shape(
            "看一下 scripts/nl_tests/fixtures/device_local/configs/app_config.toml，然后用一句大白话说它主要配置了什么",
            false,
        );
        assert_eq!(out.response_shape, OutputResponseShape::OneSentence);
        assert_eq!(out.reason, "structured_exact_sentence_count");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn exact_english_sentence_count_prefers_one_sentence_structurally() {
        let out = classify_fallback_response_shape("Summarize it in 1 sentence.", false);
        assert_eq!(out.response_shape, OutputResponseShape::OneSentence);
        assert_eq!(out.reason, "structured_exact_sentence_count");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
    }

    #[test]
    fn multi_sentence_request_stays_free() {
        let out = classify_fallback_response_shape("Summarize it in 2 sentences.", false);
        assert_eq!(out.response_shape, OutputResponseShape::Free);
        assert_eq!(out.reason, "default_free");
        assert_eq!(out.source, ResponseShapeDecisionSource::Default);
    }

    #[test]
    fn inline_json_transform_defaults_to_free() {
        let out = classify_fallback_response_shape(
            r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#,
            false,
        );
        assert_eq!(out.response_shape, OutputResponseShape::Free);
        assert_eq!(out.reason, "inline_json_default_free");
        assert_eq!(out.source, ResponseShapeDecisionSource::Structured);
        assert_eq!(
            analyze_prompt_surface(
                r#"sort this JSON array by score descending and render it as a markdown table: [{"name":"alpha","score":7},{"name":"beta","score":12}]"#
            )
            .inline_json_shape,
            Some(InlineJsonShape::EmbeddedPayload)
        );
    }
}
