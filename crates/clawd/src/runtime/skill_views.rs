use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use claw_core::config::AppConfig;
use claw_core::skill_registry::SkillsRegistry;
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::active_runtime_config_path;
use super::state::{
    AppState, PromptTemplateCache, SkillViews, SkillViewsBinding, SkillViewsSnapshot,
};
use crate::skill_admission::{OverlaySnapshot, SkillAdmissionService};

#[cfg(test)]
pub(crate) fn build_skill_views(
    workspace_root: &Path,
    registry_path: Option<&str>,
    skill_switches: &HashMap<String, bool>,
    uninstalled_skills: &[String],
) -> Result<SkillViews, String> {
    build_skill_views_with_overlay(
        workspace_root,
        registry_path,
        skill_switches,
        uninstalled_skills,
        None,
    )
}

pub(crate) fn build_skill_views_with_overlay(
    workspace_root: &Path,
    registry_path: Option<&str>,
    skill_switches: &HashMap<String, bool>,
    uninstalled_skills: &[String],
    overlay: Option<&OverlaySnapshot>,
) -> Result<SkillViews, String> {
    let Some(registry_path) = registry_path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Err(
            "skill_registry_not_configured; repair=set_skills.registry_path_to_a_valid_base_registry"
                .to_string(),
        );
    };
    let path = if Path::new(registry_path).is_absolute() {
        PathBuf::from(registry_path)
    } else {
        workspace_root.join(registry_path)
    };
    let registry = SkillsRegistry::load_from_base_and_overlay(
        &path,
        overlay.and_then(|snapshot| snapshot.registry_dir.as_deref()),
    )
    .map(Arc::new)
    .map_err(|error| format!("registry load failed: {}: {error}", path.display()))?;
    if registry.all_names().is_empty() {
        return Err(format!(
            "skill_registry_empty; path={}; repair=restore_a_non_empty_base_registry",
            path.display()
        ));
    }
    let fixed_on_skills = registry
        .fixed_on_names()
        .into_iter()
        .collect::<HashSet<_>>();

    let explicitly_disabled: HashSet<String> = skill_switches
        .iter()
        .filter(|(_, &on)| !on)
        .map(|(skill, _)| {
            registry
                .resolve_canonical(skill)
                .map(String::from)
                .unwrap_or_else(|| crate::canonical_skill_name(skill).to_string())
        })
        .collect();

    let mut enabled: HashSet<String> = registry.enabled_names().into_iter().collect();
    for (skill, is_enabled) in skill_switches {
        let canonical = registry
            .resolve_canonical(skill)
            .map(String::from)
            .unwrap_or_else(|| crate::canonical_skill_name(skill).to_string());
        if *is_enabled {
            enabled.insert(canonical);
        } else {
            enabled.remove(&canonical);
        }
    }
    for skill in &fixed_on_skills {
        if !explicitly_disabled.contains(skill) {
            enabled.insert(skill.clone());
        }
    }
    for skill in uninstalled_skills {
        let canonical = registry
            .resolve_canonical(skill)
            .map(String::from)
            .unwrap_or_else(|| crate::canonical_skill_name(skill).to_string());
        if !fixed_on_skills.contains(&canonical) {
            enabled.remove(&canonical);
        }
    }
    if let Some(overlay) = overlay {
        enabled.extend(overlay.enabled.iter().cloned());
        for skill in overlay
            .disabled
            .iter()
            .chain(overlay.awaiting_policy.iter())
            .chain(overlay.tombstoned.iter())
        {
            enabled.remove(skill);
        }
    }
    let mut planner_visible: Vec<String> = enabled
        .iter()
        .filter(|skill| registry.is_planner_visible(skill))
        .cloned()
        .collect();
    planner_visible.sort_unstable();

    Ok(SkillViews {
        registry: Some(registry),
        execution_skills: enabled,
        planner_visible,
    })
}

pub(crate) fn reload_skill_views(state: &AppState) -> Result<ReloadSkillViewsResult, String> {
    prepare_and_publish_views(state, || {})
}

