use std::collections::BTreeSet;

use tracing::{debug, info, warn};

use crate::{AppState, ClaimedTask};

use super::skill_quick_index::{
    output_contract as quick_index_output_contract,
    output_contract_metadata as quick_index_output_contract_metadata,
    planner_capabilities as quick_index_planner_capabilities,
    planner_capabilities_metadata as quick_index_planner_capabilities_metadata,
    planner_capability_candidates as quick_index_planner_capability_candidates,
};

const SKILL_QUICK_INDEX_EMPTY_TOKEN: &str = "__RC_SKILL_QUICK_INDEX_EMPTY__";
const SKILL_SUMMARY_FALLBACK_TOKEN: &str = "__RC_SKILL_SUMMARY_FALLBACK__";
const SKILL_PROMPT_FILE_MISSING_TOKEN: &str = "__RC_SKILL_PROMPT_FILE_MISSING__";
const SKILL_QUICK_INDEX_CHAR_BUDGET: usize = 32_000;
const SKILL_QUICK_INDEX_LINE_CHAR_BUDGET: usize = 720;
const SKILL_PLAYBOOK_CHAR_BUDGET: usize = 72_000;
const SKILL_PLAYBOOK_SINGLE_CHAR_LIMIT: usize = 40_000;
const MAX_SCOPED_SKILL_PLAYBOOKS: usize = 2;

#[derive(Debug, Clone)]
pub(super) struct PlannerSkillContext {
    pub(super) text: String,
    pub(super) quick_index_text: String,
    pub(super) playbook_text: String,
    pub(super) disclosure_mode: &'static str,
    pub(super) selected_skills: Vec<String>,
    pub(super) quick_index_chars: usize,
    pub(super) playbook_chars: usize,
}

#[derive(Debug, Default)]
struct SkillPlaybookBundle {
    text: String,
    included_skills: Vec<String>,
    omitted_count: usize,
}

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

fn build_skill_playbooks_bundle_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> SkillPlaybookBundle {
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
    let mut included_skills = Vec::new();
    let mut skipped_no_prompt: Vec<String> = Vec::new();
    let mut used_chars = 0usize;
    let mut omitted_count = 0usize;

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
        let section = if metadata.is_empty() {
            format!("### {skill}\n{trimmed}")
        } else {
            format!("### {skill}\n{trimmed}\n{metadata}")
        };
        let section_chars = section.chars().count();
        let separator_chars = usize::from(!sections.is_empty()) * 2;
        if section_chars > SKILL_PLAYBOOK_SINGLE_CHAR_LIMIT
            || used_chars + separator_chars + section_chars > SKILL_PLAYBOOK_CHAR_BUDGET
        {
            omitted_count += 1;
            warn!(
                "planner skill playbook omitted by budget: skill={} section_chars={} used_chars={} total_budget={}",
                skill, section_chars, used_chars, SKILL_PLAYBOOK_CHAR_BUDGET
            );
            continue;
        }
        used_chars += separator_chars + section_chars;
        included_skills.push(skill.clone());
        sections.push(section);
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

    let text = sections.join("\n\n");
    SkillPlaybookBundle {
        text,
        included_skills,
        omitted_count,
    }
}

fn first_non_heading_line(text: &str) -> Option<String> {
    let lines = text.lines().collect::<Vec<_>>();
    let capability_summary = ["## Capability Summary", "## Capability"]
        .into_iter()
        .find_map(|heading| {
            lines
                .iter()
                .position(|line| line.trim().starts_with(heading))
                .and_then(|index| first_summary_line(lines.iter().skip(index + 1).copied()))
        });
    capability_summary.or_else(|| first_summary_line(lines.into_iter()))
}

fn first_summary_line<'a>(lines: impl Iterator<Item = &'a str>) -> Option<String> {
    lines
        .map(str::trim)
        .find(|line| {
            !line.is_empty()
                && !line.starts_with('#')
                && !line.starts_with("```")
                && !line.starts_with("<!--")
                && !line.starts_with("Registry metadata:")
        })
        .map(compact_summary)
}

