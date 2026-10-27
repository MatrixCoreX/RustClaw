use super::*;
use crate::verifier::VerifyResult;

fn media_download_with_scope(args: serde_json::Value, request_text: &str) -> VerifyResult {
    let state = test_state();
    let task = test_task();
    verify_plan(
        &state,
        &task,
        VerifyInput {
            output_contract: None,
            request_text: Some(request_text),
            context_bundle_summary: None,
            plan_result: &plan_result(vec![PlanStep {
                step_id: "media".to_string(),
                action_type: "call_skill".to_string(),
                skill: "media_download".to_string(),
                args,
                depends_on: Vec::new(),
                why: String::new(),
            }]),
            execution_recipe: crate::execution_recipe::ExecutionRecipeRuntimeState::default(),
        },
        VerifyMode::Enforce,
    )
}

#[test]
fn current_request_evidence_blocks_history_inherited_optional_modifier() {
    let result = media_download_with_scope(
        json!({
            "action": "download",
            "share": "https://example.invalid/current",
            "text_conversion_scope": "images_only"
        }),
        "https://example.invalid/current",
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::InvalidArgumentValue
            && issue
                .detail
                .contains("error_code=current_request_evidence_required")
            && issue.detail.contains("field=text_conversion_scope")
    }));
}

#[test]
fn current_request_evidence_accepts_exact_current_quote_and_is_not_dispatched() {
    let result = media_download_with_scope(
        json!({
            "action": "download",
            "share": "https://example.invalid/current",
            "text_conversion_scope": "images_only",
            "_runtime_argument_evidence": {
                "text_conversion_scope": "识别图片文字"
            }
        }),
        "下载这个链接并识别图片文字 https://example.invalid/current",
    );

    assert!(result.approved);
    assert_eq!(
        result.approved_steps[0].args["text_conversion_scope"],
        "images_only"
    );
    assert!(result.approved_steps[0]
        .args
        .get("_runtime_argument_evidence")
        .is_none());
}

#[test]
fn current_request_evidence_rejects_quote_found_only_in_conversation_history() {
    let result = media_download_with_scope(
        json!({
            "action": "download",
            "share": "https://example.invalid/current",
            "text_conversion_scope": "images_only",
            "_runtime_argument_evidence": {
                "text_conversion_scope": "识别图片文字"
            }
        }),
        "https://example.invalid/current",
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        issue
            .detail
            .contains("reason=quote_not_found_in_current_request")
    }));
}

#[test]
fn current_request_evidence_blocks_direct_ocr_without_current_turn_authorization() {
    let result = media_download_with_scope(
        json!({
            "action": "ocr",
            "input_paths": ["artifacts/current.png"]
        }),
        "https://example.invalid/current",
    );

    assert!(!result.approved);
    assert!(result.issues.iter().any(|issue| {
        issue.kind == VerifyIssueKind::InvalidArgumentValue
            && issue
                .detail
                .contains("error_code=current_request_evidence_required")
            && issue.detail.contains("field=input_paths")
    }));
}
