use serde::Deserialize;
use serde_json::{json, Value};
use tracing::info;

use super::{
    append_route_reason, is_meaningful_state_patch, normalize_schema_token, parse_output_contract,
    parse_output_delivery_intent, parse_output_response_shape, parse_output_semantic_kind,
    parse_target_task_policy, parse_turn_type, ContractRepairReport, IntentExecutionRecipeOut,
    IntentNormalizerOut, IntentOutputContractOut, OutputDeliveryIntent, OutputLocatorKind,
    OutputResponseShape, OutputSemanticKind, TargetTaskPolicy, TurnType,
};
use crate::{llm_gateway, AppState, ClaimedTask};

const CONTRACT_REPAIR_JUDGE_PROMPT_LOGICAL_PATH: &str = "prompts/contract_repair_judge_prompt.md";
const EXECUTION_FAILED_STEP_CONTRACT_MARKER: &str =
    "execution_failed_step_contract_preserves_ordered_command_sequence";
const ACTIVE_TASK_INVALID_BINDING_CONTINUATION_MARKER: &str =
    "active_task_invalid_turn_binding_repaired_continuation_request";
const GENERATED_FILE_DELIVERY_RUNTIME_TARGET_MARKER: &str =
    "generated_file_delivery_allows_runtime_target";

#[derive(Debug, Deserialize)]
pub(super) struct ContractRepairJudgeOut {
    #[serde(default)]
    pub(super) apply: bool,
    #[serde(default)]
    pub(super) reason: String,
    #[serde(default)]
    pub(super) repair_target: String,
    #[serde(default)]
    pub(super) confidence: f64,
    /// Empty compatibility field accepted by the prompt schema.
    ///
    /// Repair authority comes from `needs_clarify`, `output_contract`, execution
    /// recipe, and machine markers; this field must not drive runtime routing.
    #[serde(default)]
    #[allow(dead_code)]
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
    let Some(mut output_contract) = repair.output_contract else {
        return false;
    };
    let Some(mut execution_recipe) = repair.execution_recipe else {
        return false;
    };
    if !contract_repair_reason_has_boundary_machine_authority(&repair.reason) {
        return false;
    }
    let missing_turn_binding_for_content_read =
        contract_repair_reason_requires_missing_locator_clarify(&repair.reason);
    let repaired_active_continuation = repair_reason_has_machine_marker(
        &repair.reason,
        ACTIVE_TASK_INVALID_BINDING_CONTINUATION_MARKER,
    );
    let mut needs_clarify = repair.needs_clarify;
    let mut clarify_question = repair.clarify_question;

    if repaired_active_continuation {
        needs_clarify = false;
        clarify_question.clear();
        output_contract.response_shape = OutputResponseShape::OneSentence.as_str().to_string();
        output_contract.requires_content_evidence = false;
        output_contract.delivery_required = false;
        output_contract.locator_kind = OutputLocatorKind::None.as_str().to_string();
        output_contract.delivery_intent = OutputDeliveryIntent::None.as_str().to_string();
        output_contract.contract_marker = OutputSemanticKind::None.as_str().to_string();
        output_contract.locator_hint.clear();
        execution_recipe = IntentExecutionRecipeOut::default();
    } else if missing_turn_binding_for_content_read {
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
        needs_clarify = false;
        clarify_question.clear();
        output_contract.response_shape = OutputResponseShape::Strict.as_str().to_string();
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = false;
        output_contract.locator_kind = OutputLocatorKind::None.as_str().to_string();
        output_contract.delivery_intent = OutputDeliveryIntent::None.as_str().to_string();
        output_contract.contract_marker =
            OutputSemanticKind::ExecutionFailedStep.as_str().to_string();
        output_contract.locator_hint.clear();
        execution_recipe = IntentExecutionRecipeOut::default();
    } else if generated_file_delivery_repair_allows_runtime_target(&repair.reason, &output_contract)
    {
        needs_clarify = false;
        clarify_question.clear();
        output_contract.response_shape = OutputResponseShape::FileToken.as_str().to_string();
        output_contract.requires_content_evidence = true;
        output_contract.delivery_required = true;
        output_contract.locator_kind = OutputLocatorKind::CurrentWorkspace.as_str().to_string();
        output_contract.delivery_intent = OutputDeliveryIntent::FileSingle.as_str().to_string();
        output_contract.contract_marker = OutputSemanticKind::GeneratedFileDelivery
            .as_str()
            .to_string();
        execution_recipe = IntentExecutionRecipeOut::default();
    }

    if !contract_repair_judge_output_is_schema_backed(
        out.output_contract.as_ref(),
        out.wants_file_delivery,
        needs_clarify,
        &output_contract,
        &execution_recipe,
        &repair.reason,
    ) {
        return false;
    }

