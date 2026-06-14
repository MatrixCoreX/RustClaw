use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use super::{
    append_route_reason, is_meaningful_state_patch, normalize_schema_token,
    output_contract_structured_config_path, parse_first_layer_decision_text, parse_output_contract,
    parse_output_delivery_intent, parse_output_response_shape, parse_target_task_policy,
    parse_turn_type, ContractRepairReport, FirstLayerDecision, IntentExecutionRecipeOut,
    IntentNormalizerOut, IntentOutputContract, IntentOutputContractOut, OutputDeliveryIntent,
    OutputLocatorKind, OutputResponseShape, OutputSemanticKind, TargetTaskPolicy, TurnType,
};
use crate::{llm_gateway, AppState, ClaimedTask};

const CONTRACT_REPAIR_JUDGE_PROMPT_LOGICAL_PATH: &str = "prompts/contract_repair_judge_prompt.md";
const EXECUTION_FAILED_STEP_CONTRACT_MARKER: &str =
    "execution_failed_step_contract_preserves_ordered_command_sequence";
const ACTIVE_TASK_INVALID_BINDING_CONTINUATION_MARKER: &str =
    "active_task_invalid_turn_binding_repaired_continuation_request";
const GENERATED_FILE_DELIVERY_RUNTIME_TARGET_MARKER: &str =
    "generated_file_delivery_allows_runtime_target";
const GENERATED_FILE_DELIVERY_ATTACHMENT_REPAIR_MARKER: &str =
    "generated_file_delivery_cleared_spurious_attachment_processing";

#[derive(Debug, Deserialize)]
pub(super) struct ContractRepairJudgeOut {
    #[serde(default)]
    pub(super) apply: bool,
    #[serde(default)]
    pub(super) reason: String,
    #[serde(default)]
    pub(super) confidence: f64,
    #[serde(default)]
    pub(super) decision: String,
    #[serde(default)]
    pub(super) needs_clarify: bool,
    #[serde(default)]
    pub(super) clarify_question: String,
    #[serde(default)]
    pub(super) resolved_user_intent: String,
    #[serde(default)]
    pub(super) output_contract: Option<IntentOutputContractOut>,
    #[serde(default)]
    pub(super) execution_recipe: Option<IntentExecutionRecipeOut>,
    #[serde(default)]
    pub(super) turn_type: String,
    #[serde(default)]
    pub(super) target_task_policy: String,
    #[serde(default)]
    pub(super) state_patch: Option<Value>,
}