// A private test seam, not a runtime/plugin hook; release callers pass a no-op.
fn prepare_and_publish_views(
    state: &AppState,
    before_publish: impl FnOnce(),
) -> Result<ReloadSkillViewsResult, String> {
    let config_path = active_runtime_config_path(state);
    tracing::info!(
        "reload_skill_views: started config_path={}",
        config_path.display()
    );
    let config_path = config_path
        .to_str()
        .ok_or_else(|| "runtime_config_path_not_utf8".to_string())?;
    let config = AppConfig::load(config_path)
        .map_err(|e| format!("reload_skill_views: load config failed: {}", e))?;
    let publish = |overlay| publish_verified_views(state, &config, overlay, before_publish);
    if config.skills.registry_path.is_none() {
        return publish(OverlaySnapshot::default());
    }
    SkillAdmissionService::from_config(&state.skill_rt.workspace_root, &config)
        .and_then(|service| service.with_verified_snapshot(publish))
        .map_err(|error| error.to_string())?
}

fn publish_verified_views(
    state: &AppState,
    config: &AppConfig,
    overlay: OverlaySnapshot,
    before_publish: impl FnOnce(),
) -> Result<ReloadSkillViewsResult, String> {
    let registry_path = config.skills.registry_path.as_deref();
    let path_display = registry_path.unwrap_or("(none)");
    let views = build_skill_views_with_overlay(
        &state.skill_rt.workspace_root,
        registry_path,
        &config.skills.skill_switches,
        &config.skills.uninstalled_skills,
        Some(&overlay),
    )?;
    let registry_entries = views
        .registry
        .as_ref()
        .map(|r| r.all_names().len())
        .unwrap_or(0);
    let execution_count = views.execution_skills.len();
    let planner_count = views.planner_visible.len();

    let snapshot = assemble_skill_views_snapshot(views, &overlay);
    before_publish();
    *state.core.skill_views_snapshot.write().unwrap() = Arc::new(snapshot);
    state.skill_rt.runner_pool.invalidate_all();

    tracing::info!(
        "reload_skill_views: success path={} registry_entries={} execution_skills_count={} planner_visible_count={}",
        path_display,
        registry_entries,
        execution_count,
        planner_count
    );
    Ok(ReloadSkillViewsResult {
        registry_entries,
        execution_skills_count: execution_count,
        planner_visible_count: planner_count,
    })
}

pub(crate) fn assemble_skill_views_snapshot(
    views: SkillViews,
    overlay: &OverlaySnapshot,
) -> SkillViewsSnapshot {
    let registry_generation_digest = overlay.base_registry_digest.as_ref().map(|base_digest| {
        let mut digest = Sha256::new();
        digest.update(b"agent-registry-generation-v1\0");
        digest.update(base_digest.as_bytes());
        digest.update(b"\0");
        digest.update(overlay.generation.to_le_bytes());
        digest.update(b"\0");
        if let Some(overlay_digest) = overlay.generation_digest.as_deref() {
            digest.update(overlay_digest.as_bytes());
        }
        format!("{:x}", digest.finalize())
    });
    SkillViewsSnapshot {
        registry: views.registry,
        skills_list: Arc::new(views.execution_skills),
        binding: SkillViewsBinding {
            registry_generation: overlay.generation,
            registry_generation_digest,
            base_registry_digest: overlay.base_registry_digest.clone(),
            overlay_generation_digest: overlay.generation_digest.clone(),
            admission_bindings: Arc::new(overlay.execution_bindings.clone()),
            prompt_templates: PromptTemplateCache::default(),
        },
    }
}

pub(crate) fn load_skill_admission_snapshot(
    workspace_root: &Path,
    config: &AppConfig,
) -> Result<OverlaySnapshot, String> {
    if config.skills.registry_path.is_none() {
        return Ok(OverlaySnapshot::default());
    }
    SkillAdmissionService::from_config(workspace_root, config)
        .and_then(|service| service.snapshot())
        .map_err(|error| error.to_string())
}

#[derive(Debug, Serialize)]
pub(crate) struct ReloadSkillViewsResult {
    pub(crate) registry_entries: usize,
    pub(crate) execution_skills_count: usize,
    pub(crate) planner_visible_count: usize,
}

#[cfg(test)]
#[path = "skill_reload_tests.rs"]
mod skill_reload_tests;
