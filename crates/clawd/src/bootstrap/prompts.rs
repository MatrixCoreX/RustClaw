use std::path::Path;

use claw_core::{config::PersonaConfig, prompt_layers};
use tracing::warn;

use crate::{llm_vendor_name, AppState};

fn builtin_persona_prompt(profile: &str) -> &'static str {
    match profile {
        "expert" => {
            "Persona profile: expert. Be rigorous and concise. Explain key trade-offs, assumptions, and verification steps. Prefer correctness and safety over speed."
        }
        "teacher" => {
            "Persona profile: teacher. Be patient, beginner-friendly, and clear. Explain in simple steps, define necessary terms briefly, and help the user build understanding without unnecessary jargon."
        }
        "advisor" => {
            "Persona profile: advisor. Be calm, balanced, and recommendation-oriented. Help the user choose a sensible default, explain the main trade-offs briefly, and optimize for practical decisions."
        }
        "reviewer" => {
            "Persona profile: reviewer. Be critical, precise, and risk-aware. Surface the most important issues first, distinguish severity clearly, and avoid softening concrete problems."
        }
        "companion" => {
            "Persona profile: companion. Be friendly and supportive while staying practical. Keep responses clear and encouraging, but still action-oriented."
        }
        _ => {
            "Persona profile: executor. Be direct and efficient. Give conclusion first, then minimal actionable details. Prioritize execution quality and safety."
        }
    }
}

pub(crate) fn load_persona_prompt(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    cfg: &PersonaConfig,
) -> String {
    let raw_profile = cfg.profile.trim().to_ascii_lowercase();
    let profile = match raw_profile.as_str() {
        "expert" | "companion" | "executor" | "teacher" | "advisor" | "reviewer" => raw_profile,
        other => {
            warn!("unknown persona profile={}, fallback to executor", other);
            "executor".to_string()
        }
    };
    let dir = if cfg.dir.trim().is_empty() {
        "prompts/personas".to_string()
    } else {
        cfg.dir.trim().to_string()
    };
    let rel_path = format!("{dir}/{profile}.md");
    let (template, resolved_path) = load_prompt_template_for_vendor(
        workspace_root,
        selected_vendor,
        &rel_path,
        builtin_persona_prompt(&profile),
    );
    let text = template.trim();
    if text.is_empty() {
        warn!(
            "persona prompt source resolved empty, fallback to built-in: path={}",
            resolved_path
        );
        builtin_persona_prompt(&profile).to_string()
    } else {
        text.to_string()
    }
}

pub(crate) fn prompt_vendor_name_from_selected_vendor(selected_vendor: Option<&str>) -> String {
    selected_vendor
        .map(prompt_layers::normalize_prompt_vendor_name)
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) fn active_prompt_vendor_name(state: &AppState) -> String {
    if let Some(provider) = state.core.llm_providers.first() {
        return prompt_layers::normalize_prompt_vendor_name(llm_vendor_name(provider));
    }
    if let Some(active) = state.core.active_provider_type.as_deref() {
        return prompt_layers::normalize_prompt_vendor_name(active);
    }
    "default".to_string()
}

pub(crate) fn resolve_prompt_rel_path_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> String {
    prompt_layers::resolve_prompt_rel_path_for_vendor(workspace_root, vendor, rel_path)
}

pub(crate) fn load_prompt_template_for_vendor(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = prompt_vendor_name_from_selected_vendor(selected_vendor);
    prompt_layers::load_prompt_template_for_vendor(
        workspace_root,
        &vendor,
        rel_path,
        default_template,
    )
}

pub(crate) fn load_prompt_template_for_state(
    state: &AppState,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = active_prompt_vendor_name(state);
    prompt_layers::load_prompt_template_for_vendor(
        &state.skill_rt.workspace_root,
        &vendor,
        rel_path,
        default_template,
    )
}