pub(super) async fn run_contract_repair_judge(
    state: &AppState,
    task: &ClaimedTask,
    user_request: &str,
    raw_normalizer_output: &str,
    normalized_route_json: &str,
    repair_report: &ContractRepairReport,
    repair_context: &str,
) -> Option<ContractRepairJudgeOut> {
    let resolved = match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
        state,
        CONTRACT_REPAIR_JUDGE_PROMPT_LOGICAL_PATH,
    ) {
        Ok(resolved) => resolved,
        Err(err) => {
            info!(
                "{} contract_repair_judge prompt_missing task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    let prompt = crate::render_prompt_template(
        &resolved.template,
        &[
            ("__REQUEST__", user_request.trim()),
            (
                "__NORMALIZED_ROUTE_JSON__",
                &crate::truncate_for_log(normalized_route_json),
            ),
            ("__CONTRACT_REPAIR_SOURCE__", &repair_report.source_csv()),
            ("__CONTRACT_REPAIR_DETAIL__", &repair_report.detail_csv()),
            ("__CONTRACT_REPAIR_CONTEXT__", repair_context),
            (
                "__RAW_NORMALIZER_OUTPUT__",
                &crate::truncate_for_log(raw_normalizer_output),
            ),
        ],
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "contract_repair_judge_prompt",
        &resolved.source,
        resolved.version.as_deref(),
        None,
    );
    let prompt_source = resolved.source;
    let llm_out = match llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(value) => value,
        Err(err) => {
            info!(
                "{} contract_repair_judge llm_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            return None;
        }
    };
    match crate::prompt_utils::validate_against_schema::<ContractRepairJudgeOut>(
        &llm_out,
        crate::prompt_utils::PromptSchemaId::ContractRepairJudge,
    ) {
        Ok(validated) => {
            if !validated.raw_parse_ok || validated.schema_normalized {
                info!(
                    "{} contract_repair_judge schema_parse_recovery task_id={} raw_parse_ok={} schema_normalized={}",
                    crate::highlight_tag("routing"),
                    task.task_id,
                    validated.raw_parse_ok,
                    validated.schema_normalized
                );
            }
            Some(validated.value)
        }
        Err(err) => {
            info!(
                "{} contract_repair_judge schema_validation_failed task_id={} err={}",
                crate::highlight_tag("routing"),
                task.task_id,
                err
            );
            None
        }
    }
}

pub(super) fn apply_contract_repair_judge_output(
    out: &mut IntentNormalizerOut,
    repair: ContractRepairJudgeOut,
) -> bool {
    if !repair.apply || repair.confidence < 0.60 {
        return false;
    }
    let Some(mut decision) = parse_first_layer_decision_text(&repair.decision) else {
        return false;
    };
    let Some(mut output_contract) = repair.output_contract else {
        return false;
    };
    let Some(mut execution_recipe) = repair.execution_recipe else {
        return false;
    };
    let preserved_structured_config_keys = preserve_structured_config_key_contract_during_repair(
        out.output_contract.as_ref(),
        &mut output_contract,
    );
    let preserved_structured_scalar_field = preserve_structured_scalar_field_contract_during_repair(
        out.output_contract.as_ref(),
        &mut output_contract,
    );
    let missing_turn_binding_for_content_read =
        contract_repair_reason_requires_missing_locator_clarify(&repair.reason);
    let repaired_active_continuation = repair_reason_has_machine_marker(
        &repair.reason,
        ACTIVE_TASK_INVALID_BINDING_CONTINUATION_MARKER,
    );
    let mut needs_clarify = repair.needs_clarify;
    let mut clarify_question = repair.clarify_question;

    if repaired_active_continuation {
        decision = FirstLayerDecision::DirectAnswer;
        needs_clarify = false;
        clarify_question.clear();
        output_contract.response_shape = OutputResponseShape::OneSentence.as_str().to_string();
        output_contract.requires_content_evidence = false;
        output_contract.delivery_required = false;
        output_contract.locator_kind = OutputLocatorKind::None.as_str().to_string();
        output_contract.delivery_intent = OutputDeliveryIntent::None.as_str().to_string();
        output_contract.semantic_kind = OutputSemanticKind::None.as_str().to_string();
        output_contract.locator_hint.clear();
        execution_recipe = IntentExecutionRecipeOut::default();
    } else if missing_turn_binding_for_content_read {
        decision = FirstLayerDecision::Clarify;
        needs_clarify = true;
        clarify_question.clear();
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.locator_kind = "none".to_string();
        output_contract.delivery_intent = "none".to_string();
        output_contract.locator_hint.clear();
        execution_recipe = IntentExecutionRecipeOut::default();
    } else if repair_reason_has_machine_marker(
        &repair.reason,
        EXECUTION_FAILED_STEP_CONTRACT_MARKER,
    ) {
        decision = FirstLayerDecision::PlannerExecute;
        needs_clarify = false;
        clarify_question.clear();
        output_contract.response_shape = OutputResponseShape::Strict.as_str().to_string();
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.locator_kind = OutputLocatorKind::None.as_str().to_string();
        output_contract.delivery_intent = OutputDeliveryIntent::None.as_str().to_string();
        output_contract.semantic_kind =
            OutputSemanticKind::ExecutionFailedStep.as_str().to_string();
        output_contract.locator_hint.clear();
        execution_recipe = IntentExecutionRecipeOut::default();
    } else if generated_file_delivery_repair_allows_runtime_target(&repair.reason, &output_contract)
    {
        decision = FirstLayerDecision::PlannerExecute;
        needs_clarify = false;
        clarify_question.clear();
        output_contract.response_shape = OutputResponseShape::FileToken.as_str().to_string();
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = true;
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace.as_str().to_string();
        output_contract.delivery_intent = OutputDeliveryIntent::FileSingle.as_str().to_string();
        output_contract.semantic_kind = OutputSemanticKind::GeneratedFileDelivery
            .as_str()
            .to_string();
        execution_recipe = IntentExecutionRecipeOut::default();
    }

    out.decision = decision.as_str().to_string();
    out.needs_clarify = needs_clarify;
    out.clarify_question = clarify_question;
    if !repair.resolved_user_intent.trim().is_empty() {
        out.resolved_user_intent = repair.resolved_user_intent;
    }
    out.wants_file_delivery = repaired_contract_wants_file_delivery(&output_contract);
    out.output_contract = Some(output_contract);
    out.execution_recipe = Some(execution_recipe);
    if repaired_active_continuation {
        out.state_patch = None;
    } else if missing_turn_binding_for_content_read {
        out.state_patch = Some(json!({"deictic_reference": {"target": "missing_locator"}}));
    } else if repair
        .state_patch
        .as_ref()
        .is_some_and(is_meaningful_state_patch)
    {
        out.state_patch = repair.state_patch;
    }
    out.confidence = repair.confidence.clamp(0.0, 1.0);
    let repaired_turn_type = normalize_schema_token(&repair.turn_type);
    if repaired_turn_type.is_empty() {
        if parse_turn_type(&out.turn_type).is_none() {
            out.turn_type.clear();
        }
    } else if parse_turn_type(&repaired_turn_type).is_some() {
        out.turn_type = repaired_turn_type;
    }
    let repaired_target_task_policy = normalize_schema_token(&repair.target_task_policy);
    if repaired_target_task_policy.is_empty() {
        if parse_target_task_policy(&out.target_task_policy).is_none() {
            out.target_task_policy.clear();
        }
    } else if parse_target_task_policy(&repaired_target_task_policy).is_some() {
        out.target_task_policy = repaired_target_task_policy;
    }
    if repair.reason.trim().is_empty() {
        append_route_reason(&mut out.reason, "llm_semantic_contract_repair");
    } else {
        append_route_reason(
            &mut out.reason,
            &format!("llm_semantic_contract_repair:{}", repair.reason.trim()),
        );
    }
    if preserved_structured_config_keys {
        append_route_reason(&mut out.reason, "structured_config_key_contract_preserved");
    }
    if preserved_structured_scalar_field {
        append_route_reason(
            &mut out.reason,
            "structured_scalar_field_contract_preserved",
        );
    }
    if repaired_active_continuation {
        out.turn_type = TurnType::StatusQuery.as_str().to_string();
        out.target_task_policy = TargetTaskPolicy::ReuseActive.as_str().to_string();
    }
    true
}

fn generated_file_delivery_repair_allows_runtime_target(
    reason: &str,
    output_contract: &IntentOutputContractOut,
) -> bool {
    if !repair_reason_has_machine_marker(reason, GENERATED_FILE_DELIVERY_RUNTIME_TARGET_MARKER) {
        return false;
    }
    let contract = parse_output_contract(Some(output_contract.clone()), true);
    contract.semantic_kind == OutputSemanticKind::GeneratedFileDelivery
        && contract.delivery_required
        && contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && contract.response_shape == OutputResponseShape::FileToken
}

pub(super) fn clear_spurious_generated_file_delivery_attachment_processing(
    attachment_processing_required: &mut bool,
    output_contract: &IntentOutputContract,
    wants_file_delivery: bool,
) -> Option<&'static str> {
    if !*attachment_processing_required {
        return None;
    }
    let delivery_signal = wants_file_delivery
        || output_contract.delivery_required
        || output_contract.response_shape == OutputResponseShape::FileToken
        || output_contract.delivery_intent == OutputDeliveryIntent::FileSingle;
    if delivery_signal
        && output_contract.semantic_kind == OutputSemanticKind::GeneratedFileDelivery
        && output_contract.delivery_required
        && output_contract.response_shape == OutputResponseShape::FileToken
        && output_contract.delivery_intent == OutputDeliveryIntent::FileSingle
    {
        *attachment_processing_required = false;
        Some(GENERATED_FILE_DELIVERY_ATTACHMENT_REPAIR_MARKER)
    } else {
        None
    }
}

