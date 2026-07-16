use crate::agent_engine::{append_delivery_message, AgentRunContext, LoopState};
use crate::executor::StepExecutionStatus;
use crate::{AppState, ClaimedTask};

use super::{current_user_visible_delivery_text, log_deterministic_delivery_record};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GitMachineField {
    Branch,
    WorktreeState,
    ChangedCount,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct GitMachineState {
    branch: Option<String>,
    worktree_state: Option<String>,
    changed_count: Option<usize>,
}

fn route_git_repository_state_requires_language_synthesis(
    route: &crate::IntentOutputContract,
) -> bool {
    route_requests_git_repository_state(route)
        && route.requires_content_evidence
        && !route.delivery_required
        && (matches!(
            route.response_shape,
            crate::OutputResponseShape::Free | crate::OutputResponseShape::OneSentence
        ) || route.exact_sentence_count.is_some())
}

pub(super) async fn replace_git_repository_state_machine_delivery_with_observed_synthesis(
    state: &AppState,
    task: &ClaimedTask,
    user_text: &str,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if !route_git_repository_state_requires_language_synthesis(route) {
        return false;
    }
    let Some(current_delivery) = current_user_visible_delivery_text(loop_state) else {
        return false;
    };
    if !crate::agent_engine::observed_output::answer_is_git_repository_state_machine_summary(
        current_delivery,
    ) {
        return false;
    }
    match crate::agent_engine::observed_output::try_synthesize_answer_from_observed_output(
        state,
        task,
        user_text,
        loop_state,
        agent_run_context,
    )
    .await
    {
        Ok(Some((answer, summary)))
            if matches!(
                summary.disposition,
                Some(crate::finalize::FinalizerDisposition::QualifiedCompletion)
            ) && !answer.trim().is_empty()
                && !crate::agent_engine::observed_output::answer_is_git_repository_state_machine_summary(
                    &answer,
                ) =>
        {
            loop_state.delivery_messages.clear();
            append_delivery_message(
                &task.task_id,
                &mut loop_state.delivery_messages,
                answer.clone(),
            );
            loop_state.last_user_visible_respond = Some(answer);
            *finalizer_summary = Some(summary);
            log_deterministic_delivery_record(
                &task.task_id,
                "replace_git_machine_summary_with_observed_synthesis",
                "replaced",
                agent_run_context,
                loop_state.executed_step_results.len(),
            );
            true
        }
        Ok(Some((_answer, summary))) => {
            if finalizer_summary.is_none() {
                *finalizer_summary = Some(summary);
            }
            false
        }
        Ok(None) => false,
        Err(err) => {
            tracing::warn!(
                "git_machine_summary_observed_synthesis_failed task_id={} err={}",
                task.task_id,
                err
            );
            false
        }
    }
}

pub(super) fn replace_git_repository_state_delivery_with_requested_machine_fields(
    task: &ClaimedTask,
    loop_state: &mut LoopState,
    agent_run_context: Option<&AgentRunContext>,
    finalizer_summary: &mut Option<crate::task_journal::TaskJournalFinalizerSummary>,
) -> bool {
    let Some(route) = agent_run_context.and_then(|ctx| ctx.output_contract()) else {
        return false;
    };
    if !route_requests_git_repository_state(route)
        || route.delivery_required
        || route.response_shape != crate::OutputResponseShape::Strict
    {
        return false;
    }
    let requested_fields = requested_git_machine_fields(agent_run_context);
    if requested_fields.is_empty() {
        return false;
    }
    let Some(state) = latest_git_machine_state(loop_state) else {
        return false;
    };
    let Some(answer) = render_requested_git_machine_fields(&requested_fields, &state) else {
        return false;
    };
    let current = current_user_visible_delivery_text(loop_state)
        .map(str::trim)
        .unwrap_or_default();
    if current == answer {
        loop_state.last_user_visible_respond = Some(answer);
        return true;
    }
    loop_state.delivery_messages.clear();
    append_delivery_message(
        &task.task_id,
        &mut loop_state.delivery_messages,
        answer.clone(),
    );
    loop_state.last_user_visible_respond = Some(answer);
    *finalizer_summary = Some(crate::task_journal::TaskJournalFinalizerSummary {
        stage: Some(crate::task_journal::TaskJournalFinalizerStage::ObservedGeneric),
        disposition: Some(crate::finalize::FinalizerDisposition::QualifiedCompletion),
        parsed: true,
        contract_ok: true,
        completion_ok: Some(true),
        grounded_ok: Some(true),
        format_ok: Some(true),
        needs_clarify: Some(false),
        used_evidence_ids_count: loop_state.executed_step_results.len(),
        ..Default::default()
    });
    log_deterministic_delivery_record(
        &task.task_id,
        "replace_git_repository_state_requested_machine_fields",
        "replaced",
        agent_run_context,
        loop_state.executed_step_results.len(),
    );
    true
}

fn route_requests_git_repository_state(route: &crate::IntentOutputContract) -> bool {
    route.semantic_kind_is(crate::OutputSemanticKind::GitRepositoryState)
        || crate::evidence_policy::final_answer_shape_for_output_contract(route)
            == Some(crate::evidence_policy::FinalAnswerShape::GitStateSummary)
}

fn requested_git_machine_fields(
    agent_run_context: Option<&AgentRunContext>,
) -> Vec<GitMachineField> {
    let mut fields = Vec::new();
    let Some(ctx) = agent_run_context else {
        return fields;
    };
    for surface in git_machine_request_surfaces(ctx) {
        if machine_surface_has_token(surface, "branch")
            || machine_surface_has_token(surface, "git.branch")
        {
            push_unique_git_machine_field(&mut fields, GitMachineField::Branch);
        }
        if machine_surface_has_token(surface, "worktree_state")
            || machine_surface_has_token(surface, "git.worktree")
        {
            push_unique_git_machine_field(&mut fields, GitMachineField::WorktreeState);
        }
        if machine_surface_has_token(surface, "changed_count")
            || machine_surface_has_token(surface, "git.changed.count")
        {
            push_unique_git_machine_field(&mut fields, GitMachineField::ChangedCount);
        }
    }
    fields
}

fn git_machine_request_surfaces(ctx: &AgentRunContext) -> Vec<&str> {
    let mut surfaces = Vec::new();
    for surface in [
        ctx.original_user_request.as_deref(),
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        surfaces.push(surface);
    }
    surfaces
}

fn push_unique_git_machine_field(fields: &mut Vec<GitMachineField>, field: GitMachineField) {
    if !fields.contains(&field) {
        fields.push(field);
    }
}

fn machine_surface_has_token(surface: &str, expected: &str) -> bool {
    surface
        .split(|ch: char| !(ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.')))
        .any(|token| token == expected)
}

fn latest_git_machine_state(loop_state: &LoopState) -> Option<GitMachineState> {
    loop_state
        .executed_step_results
        .iter()
        .rev()
        .filter(|step| {
            step.skill == "git_basic"
                && step.status == StepExecutionStatus::Ok
                && step
                    .output
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|output| !output.is_empty())
        })
        .filter_map(|step| {
            step.output
                .as_deref()
                .and_then(git_machine_state_from_output)
        })
        .next()
}

fn git_machine_state_from_output(output: &str) -> Option<GitMachineState> {
    let mut state = GitMachineState::default();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim()) {
        merge_git_machine_state_from_json(&value, &mut state, 0);
    }
    for candidate in git_status_text_candidates(output) {
        if let Some(observation) =
            crate::agent_engine::observed_output::git_repository_state_observation_from_status_output(
                &candidate,
                state.branch.as_deref(),
            )
        {
            if state.branch.is_none() {
                state.branch = observation.branch;
            }
            if state.worktree_state.is_none() {
                state.worktree_state = Some(if observation.dirty {
                    "dirty".to_string()
                } else {
                    "clean".to_string()
                });
            }
            if state.changed_count.is_none() {
                state.changed_count = Some(observation.changed_entries.len());
            }
        }
    }
    (state.branch.is_some() || state.worktree_state.is_some() || state.changed_count.is_some())
        .then_some(state)
}

fn merge_git_machine_state_from_json(
    value: &serde_json::Value,
    state: &mut GitMachineState,
    depth: usize,
) {
    if depth > 5 {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            if state.branch.is_none() {
                state.branch = object
                    .get("branch")
                    .or_else(|| object.get("current_branch"))
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|branch| !branch.is_empty())
                    .map(ToOwned::to_owned);
            }
            if state.worktree_state.is_none() {
                state.worktree_state = object
                    .get("worktree_state")
                    .and_then(serde_json::Value::as_str)
                    .map(str::trim)
                    .filter(|value| matches!(*value, "clean" | "dirty"))
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        object
                            .get("dirty")
                            .and_then(serde_json::Value::as_bool)
                            .map(|dirty| {
                                if dirty {
                                    "dirty".to_string()
                                } else {
                                    "clean".to_string()
                                }
                            })
                    })
                    .or_else(|| {
                        object
                            .get("clean")
                            .and_then(serde_json::Value::as_bool)
                            .map(|clean| {
                                if clean {
                                    "clean".to_string()
                                } else {
                                    "dirty".to_string()
                                }
                            })
                    });
            }
            if state.changed_count.is_none() {
                state.changed_count = object
                    .get("changed_count")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|count| usize::try_from(count).ok())
                    .or_else(|| {
                        object
                            .get("changed_files")
                            .and_then(serde_json::Value::as_array)
                            .map(Vec::len)
                    });
            }
            for child in object.values() {
                merge_git_machine_state_from_json(child, state, depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                merge_git_machine_state_from_json(child, state, depth + 1);
            }
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if let Ok(nested) = serde_json::from_str::<serde_json::Value>(trimmed) {
                merge_git_machine_state_from_json(&nested, state, depth + 1);
            }
            if let Some(observation) =
                crate::agent_engine::observed_output::git_repository_state_observation_from_status_output(
                    trimmed,
                    state.branch.as_deref(),
                )
            {
                if state.branch.is_none() {
                    state.branch = observation.branch;
                }
                if state.worktree_state.is_none() {
                    state.worktree_state = Some(if observation.dirty {
                        "dirty".to_string()
                    } else {
                        "clean".to_string()
                    });
                }
                if state.changed_count.is_none() {
                    state.changed_count = Some(observation.changed_entries.len());
                }
            }
        }
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => {}
    }
}