fn compact_summary(value: &str) -> String {
    if value.chars().count() > 90 {
        value.chars().take(90).collect::<String>() + "..."
    } else {
        value.to_string()
    }
}

/// First-round route hint: give the LLM a compact skill index so ordinary
/// capability decisions stay inside the planner instead of a pre-route branch.
pub(super) fn build_skill_quick_index_text_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> String {
    let mut enabled = planner_available_skills_for_task_scoped(state, task, skill_scope);
    if enabled.is_empty() {
        let mut line = String::from("- ");
        line.push_str(SKILL_QUICK_INDEX_EMPTY_TOKEN);
        return line;
    }
    let registry = state.get_skills_registry();
    enabled.sort_by(|left, right| {
        let eager = |skill: &str| {
            registry
                .as_ref()
                .and_then(|registry| registry.get(skill))
                .is_some_and(|entry| entry.planner_eager_load)
        };
        eager(right).cmp(&eager(left)).then_with(|| left.cmp(right))
    });
    let mut candidate_lines = Vec::new();
    for skill in &enabled {
        let Some(manifest) = state.skill_manifest(skill) else {
            warn!(
                "planner skill quick index omitted skill without registry manifest: skill={}",
                skill
            );
            continue;
        };
        let summary = manifest
            .description
            .as_deref()
            .map(compact_summary)
            .unwrap_or_else(|| {
                if let Some(registry_prompt_rel_path) = state.skill_registry_prompt_rel_path(skill)
                {
                    let prompt_body =
                        crate::load_prompt_template_for_state(state, &registry_prompt_rel_path, "")
                            .0;
                    first_non_heading_line(&prompt_body)
                        .unwrap_or_else(|| SKILL_SUMMARY_FALLBACK_TOKEN.to_string())
                } else {
                    SKILL_PROMPT_FILE_MISSING_TOKEN.to_string()
                }
            });
        let detailed_capabilities = quick_index_planner_capabilities(&manifest)
            .strip_prefix("; planner_capabilities: ")
            .unwrap_or_default()
            .to_string();
        let compact_capabilities = quick_index_planner_capability_candidates(&manifest)
            .strip_prefix("; capability_candidates=")
            .unwrap_or_default()
            .to_string();
        if detailed_capabilities.is_empty() {
            warn!(
                "planner skill quick index omitted skill without callable capability: skill={}",
                skill
            );
            continue;
        }
        let detailed = format!(
            "- callable_capabilities={detailed_capabilities}; summary={summary}; planner_layer={}{}",
            manifest.planner_kind.as_token(),
            quick_index_output_contract(&manifest)
        );
        let compact = format!(
            "- callable_capabilities={compact_capabilities}; summary={summary}; planner_layer={}{}",
            manifest.planner_kind.as_token(),
            quick_index_output_contract(&manifest)
        );
        candidate_lines.push(
            if detailed.chars().count() <= SKILL_QUICK_INDEX_LINE_CHAR_BUDGET {
                detailed
            } else {
                compact
            },
        );
    }
    let mut lines = Vec::new();
    let mut used_chars = 0usize;
    let mut omitted_count = 0usize;
    for line in candidate_lines {
        let separator_chars = usize::from(!lines.is_empty());
        let line_chars = line.chars().count();
        if used_chars + separator_chars + line_chars > SKILL_QUICK_INDEX_CHAR_BUDGET {
            omitted_count += 1;
            continue;
        }
        used_chars += separator_chars + line_chars;
        lines.push(line);
    }
    if omitted_count > 0 {
        let marker = format!("- omitted_skill_details={omitted_count}; reason=prompt_budget");
        let marker_chars = marker.chars().count() + usize::from(!lines.is_empty());
        if used_chars + marker_chars <= SKILL_QUICK_INDEX_CHAR_BUDGET {
            lines.push(marker);
        }
    }
    lines.join("\n")
}

