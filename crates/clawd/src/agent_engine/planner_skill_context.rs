use std::collections::{BTreeMap, BTreeSet};

use tracing::{debug, info, warn};

const CATALOG_PROMPT_SHARE_PERCENT: usize = 25;

use crate::{AppState, ClaimedTask};

use super::skill_quick_index::{
    output_contract_metadata as quick_index_output_contract_metadata,
    planner_capabilities_metadata as quick_index_planner_capabilities_metadata,
};

const SKILL_QUICK_INDEX_EMPTY_TOKEN: &str = "__RC_SKILL_QUICK_INDEX_EMPTY__";
const SKILL_SUMMARY_FALLBACK_TOKEN: &str = "__RC_SKILL_SUMMARY_FALLBACK__";
const SKILL_PROMPT_FILE_MISSING_TOKEN: &str = "__RC_SKILL_PROMPT_FILE_MISSING__";

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

struct ProviderFittedCatalog {
    text: String,
    mode: &'static str,
}

#[derive(Debug, Default)]
struct SkillPlaybookBundle {
    text: String,
    included_skills: Vec<String>,
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
    if let Some(allowed) = crate::capability_map::child_allowed_capabilities(state, task) {
        enabled.retain(|skill| {
            state.skill_manifest(skill).is_some_and(|manifest| {
                manifest
                    .planner_capabilities
                    .iter()
                    .any(|capability| allowed.contains(&capability.name))
            })
        });
    }
    if let Some(skill_scope) = skill_scope {
        enabled.retain(|skill| skill_scope.contains(skill));
    }
    enabled
}

#[cfg(test)]
fn build_skill_playbooks_bundle_scoped(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
) -> SkillPlaybookBundle {
    build_skill_playbooks_for_actions(state, task, skill_scope, None)
}

