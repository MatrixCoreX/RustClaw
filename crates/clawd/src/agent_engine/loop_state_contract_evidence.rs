use serde_json::Value;

use super::AgentRunContext;

pub(super) fn active_plan_file_targets_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut targets = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        targets.extend(active_plan_file_targets_from_boundary_observation_blocks(
            summary,
        ));
    }
    targets.sort();
    targets.dedup();
    targets
}

fn active_plan_file_targets_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(files) = value.get("active_plan_files").and_then(Value::as_array) else {
            continue;
        };
        out.extend(
            files
                .iter()
                .filter_map(|file| file.get("workspace_path"))
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }
    out
}

pub(super) fn default_main_config_contract_evidence_for_loop_seed(
    ctx: &AgentRunContext,
) -> Vec<Value> {
    let mut evidence = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        evidence.extend(
            default_main_config_contract_evidence_from_boundary_observation_blocks(summary),
        );
    }
    evidence.sort_by_key(|value| serde_json::to_string(value).unwrap_or_default());
    evidence.dedup_by(|left, right| left == right);
    evidence
}

fn default_main_config_contract_evidence_from_boundary_observation_blocks(
    summary: &str,
) -> Vec<Value> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(contract) = value
            .get("default_main_config_contract")
            .and_then(Value::as_object)
        else {
            continue;
        };
        let contract_name = contract
            .get("contract")
            .and_then(Value::as_str)
            .map(str::trim);
        let logical_path = contract
            .get("logical_path")
            .and_then(Value::as_str)
            .map(str::trim);
        if contract_name == Some("rustclaw_main_config")
            && logical_path.is_some_and(|path| !path.is_empty())
        {
            out.push(Value::Object(contract.clone()));
        }
    }
    out
}

pub(super) fn first_string_field(values: &[Value], field: &str) -> Option<String> {
    values.iter().find_map(|value| {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    })
}

pub(super) fn registry_capability_contract_evidence_for_loop_seed(
    ctx: &AgentRunContext,
) -> Vec<Value> {
    let mut evidence = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        evidence.extend(
            registry_capability_contract_evidence_from_boundary_observation_blocks(summary),
        );
    }
    evidence.sort_by_key(|value| serde_json::to_string(value).unwrap_or_default());
    evidence.dedup_by(|left, right| left == right);
    evidence
}

fn registry_capability_contract_evidence_from_boundary_observation_blocks(
    summary: &str,
) -> Vec<Value> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(contract) = value
            .get("registry_capability_contract")
            .and_then(Value::as_object)
        else {
            continue;
        };
        let has_refs = contract
            .get("capability_refs")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .any(|value| !value.is_empty())
            });
        if has_refs {
            out.push(Value::Object(contract.clone()));
        }
    }
    out
}

pub(super) fn registry_capability_contract_refs(evidence: &[Value]) -> Vec<String> {
    let mut refs = evidence
        .iter()
        .flat_map(|value| {
            value
                .get("capability_refs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string)
        })
        .collect::<Vec<_>>();
    refs.sort();
    refs.dedup();
    refs
}

pub(super) fn contract_repair_candidate_evidence_for_loop_seed(
    ctx: &AgentRunContext,
) -> Vec<Value> {
    let mut evidence = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        evidence.extend(contract_repair_candidates_from_boundary_observation_blocks(
            summary,
        ));
    }
    evidence.sort_by_key(|value| serde_json::to_string(value).unwrap_or_default());
    evidence.dedup_by(|left, right| left == right);
    evidence
}

fn contract_repair_candidates_from_boundary_observation_blocks(summary: &str) -> Vec<Value> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(candidates) = value
            .get("contract_repair_candidates")
            .and_then(Value::as_array)
        else {
            continue;
        };
        for candidate in candidates {
            let has_contract = candidate
                .get("contract_ref")
                .and_then(Value::as_str)
                .map(str::trim)
                .is_some_and(|value| !value.is_empty());
            if has_contract {
                out.push(candidate.clone());
            }
        }
    }
    out
}

pub(super) fn pre_loop_clarify_candidates_for_loop_seed(ctx: &AgentRunContext) -> Vec<String> {
    let mut candidates = Vec::new();
    for summary in [
        ctx.user_request.as_deref(),
        ctx.context_bundle_summary.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        candidates.extend(pre_loop_clarify_candidates_from_boundary_observation_blocks(summary));
    }
    candidates.sort();
    candidates.dedup();
    candidates
}

fn pre_loop_clarify_candidates_from_boundary_observation_blocks(summary: &str) -> Vec<String> {
    const START: &str = "### AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    const END: &str = "### END_AGENT_LOOP_BOUNDARY_OBSERVATIONS";
    let mut out = Vec::new();
    for tail in summary.split(START).skip(1) {
        let block = tail.split(END).next().unwrap_or(tail).trim();
        let Ok(value) = serde_json::from_str::<Value>(block) else {
            continue;
        };
        let Some(candidates) = value
            .get("pre_loop_clarify_candidates")
            .and_then(Value::as_array)
        else {
            continue;
        };
        out.extend(
            candidates
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }
    out
}