fn candidate_skill_scope_from_loop_state(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &super::LoopState,
) -> BTreeSet<String> {
    if loop_state.round_no <= 1 {
        return BTreeSet::new();
    }
    let available = state
        .planner_available_skills_for_task(task)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let mut selected = loop_state
        .loaded_capability_skills
        .iter()
        .filter(|skill| available.contains(*skill))
        .cloned()
        .collect::<Vec<_>>();
    selected.truncate(MAX_SCOPED_SKILL_PLAYBOOKS);
    if selected.len() >= MAX_SCOPED_SKILL_PLAYBOOKS {
        return selected.into_iter().collect();
    }
    for round in loop_state.round_traces.iter().rev() {
        let Some(plan) = round.plan_result.as_ref() else {
            continue;
        };
        for step in &plan.steps {
            let Some(action) = step.to_agent_action() else {
                continue;
            };
            let resolved =
                crate::capability_resolver::resolve_agent_action_for_state(state, action);
            let candidate = match resolved {
                crate::AgentAction::CallSkill { skill, .. } => Some(skill),
                crate::AgentAction::CallTool { tool, .. } => Some(tool),
                _ => None,
            };
            let Some(candidate) = candidate else {
                continue;
            };
            let canonical = state.resolve_canonical_skill_name(&candidate);
            if available.contains(&canonical) && !selected.contains(&canonical) {
                selected.push(canonical);
            }
            if selected.len() >= MAX_SCOPED_SKILL_PLAYBOOKS {
                return selected.into_iter().collect();
            }
        }
    }
    selected.into_iter().collect()
}

pub(super) fn build_planner_skill_context(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &super::LoopState,
) -> PlannerSkillContext {
    let quick_index = build_skill_quick_index_text_scoped(state, task, None);
    let quick_index_chars = quick_index.chars().count();
    let scope = candidate_skill_scope_from_loop_state(state, task, loop_state);
    let playbooks = if scope.is_empty() {
        SkillPlaybookBundle::default()
    } else {
        build_skill_playbooks_bundle_scoped(state, task, Some(&scope))
    };
    let disclosure_mode = if playbooks.included_skills.is_empty() {
        "compact_index"
    } else {
        "scoped_playbooks"
    };
    let candidate_source = if scope.is_empty() {
        "registry_machine_metadata"
    } else {
        "structured_prior_plan"
    };
    let playbook_chars = playbooks.text.chars().count();
    let selected_skills = playbooks.included_skills;
    let playbook_text = playbooks.text;
    let omitted_playbook_count = playbooks.omitted_count;
    let selected_token = if selected_skills.is_empty() {
        "none".to_string()
    } else {
        selected_skills.join(",")
    };
    let mut text = format!(
        "runtime_skill_context_v2\ndisclosure_mode={disclosure_mode}\ncandidate_source={candidate_source}\nselected_skills={selected_token}\nquick_index_budget_chars={SKILL_QUICK_INDEX_CHAR_BUDGET}\nplaybook_budget_chars={SKILL_PLAYBOOK_CHAR_BUDGET}\nmcp_disclosure=bounded_catalog_with_search\n\nCompact skill index:\n{quick_index}"
    );
    if !selected_skills.is_empty() {
        text.push_str("\n\nSelected skill playbooks:\n");
        text.push_str(&playbook_text);
    }
    if omitted_playbook_count > 0 {
        text.push_str(&format!(
            "\n\nomitted_selected_playbooks={}; reason=prompt_budget",
            omitted_playbook_count
        ));
    }
    info!(
        "planner skill context: mode={} selected_skills=[{}] quick_index_chars={} playbook_chars={} total_chars={}",
        disclosure_mode,
        selected_token,
        quick_index_chars,
        playbook_chars,
        text.chars().count()
    );
    PlannerSkillContext {
        text,
        quick_index_text: quick_index,
        playbook_text,
        disclosure_mode,
        selected_skills,
        quick_index_chars,
        playbook_chars,
    }
}

#[cfg(test)]
#[path = "planner_skill_context_tests.rs"]
mod tests;