fn build_skill_playbooks_for_actions(
    state: &AppState,
    task: &ClaimedTask,
    skill_scope: Option<&BTreeSet<String>>,
    actions: Option<&BTreeMap<String, BTreeSet<String>>>,
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

        let selected_actions = actions
            .and_then(|map| map.get(skill))
            .cloned()
            .unwrap_or_default();
        let action_body =
            actions.and_then(|_| super::skill_action_docs::render(&prompt_body, &selected_actions));
        let narrowed = action_body.is_some();
        let trimmed = action_body.as_deref().unwrap_or(&prompt_body).trim();
        if trimmed.is_empty() {
            continue;
        }
        let metadata = state
            .skill_manifest(skill)
            .map(|mut manifest| {
                if narrowed {
                    manifest.planner_capabilities.retain(|capability| {
                        capability
                            .action
                            .as_ref()
                            .is_none_or(|action| selected_actions.contains(action))
                    });
                }
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
    value.split_whitespace().collect::<Vec<_>>().join(" ")
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
    let catalog_entries = super::capability_catalog::catalog_entries_for_task(state, task);
    let native_descriptions =
        crate::capability_map::planner_disclosed_native_capability_groups_for_task(
            state,
            task,
            &BTreeSet::new(),
        )
        .into_iter()
        .flat_map(|group| {
            group
                .capability_names
                .iter()
                .map(|name| (name.clone(), group.native_description_for(name)))
                .collect::<Vec<_>>()
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let catalog_digest = {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for entry in &catalog_entries {
            hash.update(entry.capability_ref.as_bytes());
            hash.update([0]);
            hash.update(entry.contract_sha256.as_bytes());
            hash.update([0]);
        }
        format!("{:x}", hash.finalize())
    };
    enabled.sort_by(|left, right| {
        let eager = |skill: &str| {
            registry
                .as_ref()
                .and_then(|registry| registry.get(skill))
                .is_some_and(|entry| entry.planner_eager_load)
        };
        eager(right).cmp(&eager(left)).then_with(|| left.cmp(right))
    });
    let mut catalog_lines = Vec::new();
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
        let capability_entries = catalog_entries
            .iter()
            .filter(|entry| entry.skill_id == *skill)
            .map(|entry| {
                super::capability_catalog::navigation_catalog_line(
                    entry,
                    native_descriptions
                        .get(&entry.capability_id)
                        .map(String::as_str),
                )
            })
            .collect::<Vec<_>>();
        if !capability_entries.is_empty() {
            catalog_lines.push(format!(
                "- catalog_skill={skill}; summary={summary}; capabilities={}",
                capability_entries.join("|")
            ));
        }
    }
    format!(
        "capability_catalog_v1 complete=true skill_count={} catalog_contract_sha256={catalog_digest} contract_details=load_capability_groups(op=expand)\n{}",
        catalog_lines.len(),
        catalog_lines.join("\n")
    )
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
        }
    }
    selected.into_iter().collect()
}

fn structured_action_scope(
    state: &AppState,
    loop_state: &super::LoopState,
) -> BTreeMap<String, BTreeSet<String>> {
    let mut actions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for round in &loop_state.round_traces {
        let Some(plan) = &round.plan_result else {
            continue;
        };
        for step in &plan.steps {
            let Some(action) = step.to_agent_action() else {
                continue;
            };
            let resolved =
                crate::capability_resolver::resolve_agent_action_for_state(state, action);
            let (skill, args) = match resolved {
                crate::AgentAction::CallSkill { skill, args } => (skill, args),
                crate::AgentAction::CallTool { tool, args } => (tool, args),
                _ => continue,
            };
            if let Some(action) = args
                .get("action")
                .and_then(serde_json::Value::as_str)
                .filter(|v| !v.is_empty())
            {
                actions
                    .entry(state.resolve_canonical_skill_name(&skill))
                    .or_default()
                    .insert(action.to_string());
            }
        }
    }
    actions
}

pub(super) fn build_planner_skill_context(
    state: &AppState,
    task: &ClaimedTask,
    loop_state: &super::LoopState,
) -> PlannerSkillContext {
    let canonical_quick_index = build_skill_quick_index_text_scoped(state, task, None);
    let catalog_entries = super::capability_catalog::catalog_entries_for_task(state, task);
    let fitted = fit_catalog_to_provider_window(state, &canonical_quick_index, &catalog_entries);
    let quick_index = fitted.text;
    let quick_index_chars = quick_index.chars().count();
    let scope = candidate_skill_scope_from_loop_state(state, task, loop_state);
    let action_scope = structured_action_scope(state, loop_state);
    let playbooks = if scope.is_empty() {
        SkillPlaybookBundle::default()
    } else {
        build_skill_playbooks_for_actions(state, task, Some(&scope), Some(&action_scope))
    };
    let disclosure_mode = if playbooks.included_skills.is_empty() {
        fitted.mode
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
    let selected_token = if selected_skills.is_empty() {
        "none".to_string()
    } else {
        selected_skills.join(",")
    };
    let mut text = format!(
        "runtime_skill_context_v3\ndisclosure_mode={disclosure_mode}\ncandidate_source={candidate_source}\nselected_skills={selected_token}\nregistry_disclosure=complete_catalog_and_selected_playbooks\nmcp_disclosure=complete_catalog_with_search\n\nCompact skill index:\n{quick_index}"
    );
    if !selected_skills.is_empty() {
        text.push_str("\n\nSelected skill playbooks:\n");
        text.push_str(&playbook_text);
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

fn fit_catalog_to_provider_window(
    state: &AppState,
    canonical: &str,
    entries: &[super::capability_catalog::CapabilityCatalogEntry],
) -> ProviderFittedCatalog {
    let descriptor = state
        .core
        .llm_providers
        .iter()
        .map(|provider| provider.model_descriptor())
        .filter_map(|descriptor| {
            descriptor
                .context_window_tokens
                .map(|window| (window, descriptor.output_reserve_tokens))
        })
        .min_by_key(|(window, _)| *window);
    let Some((context_window, output_reserve)) = descriptor else {
        return ProviderFittedCatalog {
            text: canonical.to_string(),
            mode: "compact_index",
        };
    };
    let prompt_capacity = context_window.saturating_sub(output_reserve);
    let catalog_budget = prompt_capacity
        .saturating_mul(CATALOG_PROMPT_SHARE_PERCENT)
        .saturating_div(100)
        .clamp(1, 2048);
    let estimate = crate::token_estimator::estimate_generic_tokens(canonical).provider_tokens;
    if estimate <= catalog_budget {
        return ProviderFittedCatalog {
            text: canonical.to_string(),
            mode: "compact_index",
        };
    }
    let canonical_sha256 = {
        use sha2::{Digest, Sha256};
        format!("{:x}", Sha256::digest(canonical.as_bytes()))
    };
    let skills = entries
        .iter()
        .map(|entry| entry.skill_id.clone())
        .collect::<BTreeSet<_>>();
    ProviderFittedCatalog {
        text: format!(
            "capability_catalog_view_v1 complete=false canonical_complete=true canonical_ref=catalog:{canonical_sha256} canonical_capability_count={} skill_count={} provider_context_window_tokens={context_window} provider_catalog_budget_tokens={catalog_budget} recovery=load_capability_groups(op=search|expand)\n- catalog_skills={}",
            entries.len(),
            skills.len(),
            skills.into_iter().collect::<Vec<_>>().join("|")
        ),
        mode: "provider_fitted_catalog",
    }
}

#[cfg(test)]
#[path = "planner_skill_context_tests.rs"]
mod tests;
