use super::*;

fn contract_existence(hint: &str) -> IntentOutputContract {
    IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::OneSentence,
        semantic_kind: OutputSemanticKind::ExistenceWithPath,
        locator_hint: hint.to_string(),
        ..IntentOutputContract::default()
    }
}

#[test]
fn pass_for_default_contract() {
    let v = verify_output_contract(&IntentOutputContract::default(), "anything goes", "what?");
    assert_eq!(v, OutputContractVerdict::Pass);
}

#[test]
fn reject_for_empty_candidate() {
    let v = verify_output_contract(&contract_existence("rustclaw.service"), "  ", "?");
    assert_eq!(v.owner_layer(), "output_contract_verifier");
    assert_eq!(v.reason_code(), Some("candidate_empty"));
    assert!(matches!(v, OutputContractVerdict::Reject { .. }));
}

#[test]
fn existence_with_path_no_longer_autoprepends_or_hard_rejects() {
    assert_eq!(
        verify_output_contract(
            &contract_existence("rustclaw.service"),
            "/home/guagua/rustclaw/rustclaw.service",
            "?",
        ),
        OutputContractVerdict::Pass
    );
    assert_eq!(
        verify_output_contract(
            &contract_existence("rustclaw.service"),
            "这是一个 systemd 服务单元文件，用于在系统启动时拉起 rustclaw 守护进程。",
            "检查仓库里有没有 rustclaw.service",
        ),
        OutputContractVerdict::Pass
    );
}

#[test]
fn directory_names_allows_parent_dirs_with_dotted_intermediate_component() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Strict,
        semantic_kind: OutputSemanticKind::None,
        ..IntentOutputContract::default()
    };

    let v = verify_output_contract(
        &contract,
        ".\ncomponent_start\ndata/vendor/whisper.cpp/scripts/apple",
        "Find directories containing .sh files",
    );

    assert_eq!(v, OutputContractVerdict::Pass);
}

#[test]
fn pass_scalar_count_for_pure_integer() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..IntentOutputContract::default()
    };
    let v = verify_output_contract(&contract, "3", "?");
    assert_eq!(v, OutputContractVerdict::Pass);
}

#[test]
fn reshape_scalar_count_extracts_sole_int_from_multiline_candidate() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..IntentOutputContract::default()
    };
    let candidate = "目录检查完成。\n一共是 5 个。";
    let v = verify_output_contract(&contract, candidate, "?");
    match v {
        OutputContractVerdict::Reshape {
            reason_code,
            reshaped,
            ..
        } => {
            assert_eq!(reason_code, "scalar_count_extracted_unique_integer");
            assert_eq!(reshaped, "5");
        }
        other => panic!("expected Reshape extracting int, got: {other:?}"),
    }
}

#[test]
fn reshape_scalar_count_extracts_sole_int_from_single_line_candidate() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..IntentOutputContract::default()
    };
    let v = verify_output_contract(&contract, "3，当前范围内共有 3 个项目。", "?");
    match v {
        OutputContractVerdict::Reshape { reshaped, .. } => assert_eq!(reshaped, "3"),
        other => panic!("expected Reshape extracting int, got: {other:?}"),
    }
}

#[test]
fn pass_scalar_count_missing_target_failure_without_integer() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        locator_hint: "configs/config_copy".to_string(),
        ..IntentOutputContract::default()
    };
    let v = verify_output_contract(
        &contract,
        "`configs/config_copy` 不存在，无法统计匹配项数量。",
        "?",
    );
    assert_eq!(v, OutputContractVerdict::Pass);
}

#[test]
fn pass_scalar_count_missing_target_failure_with_digits_in_path() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        locator_hint: "configs/config_copy_2026".to_string(),
        ..IntentOutputContract::default()
    };
    let v = verify_output_contract(
        &contract,
        "`configs/config_copy_2026` does not exist, so the matching item count cannot be computed.",
        "?",
    );
    assert_eq!(v, OutputContractVerdict::Pass);
}

#[test]
fn reject_scalar_count_when_no_integer_at_all() {
    let contract = IntentOutputContract {
        exact_sentence_count: None,
        response_shape: OutputResponseShape::Scalar,
        semantic_kind: OutputSemanticKind::ScalarCount,
        ..IntentOutputContract::default()
    };
    let v = verify_output_contract(&contract, "数不清", "?");
    assert_eq!(
        v.reason_code(),
        Some("scalar_count_missing_integer_literal")
    );
    assert!(matches!(v, OutputContractVerdict::Reject { .. }));
}
