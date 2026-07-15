use std::collections::BTreeSet;

use tracing::{debug, info, warn};

use crate::{AppState, ClaimedTask};

use super::skill_quick_index::{
    output_contract as quick_index_output_contract,
    output_contract_metadata as quick_index_output_contract_metadata,
    planner_capabilities as quick_index_planner_capabilities,
    planner_capabilities_metadata as quick_index_planner_capabilities_metadata,
};

const SKILL_PLAYBOOKS_EMPTY_TOKEN: &str = "__RC_SKILL_PLAYBOOKS_EMPTY__";
const SKILL_QUICK_INDEX_EMPTY_TOKEN: &str = "__RC_SKILL_QUICK_INDEX_EMPTY__";
const SKILL_SUMMARY_FALLBACK_TOKEN: &str = "__RC_SKILL_SUMMARY_FALLBACK__";
const SKILL_PROMPT_FILE_MISSING_TOKEN: &str = "__RC_SKILL_PROMPT_FILE_MISSING__";

/// Phase 2+: Planner-visible skills are dynamically narrowed by
/// execution-enabled skills intersected with the agent's allowed skill scope.
/// Each visible skill should provide a registry prompt logical path before its
/// playbook is injected into the planner prompt.
fn planner_available_skills_for_task_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> Vec<String> {
    let mut enabled = state.planner_available_skills_for_task(task);
    if let Some(skill_scope) = skill_scope {
        enabled.retain(|skill| skill_scope.contains(skill));
    }
    enabled
}

pub(super) fn build_skill_playbooks_text_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> String {
    let enabled = planner_available_skills_for_task_scoped(state, task, skill_scope);
    let enabled_count = enabled.len();
    let agent_id = state.task_agent_id(task);
    info!(
        "planner skill playbooks: agent_id={} planner_visible_skills_count={} scoped={} skills=[{}]",
        agent_id,
        enabled_count,
        skill_scope.is_some(),
        enabled.join(", ")
    );

    let mut sections = Vec::new();
    let mut skipped_no_prompt: Vec<String> = Vec::new();

    for skill in &enabled {
        let Some(registry_prompt_rel_path) = state.skill_registry_prompt_rel_path(skill) else {
            warn!(
                "planner skill playbook: skill={} registry prompt_file missing, skipping",
                skill
            );
            skipped_no_prompt.push(skill.clone());
            continue;
        };

        let prompt_body =
            crate::load_prompt_template_for_state(state, &registry_prompt_rel_path, "").0;

        debug!(
            "planner skill playbook: skill={} prompt_logical_path={} source=registry",
            skill, registry_prompt_rel_path
        );

        let trimmed = prompt_body.trim();
        if trimmed.is_empty() {
            continue;
        }
        let metadata = state
            .skill_manifest(skill)
            .map(|manifest| {
                let mut parts = vec![format!(
                    "planner_kind: {}",
                    manifest.planner_kind.as_token()
                )];
                parts.extend(crate::skill_availability::availability_metadata_parts(
                    &crate::skill_availability::evaluate_manifest_availability(&manifest),
                ));
                if let Some(capabilities) = quick_index_planner_capabilities_metadata(&manifest) {
                    parts.push(capabilities);
                }
                parts.push(quick_index_output_contract_metadata(&manifest));
                format!("Registry metadata: {}", parts.join("; "))
            })
            .unwrap_or_default();
        if metadata.is_empty() {
            sections.push(format!("### {skill}\n{trimmed}"));
        } else {
            sections.push(format!("### {skill}\n{trimmed}\n{metadata}"));
        }
    }

    if !skipped_no_prompt.is_empty() {
        warn!(
            "planner skill playbooks: skipped_no_prompt_count={} skills=[{}]",
            skipped_no_prompt.len(),
            skipped_no_prompt.join(", ")
        );
    }

    let included_count = sections.len();
    info!(
        "planner skill playbooks: included_count={} (enabled={} skipped={})",
        included_count,
        enabled_count,
        enabled_count.saturating_sub(included_count)
    );

    if sections.is_empty() {
        SKILL_PLAYBOOKS_EMPTY_TOKEN.to_string()
    } else {
        sections.join("\n\n")
    }
}

fn first_non_heading_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("```")
                && !line.starts_with("<!--")
        })
        .map(|line| {
            let mut out = line.to_string();
            if out.chars().count() > 90 {
                out = out.chars().take(90).collect::<String>() + "...";
            }
            out
        })
}

/// First-round route hint: give the LLM a compact skill index so ordinary
/// capability decisions stay inside the planner instead of a pre-route branch.
pub(super) fn build_skill_quick_index_text_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> String {
    let enabled = planner_available_skills_for_task_scoped(state, task, skill_scope);
    if enabled.is_empty() {
        let mut line = String::from("- ");
        line.push_str(SKILL_QUICK_INDEX_EMPTY_TOKEN);
        return line;
    }
    let mut lines = Vec::new();
    for skill in &enabled {
        let summary =
            if let Some(registry_prompt_rel_path) = state.skill_registry_prompt_rel_path(skill) {
                let prompt_body =
                    crate::load_prompt_template_for_state(state, &registry_prompt_rel_path, "").0;
                first_non_heading_line(&prompt_body)
                    .unwrap_or_else(|| SKILL_SUMMARY_FALLBACK_TOKEN.to_string())
            } else {
                SKILL_PROMPT_FILE_MISSING_TOKEN.to_string()
            };
        if let Some(manifest) = state.skill_manifest(skill) {
            lines.push(format!(
                "- skill={skill}; summary={summary}; planner_kind={}{}{}",
                manifest.planner_kind.as_token(),
                quick_index_planner_capabilities(&manifest),
                quick_index_output_contract(&manifest)
            ));
        } else {
            lines.push(format!("- {skill}: {summary}"));
        }
    }
    lines.join("\n")
}