fn repair_reason_has_machine_marker(reason: &str, marker: &str) -> bool {
    reason
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|token| token == marker)
}

fn preserve_structured_config_key_contract_during_repair(
    current: Option<&IntentOutputContractOut>,
    repair: &mut IntentOutputContractOut,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    let current_contract = parse_output_contract(Some(current.clone()), false);
    let repaired_contract = parse_output_contract(Some(repair.clone()), false);
    if current_contract.semantic_kind != OutputSemanticKind::StructuredKeys
        || repaired_contract.semantic_kind != OutputSemanticKind::None
        || current_contract.delivery_required
        || repaired_contract.delivery_required
        || !matches!(current_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            repaired_contract.delivery_intent,
            OutputDeliveryIntent::None
        )
        || output_contract_structured_config_path(&current_contract).is_none()
        || output_contract_structured_config_path(&repaired_contract).is_none()
    {
        return false;
    }
    repair.semantic_kind = OutputSemanticKind::StructuredKeys.as_str().to_string();
    repair.requires_content_evidence = true;
    repair.delivery_required = false;
    repair.delivery_intent = OutputDeliveryIntent::None.as_str().to_string();
    if matches!(
        repaired_contract.response_shape,
        OutputResponseShape::Free | OutputResponseShape::OneSentence
    ) {
        repair.response_shape = OutputResponseShape::Strict.as_str().to_string();
    }
    true
}

