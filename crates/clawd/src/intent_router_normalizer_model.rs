use tracing::{info, warn};

use super::{
    normalize_intent_normalizer_raw_for_schema_with_report, normalizer_llm_failed_fallback_output,
    normalizer_prompt_missing_fallback_output, render_auth_policy_context,
    render_intent_normalizer_prompt_for_route, render_self_extension_runtime,
    retry_intent_normalizer_json_parse, ContractRepairReport, IntentNormalizerOut,
    IntentNormalizerOutput, INTENT_NORMALIZER_PROMPT_LOGICAL_PATH,
};
use crate::intent::surface_signals::PromptSurfaceSignals;
use crate::{llm_gateway, AppState, ClaimedTask};

pub(super) struct NormalizerModelSuccess {
    pub(super) llm_out: String,
    pub(super) llm_out_for_parse: String,
    pub(super) contract_repair_report: ContractRepairReport,
    pub(super) parsed: Option<IntentNormalizerOut>,
}

pub(super) enum NormalizerModelOutcome {
    Success(NormalizerModelSuccess),
    Fallback(IntentNormalizerOutput),
}

pub(super) async fn run_intent_normalizer_model_step(
    state: &AppState,
    task: &ClaimedTask,
    req: &str,
    surface_req: &str,
    req_surface: &PromptSurfaceSignals,
    route_view: &crate::task_context_builder::RouteContextView,
    context_bundle: &crate::task_context_builder::TaskContextBundle,
    session_snapshot: Option<&crate::conversation_state::ActiveSessionSnapshot>,
) -> NormalizerModelOutcome {
    let resolved_prompt =
        match crate::bootstrap::load_required_prompt_template_for_state_with_meta(
            state,
            INTENT_NORMALIZER_PROMPT_LOGICAL_PATH,
        ) {
            Ok(resolved) => resolved,
            Err(err) => {
                return NormalizerModelOutcome::Fallback(
                    normalizer_prompt_missing_fallback_output(state, task, req, surface_req, &err),
                );
            }
        };
    let prompt_template = resolved_prompt.template;
    let prompt_source = resolved_prompt.source;
    let prompt_version = resolved_prompt.version;
    let request_language_hint = crate::language_policy::preferred_response_language_hint(
        req,
        session_snapshot
            .and_then(|snapshot| snapshot.conversation_state.as_ref())
            .and_then(|conversation_state| conversation_state.locale_hint.as_deref()),
    );
    let auth_policy_context = render_auth_policy_context(state, task);
    let self_extension_runtime = render_self_extension_runtime(state);
    let prompt = render_intent_normalizer_prompt_for_route(
        state,
        task,
        route_view,
        context_bundle,
        &prompt_template,
        &auth_policy_context,
        &self_extension_runtime,
        &request_language_hint,
        req,
    );
    crate::log_prompt_render_with_version(
        state,
        &task.task_id,
        "intent_normalizer_prompt",
        &prompt_source,
        prompt_version.as_deref(),
        None,
    );
    let llm_out = match llm_gateway::run_with_fallback_with_prompt_source(
        state,
        task,
        &prompt,
        &prompt_source,
    )
    .await
    {
        Ok(v) => v,
        Err(err) => {
            return NormalizerModelOutcome::Fallback(normalizer_llm_failed_fallback_output(
                state,
                task,
                req,
                surface_req,
                req_surface,
                &err,
            ));
        }
    };
    let (llm_out_for_parse, contract_repair_report) =
        normalize_intent_normalizer_raw_for_schema_with_report(&llm_out, req);
    let parsed = crate::prompt_utils::validate_against_schema::<IntentNormalizerOut>(
        &llm_out_for_parse,
        crate::prompt_utils::PromptSchemaId::IntentNormalizer,
    );
    if let Ok(validated) = &parsed {
        if !validated.raw_parse_ok {
            info!(
                "{} intent_normalizer task_id={} parse_recovery=schema_repair contract_repair_source={} contract_repair_detail={} input={}",
                crate::highlight_tag("routing"),
                task.task_id,
                contract_repair_report.source_csv(),
                contract_repair_report.detail_csv(),
                crate::truncate_for_log(req)
            );
        }
    }
    let parsed = match parsed {
        Ok(validated) => Some(validated.value),
        Err(err) => {
            warn!(
                "intent_normalizer schema parse failed, falling back to safe clarify: task_id={} err={} normalized_raw={}",
                task.task_id,
                err,
                crate::truncate_for_log(&llm_out_for_parse)
            );
            None
        }
    };
    let (parsed, contract_repair_report) = match parsed {
        Some(parsed) => (Some(parsed), contract_repair_report),
        None => {
            if let Some((retry_out, retry_report)) = retry_intent_normalizer_json_parse(
                state,
                task,
                route_view,
                context_bundle,
                &auth_policy_context,
                &request_language_hint,
                req,
                &prompt_source,
                &contract_repair_report,
            )
            .await
            {
                (Some(retry_out), retry_report)
            } else {
                (None, contract_repair_report)
            }
        }
    };
    NormalizerModelOutcome::Success(NormalizerModelSuccess {
        llm_out,
        llm_out_for_parse,
        contract_repair_report,
        parsed,
    })
}