    out.needs_clarify = needs_clarify;
    out.clarify_question = clarify_question;
    if !repair.resolved_user_intent.trim().is_empty() {
        out.resolved_user_intent = repair.resolved_user_intent;
    }
    out.wants_file_delivery = repaired_contract_wants_file_delivery(&output_contract);
    append_contract_repair_machine_tokens(
        &mut out.reason,
        &repair.repair_target,
        &output_contract,
        &repair.reason,
    );
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
    append_route_reason(&mut out.reason, "contract_repair_applied");
    if !repair.reason.trim().is_empty() {
        append_route_reason(&mut out.reason, "contract_repair_note_present");
    }
    if repaired_active_continuation {
        out.turn_type = TurnType::StatusQuery.as_str().to_string();
        out.target_task_policy = TargetTaskPolicy::ReuseActive.as_str().to_string();
    }
    true
}

fn append_contract_repair_machine_tokens(
    reason: &mut String,
    repair_target: &str,
    output_contract: &IntentOutputContractOut,
    repair_reason: &str,
) {
    let explicit_target = parse_output_semantic_kind(repair_target);
    let output_target = parse_output_semantic_kind(&output_contract.contract_marker);
    let target = if explicit_target != OutputSemanticKind::None {
        explicit_target
    } else {
        output_target
    };
    if target != OutputSemanticKind::None {
        append_route_reason(
            reason,
            &format!("contract_repair_target={}", target.as_str()),
        );
    }
    if repair_reason_has_machine_marker(
        repair_reason,
        ACTIVE_TASK_INVALID_BINDING_CONTINUATION_MARKER,
    ) {
        append_route_reason(
            reason,
            "contract_repair_marker=active_task_invalid_turn_binding",
        );
    }
}

fn contract_repair_judge_output_is_schema_backed(
    original_output_contract: Option<&IntentOutputContractOut>,
    original_wants_file_delivery: bool,
    needs_clarify: bool,
    output_contract: &IntentOutputContractOut,
    execution_recipe: &IntentExecutionRecipeOut,
    reason: &str,
) -> bool {
    if contract_repair_reason_has_allowed_machine_override(reason) {
        return true;
    }
    if needs_clarify {
        return needs_clarify;
    }
    let repaired_execution_signal = repaired_contract_has_execution_signal(output_contract)
        || repaired_recipe_has_execution_signal(execution_recipe);
    if repaired_execution_signal {
        return true;
    }
    if original_contract_has_execution_signal(
        original_output_contract,
        original_wants_file_delivery,
    ) {
        return true;
    }
    false
}

fn contract_repair_reason_has_allowed_machine_override(reason: &str) -> bool {
    [
        ACTIVE_TASK_INVALID_BINDING_CONTINUATION_MARKER,
        EXECUTION_FAILED_STEP_CONTRACT_MARKER,
        GENERATED_FILE_DELIVERY_RUNTIME_TARGET_MARKER,
    ]
    .iter()
    .any(|marker| repair_reason_has_machine_marker(reason, marker))
}

fn contract_repair_reason_has_boundary_machine_authority(reason: &str) -> bool {
    contract_repair_reason_has_allowed_machine_override(reason)
        || contract_repair_reason_requires_missing_locator_clarify(reason)
}

fn repaired_contract_has_execution_signal(output_contract: &IntentOutputContractOut) -> bool {
    let contract = parse_output_contract(
        Some(output_contract.clone()),
        repaired_contract_wants_file_delivery(output_contract),
    );
    contract.requires_content_evidence
        || contract.delivery_required
        || contract.locator_kind != OutputLocatorKind::None
        || contract.delivery_intent != OutputDeliveryIntent::None
        || matches!(contract.response_shape, OutputResponseShape::FileToken)
}

fn original_contract_has_execution_signal(
    output_contract: Option<&IntentOutputContractOut>,
    wants_file_delivery: bool,
) -> bool {
    let Some(output_contract) = output_contract else {
        return wants_file_delivery;
    };
    let contract = parse_output_contract(Some(output_contract.clone()), wants_file_delivery);
    wants_file_delivery
        || contract.requires_content_evidence
        || contract.delivery_required
        || contract.locator_kind != OutputLocatorKind::None
        || contract.delivery_intent != OutputDeliveryIntent::None
        || matches!(contract.response_shape, OutputResponseShape::FileToken)
}

fn repaired_recipe_has_execution_signal(execution_recipe: &IntentExecutionRecipeOut) -> bool {
    !matches!(
        crate::execution_recipe::parse_execution_recipe_kind_text(&execution_recipe.kind),
        crate::execution_recipe::ExecutionRecipeKind::None
    )
}

fn generated_file_delivery_repair_allows_runtime_target(
    reason: &str,
    output_contract: &IntentOutputContractOut,
) -> bool {
    if !repair_reason_has_machine_marker(reason, GENERATED_FILE_DELIVERY_RUNTIME_TARGET_MARKER) {
        return false;
    }
    let contract = parse_output_contract(Some(output_contract.clone()), true);
    contract.semantic_kind_is(OutputSemanticKind::GeneratedFileDelivery)
        && contract.delivery_required
        && contract.delivery_intent == OutputDeliveryIntent::FileSingle
        && contract.response_shape == OutputResponseShape::FileToken
}

fn repair_reason_has_machine_marker(reason: &str, marker: &str) -> bool {
    reason
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
        .any(|token| token == marker)
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