fn preserve_structured_scalar_field_contract_during_repair(
    current: Option<&IntentOutputContractOut>,
    repair: &mut IntentOutputContractOut,
) -> bool {
    let Some(current) = current else {
        return false;
    };
    let current_contract = parse_output_contract(Some(current.clone()), false);
    let repaired_contract = parse_output_contract(Some(repair.clone()), false);
    if current_contract.semantic_kind != OutputSemanticKind::None
        || repaired_contract.semantic_kind != OutputSemanticKind::ContentExcerptSummary
        || !matches!(
            current_contract.response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::Strict
        )
        || !matches!(
            repaired_contract.response_shape,
            OutputResponseShape::Scalar | OutputResponseShape::Strict
        )
        || !current_contract.requires_content_evidence
        || current_contract.delivery_required
        || repaired_contract.delivery_required
        || !matches!(current_contract.delivery_intent, OutputDeliveryIntent::None)
        || !matches!(
            repaired_contract.delivery_intent,
            OutputDeliveryIntent::None
        )
        || output_contract_structured_config_path(&current_contract).is_none()
        || output_contract_structured_config_path(&repaired_contract).is_none()
    {
        return false;
    }
    repair.semantic_kind = OutputSemanticKind::None.as_str().to_string();
    repair.requires_content_evidence = true;
    repair.delivery_required = false;
    repair.delivery_intent = OutputDeliveryIntent::None.as_str().to_string();
    if matches!(
        repaired_contract.response_shape,
        OutputResponseShape::Free | OutputResponseShape::OneSentence
    ) {
        repair.response_shape = current_contract.response_shape.as_str().to_string();
    }
    true
}

fn contract_repair_reason_requires_missing_locator_clarify(reason: &str) -> bool {
    reason
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|token| token == "execution_recipe_untrusted_text_ignored_and_turn_binding_missing_for_content_read")
}

fn repaired_contract_wants_file_delivery(contract: &IntentOutputContractOut) -> bool {
    contract.delivery_required
        || matches!(
            parse_output_response_shape(&contract.response_shape),
            OutputResponseShape::FileToken
        )
        || !matches!(
            parse_output_delivery_intent(&contract.delivery_intent),
            OutputDeliveryIntent::None
        )
}
