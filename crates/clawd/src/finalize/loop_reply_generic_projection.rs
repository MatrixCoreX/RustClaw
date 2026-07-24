use std::collections::BTreeMap;

use claw_core::capability_result::{CapabilityResultEnvelope, CapabilityResultStatus};
use serde_json::{Map as JsonMap, Value};

use crate::agent_engine::{AgentRunContext, LoopState};
use crate::{AppState, AskReply, ClaimedTask};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GenericProjectionIssueCode {
    MissingSelector,
    MissingValue,
    AmbiguousValue,
    ContradictoryDelivery,
}

impl GenericProjectionIssueCode {
    fn as_str(self) -> &'static str {
        match self {
            Self::MissingSelector => "generic_projection_missing_selector",
            Self::MissingValue => "generic_projection_missing_value",
            Self::AmbiguousValue => "generic_projection_ambiguous_value",
            Self::ContradictoryDelivery => "generic_projection_contradictory_delivery",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct GenericProjectionIssue {
    pub(super) code: GenericProjectionIssueCode,
    pub(super) selectors: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum GenericProjection {
    NotApplicable,
    Projected { text: String, evidence_count: usize },
    RepairIssue(GenericProjectionIssue),
}

pub(crate) fn project_capability_results(
    route: &crate::IntentOutputContract,
    results: &[CapabilityResultEnvelope],
) -> GenericProjection {
    if !strict_projection_requested(route) {
        return GenericProjection::NotApplicable;
    }

    if route.requests_single_file_delivery() {
        return project_single_artifact(results);
    }

    let Some(selectors) = route
        .selection
        .structured_field_selector
        .as_deref()
        .and_then(crate::machine_selector::exact_machine_field_selector)
    else {
        return GenericProjection::RepairIssue(GenericProjectionIssue {
            code: GenericProjectionIssueCode::MissingSelector,
            selectors: Vec::new(),
        });
    };

    let mut projected = BTreeMap::new();
    for selector in &selectors {
        match unique_selected_value(results, selector) {
            Ok(value) => {
                projected.insert(selector.clone(), value);
            }
            Err(code) => {
                return GenericProjection::RepairIssue(GenericProjectionIssue {
                    code,
                    selectors: vec![selector.clone()],
                });
            }
        }
    }

    let value = if selectors.len() == 1 {
        projected
            .remove(&selectors[0])
            .expect("selected field must be present")
    } else {
        Value::Object(projected.into_iter().collect::<JsonMap<_, _>>())
    };
    let Some(text) = render_projected_value(route, value) else {
        return GenericProjection::RepairIssue(GenericProjectionIssue {
            code: GenericProjectionIssueCode::MissingValue,
            selectors,
        });
    };
    GenericProjection::Projected {
        text,
        evidence_count: results
            .iter()
            .filter(|result| result.status == CapabilityResultStatus::Ok)
            .map(|result| result.evidence.len())
            .sum(),
    }
}

pub(crate) fn strict_capability_projection_ready(
    route: &crate::IntentOutputContract,
    loop_state: &LoopState,
) -> bool {
    matches!(
        project_capability_results(route, &loop_state.capability_results),
        GenericProjection::Projected { .. }
    )
}

pub(crate) fn record_strict_capability_projection_issue(
    route: &crate::IntentOutputContract,
    loop_state: &mut LoopState,
) -> bool {
    let GenericProjection::RepairIssue(issue) =
        project_capability_results(route, &loop_state.capability_results)
    else {
        return false;
    };
    record_projection_issue(loop_state, &issue);
    true
}

pub(super) async fn finalize_generic_machine_projection(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
) -> Option<AskReply> {
    let route = agent_run_context.and_then(AgentRunContext::output_contract)?;
    match project_capability_results(route, &loop_state.capability_results) {
        GenericProjection::NotApplicable => None,
        GenericProjection::RepairIssue(issue) => {
            record_projection_issue(loop_state, &issue);
            record_projection_renderer_trace(task, loop_state, false, Some(issue.code.as_str()));
            None
        }
        GenericProjection::Projected {
            text,
            evidence_count,
        } => {
            if let Some(candidate) = loop_state
                .delivery_messages
                .last()
                .or(loop_state.last_user_visible_respond.as_ref())
                .map(String::as_str)
                .filter(|candidate| !candidate.trim().is_empty())
            {
                if candidate_contradicts_projection(candidate, &text) {
                    record_projection_issue(
                        loop_state,
                        &GenericProjectionIssue {
                            code: GenericProjectionIssueCode::ContradictoryDelivery,
                            selectors: route
                                .selection
                                .structured_field_selector
                                .as_deref()
                                .and_then(crate::machine_selector::exact_machine_field_selector)
                                .unwrap_or_default(),
                        },
                    );
                }
            }
            loop_state.delivery_messages = vec![text.clone()];
            loop_state.last_user_visible_respond = Some(text.clone());
            record_projection_renderer_trace(task, loop_state, true, None);
            let summary = crate::task_journal::TaskJournalFinalizerSummary {
                stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
                disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
                parsed: true,
                contract_ok: true,
                completion_ok: Some(true),
                grounded_ok: Some(true),
                format_ok: Some(true),
                needs_clarify: Some(false),
                used_evidence_ids_count: evidence_count,
                ..Default::default()
            };
            let delivery_messages = vec![text.clone()];
            let delivery_consistent =
                crate::task_journal::delivery_payload_consistent(&text, &delivery_messages);
            let journal = crate::finalize::build_terminal_from_loop_state(
                state,
                task,
                user_text,
                loop_state,
                agent_run_context,
                Some(summary),
                delivery_consistent,
                &text,
                crate::task_journal::TaskJournalFinalStatus::Success,
            )
            .await;
            Some(
                AskReply::non_llm(text)
                    .with_messages(delivery_messages)
                    .with_task_journal(journal),
            )
        }
    }
}

fn strict_projection_requested(route: &crate::IntentOutputContract) -> bool {
    route.requests_single_file_delivery()
        || route.requests_exact_structured_fields()
        || route.requests_exact_list()
        || route.requests_exact_command_output()
}

fn unique_selected_value(
    results: &[CapabilityResultEnvelope],
    selector: &str,
) -> Result<Value, GenericProjectionIssueCode> {
    let mut selected = Vec::<Value>::new();
    for result in results
        .iter()
        .filter(|result| result.status == CapabilityResultStatus::Ok)
    {
        let Some(value) = crate::capability_result::selected_result_value(result, selector) else {
            continue;
        };
        if value.is_null() || value.as_str().is_some_and(|text| text.trim().is_empty()) {
            continue;
        }
        if !selected.iter().any(|existing| existing == value) {
            selected.push(value.clone());
        }
    }
    match selected.len() {
        0 => Err(GenericProjectionIssueCode::MissingValue),
        1 => Ok(selected.remove(0)),
        _ => Err(GenericProjectionIssueCode::AmbiguousValue),
    }
}

fn project_single_artifact(results: &[CapabilityResultEnvelope]) -> GenericProjection {
    let mut paths = Vec::new();
    let mut evidence_count = 0usize;
    for result in results
        .iter()
        .filter(|result| result.status == CapabilityResultStatus::Ok)
    {
        evidence_count += result.evidence.len();
        for path in result
            .artifacts
            .iter()
            .filter_map(|artifact| artifact.path.as_deref())
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            if !paths.iter().any(|existing| existing == path) {
                paths.push(path.to_string());
            }
        }
    }
    match paths.len() {
        0 => GenericProjection::RepairIssue(GenericProjectionIssue {
            code: GenericProjectionIssueCode::MissingValue,
            selectors: vec!["artifact.path".to_string()],
        }),
        1 => GenericProjection::Projected {
            text: format!("FILE:{}", paths.remove(0)),
            evidence_count,
        },
        _ => GenericProjection::RepairIssue(GenericProjectionIssue {
            code: GenericProjectionIssueCode::AmbiguousValue,
            selectors: vec!["artifact.path".to_string()],
        }),
    }
}

fn render_projected_value(route: &crate::IntentOutputContract, mut value: Value) -> Option<String> {
    if route.requests_exact_list() {
        let Value::Array(mut items) = value else {
            return None;
        };
        if let Some(limit) = route.selection.list_selector.limit {
            items.truncate(usize::try_from(limit).unwrap_or(usize::MAX));
        }
        let rendered = items
            .into_iter()
            .filter_map(exact_value_text)
            .collect::<Vec<_>>();
        return (!rendered.is_empty()).then(|| rendered.join("\n"));
    }
    exact_value_text(value.take())
}

fn exact_value_text(value: Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) => (!text.trim().is_empty()).then_some(text),
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        Value::Array(_) | Value::Object(_) => serde_json::to_string(&value).ok(),
    }
}