fn git_status_text_candidates(output: &str) -> Vec<String> {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut candidates = Vec::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        collect_git_status_text_candidates_from_json(&value, &mut candidates, 0);
    }
    candidates.push(trimmed.to_string());
    candidates.sort();
    candidates.dedup();
    candidates
}

fn collect_git_status_text_candidates_from_json(
    value: &serde_json::Value,
    candidates: &mut Vec<String>,
    depth: usize,
) {
    if depth > 5 {
        return;
    }
    match value {
        serde_json::Value::Object(object) => {
            for key in ["output", "command_output", "text"] {
                if let Some(child) = object.get(key) {
                    collect_git_status_text_candidates_from_json(child, candidates, depth + 1);
                }
            }
            if let Some(extra) = object.get("extra") {
                collect_git_status_text_candidates_from_json(extra, candidates, depth + 1);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_git_status_text_candidates_from_json(item, candidates, depth + 1);
            }
        }
        serde_json::Value::String(text) => {
            let trimmed = text.trim();
            if trimmed.is_empty() {
                return;
            }
            if let Ok(nested) = serde_json::from_str::<serde_json::Value>(trimmed) {
                collect_git_status_text_candidates_from_json(&nested, candidates, depth + 1);
            }
            candidates.push(trimmed.to_string());
        }
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => {}
    }
}

fn render_requested_git_machine_fields(
    requested_fields: &[GitMachineField],
    state: &GitMachineState,
) -> Option<String> {
    let mut parts = Vec::new();
    for field in requested_fields {
        match field {
            GitMachineField::Branch => {
                parts.push(format!("branch={}", state.branch.as_deref()?));
            }
            GitMachineField::WorktreeState => {
                parts.push(format!(
                    "worktree_state={}",
                    state.worktree_state.as_deref()?
                ));
            }
            GitMachineField::ChangedCount => {
                parts.push(format!("changed_count={}", state.changed_count?));
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}
