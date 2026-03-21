use std::path::Path;

use claw_core::config::PersonaConfig;
use tracing::warn;

use crate::{llm_vendor_name, AppState};

fn builtin_persona_prompt(profile: &str) -> &'static str {
    match profile {
        "expert" => {
            "Persona profile: expert. Be rigorous and concise. Explain key trade-offs, assumptions, and verification steps. Prefer correctness and safety over speed."
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
        "expert" | "companion" | "executor" => raw_profile,
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
            "persona prompt file is empty, fallback to built-in: path={}",
            resolved_path
        );
        builtin_persona_prompt(&profile).to_string()
    } else {
        text.to_string()
    }
}

pub(crate) fn load_runtime_prompt_template(
    workspace_root: &Path,
    rel_path: &str,
    default_template: &str,
) -> String {
    let path = workspace_root.join(rel_path);
    match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => s,
        _ => default_template.to_string(),
    }
}

fn normalize_prompt_vendor_name(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" | "claude" => "claude".to_string(),
        "google" | "gemini" => "google".to_string(),
        "openai" => "openai".to_string(),
        "grok" | "xai" => "grok".to_string(),
        "deepseek" => "deepseek".to_string(),
        "qwen" => "qwen".to_string(),
        "minimax" => "minimax".to_string(),
        "custom" => "openai".to_string(),
        _ => "default".to_string(),
    }
}

pub(crate) fn prompt_vendor_name_from_selected_vendor(selected_vendor: Option<&str>) -> String {
    selected_vendor
        .map(normalize_prompt_vendor_name)
        .unwrap_or_else(|| "default".to_string())
}

pub(crate) fn active_prompt_vendor_name(state: &AppState) -> String {
    if let Some(provider) = state.llm_providers.first() {
        return normalize_prompt_vendor_name(llm_vendor_name(provider));
    }
    if let Some(active) = state.active_provider_type.as_deref() {
        return normalize_prompt_vendor_name(active);
    }
    "default".to_string()
}

pub(crate) fn resolve_prompt_rel_path_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> String {
    let trimmed = rel_path.trim();
    if trimmed.is_empty() || !trimmed.starts_with("prompts/") {
        return trimmed.to_string();
    }
    let suffix = trimmed.trim_start_matches("prompts/");
    let vendor_candidate = format!("prompts/vendors/{vendor}/{suffix}");
    if workspace_root.join(&vendor_candidate).is_file() {
        return vendor_candidate;
    }
    let default_candidate = format!("prompts/vendors/default/{suffix}");
    if workspace_root.join(&default_candidate).is_file() {
        return default_candidate;
    }
    if suffix.starts_with("skills/") {
        return default_candidate;
    }
    trimmed.to_string()
}

pub(crate) fn load_prompt_template_for_vendor(
    workspace_root: &Path,
    selected_vendor: Option<&str>,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = prompt_vendor_name_from_selected_vendor(selected_vendor);
    let resolved_path = resolve_prompt_rel_path_for_vendor(workspace_root, &vendor, rel_path);
    let template = load_runtime_prompt_template(workspace_root, &resolved_path, default_template);
    (template, resolved_path)
}

pub(crate) fn load_prompt_template_for_state(
    state: &AppState,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor = active_prompt_vendor_name(state);
    let resolved_path =
        resolve_prompt_rel_path_for_vendor(&state.workspace_root, &vendor, rel_path);
    let template =
        load_runtime_prompt_template(&state.workspace_root, &resolved_path, default_template);
    (template, resolved_path)
}