fn candidate_contradicts_projection(candidate: &str, projection: &str) -> bool {
    let candidate = candidate.trim();
    let projection = projection.trim();
    if candidate == projection {
        return false;
    }
    match (
        serde_json::from_str::<Value>(candidate),
        serde_json::from_str::<Value>(projection),
    ) {
        (Ok(candidate), Ok(projection)) => candidate != projection,
        _ => true,
    }
}

fn record_projection_issue(loop_state: &mut LoopState, issue: &GenericProjectionIssue) {
    let payload = serde_json::json!({
        "schema_version": 1,
        "kind": "generic_projection_repair_issue",
        "code": issue.code.as_str(),
        "selectors": issue.selectors,
    });
    let serialized = payload.to_string();
    let unchanged = loop_state
        .output_vars
        .get("finalizer.generic_projection_issue")
        .is_some_and(|existing| existing == &serialized);
    loop_state
        .output_vars
        .insert("finalizer.generic_projection_issue".to_string(), serialized);
    if !unchanged {
        loop_state.task_observations.push(payload);
    }
}

fn record_projection_renderer_trace(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    rendered: bool,
    failure_reason: Option<&'static str>,
) {
    let Some(renderer) = super::renderer_registry::renderers_for_shape_class(
        super::renderer_registry::FinalizerRendererShapeClass::FinalAnswerShape,
    )
    .find(|renderer| renderer.key == "generic_schema_projection") else {
        return;
    };
    let mut evidence_refs = loop_state
        .capability_results
        .iter()
        .flat_map(|result| result.evidence.iter())
        .map(|evidence| format!("evidence:{}", evidence.id))
        .collect::<Vec<_>>();
    evidence_refs.sort();
    evidence_refs.dedup();
    if evidence_refs.is_empty() {
        evidence_refs.push(format!("task:{}", task.task_id));
    }
    super::renderer_registry::record_renderer_trace(
        loop_state,
        renderer,
        rendered,
        evidence_refs,
        failure_reason,
    );
}

#[cfg(test)]
#[path = "loop_reply_generic_projection_tests.rs"]
mod tests;
