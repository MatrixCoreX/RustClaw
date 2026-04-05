use std::path::Path;

use serde::Deserialize;

const PROMPT_LAYER_MANIFEST_PATH: &str = "prompts/layers/manifest.toml";
const SKILL_LAYER_BASE_PATH: &str = "prompts/layers/base/skills/common_rules.md";
const SKILL_LAYER_BODY_DIR: &str = "prompts/layers/generated/skills";
const LAYERED_PROMPT_SOURCE_PREFIX: &str = "layered:";

#[derive(Debug, Deserialize)]
struct PromptLayerManifest {
    #[serde(default)]
    prompts: Vec<PromptLayerEntry>,
}

#[derive(Debug, Deserialize)]
struct PromptLayerEntry {
    logical_path: String,
    #[serde(default)]
    base: Vec<String>,
    #[serde(default)]
    overlay: Vec<String>,
    vendor_patch: Option<String>,
}

pub fn normalize_prompt_vendor_name(raw: &str) -> String {
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

pub fn resolve_prompt_rel_path_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> String {
    let trimmed = rel_path.trim();
    if trimmed.is_empty() || !trimmed.starts_with("prompts/") {
        return trimmed.to_string();
    }
    let _vendor_name = normalize_prompt_vendor_name(vendor);
    if let Some(skill_body_rel) = canonical_skill_prompt_body_rel_path(trimmed) {
        if workspace_root.join(&skill_body_rel).is_file() {
            return skill_body_rel;
        }
        return trimmed.to_string();
    }
    let manifest = prompt_layer_manifest(workspace_root);
    if manifest
        .as_ref()
        .and_then(|manifest| layered_prompt_entry(manifest, trimmed))
        .is_some()
    {
        return trimmed.to_string();
    }
    trimmed.to_string()
}

pub fn canonical_skill_prompt_logical_path(rel_path: &str) -> Option<String> {
    let skill_suffix = skill_prompt_suffix(rel_path)?;
    Some(format!("prompts/skills/{skill_suffix}"))
}

pub fn canonical_skill_prompt_body_rel_path(rel_path: &str) -> Option<String> {
    let skill_suffix = skill_prompt_suffix(rel_path)?;
    Some(format!("{SKILL_LAYER_BODY_DIR}/{skill_suffix}"))
}

pub fn load_prompt_template_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
    default_template: &str,
) -> (String, String) {
    let vendor_name = normalize_prompt_vendor_name(vendor);
    if let Some(rendered) =
        load_layered_prompt_template_for_vendor(workspace_root, &vendor_name, rel_path)
    {
        return rendered;
    }
    let resolved_path = resolve_prompt_rel_path_for_vendor(workspace_root, &vendor_name, rel_path);
    let template = match std::fs::read_to_string(workspace_root.join(&resolved_path)) {
        Ok(text) if !text.trim().is_empty() => text,
        _ => default_template.to_string(),
    };
    (template, resolved_path)
}

fn load_layered_prompt_template_for_vendor(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> Option<(String, String)> {
    if !rel_path.trim().starts_with("prompts/") {
        return None;
    }
    if let Some(skill_prompt) = load_layered_skill_prompt(workspace_root, vendor, rel_path) {
        return Some(skill_prompt);
    }
    let manifest = prompt_layer_manifest(workspace_root)?;
    let entry = layered_prompt_entry(&manifest, rel_path.trim())?;
    let mut parts = Vec::new();
    for part in &entry.base {
        parts.push(read_prompt_part(workspace_root, part)?);
    }
    if let Some(patch_rel) = entry.vendor_patch.as_deref() {
        for candidate in vendor_patch_candidates(vendor, patch_rel) {
            if let Some(patch) = read_optional_prompt_part(workspace_root, &candidate) {
                parts.push(patch);
                break;
            }
        }
    }
    for part in &entry.overlay {
        parts.push(read_prompt_part(workspace_root, part)?);
    }
    let rendered = compose_prompt_parts(parts)?;
    Some((
        rendered,
        format!("{LAYERED_PROMPT_SOURCE_PREFIX}{rel_path}#vendor={vendor}"),
    ))
}

fn load_layered_skill_prompt(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> Option<(String, String)> {
    let logical_skill_rel = canonical_skill_prompt_logical_path(rel_path)?;
    let default_skill_rel = canonical_skill_prompt_body_rel_path(rel_path)?;
    let skill_name = default_skill_rel
        .trim_start_matches(&format!("{SKILL_LAYER_BODY_DIR}/"))
        .to_string();
    let mut parts = Vec::new();
    if let Some(base_rules) = read_optional_prompt_part(workspace_root, SKILL_LAYER_BASE_PATH) {
        parts.push(base_rules);
    }
    let default_skill_body = read_prompt_part(workspace_root, &default_skill_rel)?;
    parts.push(default_skill_body);
    for candidate in vendor_patch_candidates(vendor, &format!("skills/{skill_name}")) {
        if let Some(patch) = read_optional_prompt_part(workspace_root, &candidate) {
            parts.push(patch);
            break;
        }
    }
    let rendered = compose_prompt_parts(parts)?;
    Some((
        rendered,
        format!("{LAYERED_PROMPT_SOURCE_PREFIX}{logical_skill_rel}#vendor={vendor}#skill"),
    ))
}

fn read_prompt_part(workspace_root: &Path, rel_path: &str) -> Option<String> {
    std::fs::read_to_string(workspace_root.join(rel_path))
        .ok()
        .map(|text| strip_legacy_vendor_tuning(&text))
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn read_optional_prompt_part(workspace_root: &Path, rel_path: &str) -> Option<String> {
    std::fs::read_to_string(workspace_root.join(rel_path))
        .ok()
        .map(|text| strip_legacy_vendor_tuning(&text))
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

fn strip_legacy_vendor_tuning(text: &str) -> String {
    const BODY_STARTERS: &[&str] = &[
        "You ",
        "**",
        "Task:",
        "Input:",
        "Rules:",
        "Output format:",
        "Routing rules",
        "Goal/context:",
        "User follow-up:",
        "User request:",
        "Execution policy:",
        "Decision rules:",
        "Interpretation hints:",
        "Primary goal:",
        "Schema:",
        "Context:",
        "Language policy",
        "Summarize ",
        "Transcribe ",
    ];

    let mut lines_out = Vec::new();
    let mut skipping_vendor = false;
    let mut touched = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if !skipping_vendor && trimmed.starts_with("Vendor tuning for ") {
            skipping_vendor = true;
            touched = true;
            continue;
        }
        if skipping_vendor {
            if trimmed.is_empty() {
                continue;
            }
            let is_body_start = BODY_STARTERS
                .iter()
                .any(|prefix| trimmed.starts_with(prefix));
            if !is_body_start && (trimmed.starts_with('-') || trimmed.ends_with(':')) {
                continue;
            }
            skipping_vendor = false;
        }
        lines_out.push(line);
    }
    if touched {
        lines_out.join("\n")
    } else {
        text.to_string()
    }
}

fn prompt_layer_manifest(workspace_root: &Path) -> Option<PromptLayerManifest> {
    let path = workspace_root.join(PROMPT_LAYER_MANIFEST_PATH);
    let raw = std::fs::read_to_string(path).ok()?;
    toml::from_str::<PromptLayerManifest>(&raw).ok()
}

fn layered_prompt_entry<'a>(
    manifest: &'a PromptLayerManifest,
    rel_path: &str,
) -> Option<&'a PromptLayerEntry> {
    manifest
        .prompts
        .iter()
        .find(|entry| entry.logical_path == rel_path)
}

fn vendor_patch_candidates(vendor: &str, patch_rel: &str) -> Vec<String> {
    let mut out = vec![format!(
        "prompts/layers/vendor_patches/{vendor}/{patch_rel}"
    )];
    if vendor != "default" {
        out.push(format!("prompts/layers/vendor_patches/default/{patch_rel}"));
    }
    out
}

fn compose_prompt_parts(parts: Vec<String>) -> Option<String> {
    let collected: Vec<String> = parts
        .into_iter()
        .map(|part| part.trim().to_string())
        .filter(|part| !part.is_empty())
        .collect();
    if collected.is_empty() {
        None
    } else {
        Some(collected.join("\n\n"))
    }
}

fn skill_prompt_suffix(rel_path: &str) -> Option<&str> {
    let suffix = rel_path.trim().strip_prefix("prompts/")?;
    if let Some(skill_suffix) = suffix.strip_prefix("skills/") {
        return Some(skill_suffix);
    }
    if let Some(rest) = suffix.strip_prefix("layers/generated/skills/") {
        return Some(rest);
    }
    if let Some(rest) = suffix.strip_prefix("vendors/") {
        let (_, skill_suffix) = rest.split_once("/skills/")?;
        return Some(skill_suffix);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("rustclaw_prompt_layers_{name}_{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn write_file(root: &Path, rel_path: &str, text: &str) {
        let path = root.join(rel_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    #[test]
    fn test_skill_prompt_layering_supports_logical_and_legacy_vendor_paths() {
        let root = temp_workspace("skill_paths");
        write_file(
            &root,
            "prompts/layers/base/skills/common_rules.md",
            "base rules",
        );
        write_file(
            &root,
            "prompts/layers/generated/skills/demo.md",
            "demo body",
        );
        write_file(
            &root,
            "prompts/layers/vendor_patches/claude/skills/demo.md",
            "claude patch",
        );

        let logical =
            load_prompt_template_for_vendor(&root, "claude", "prompts/skills/demo.md", "");
        let legacy = load_prompt_template_for_vendor(
            &root,
            "claude",
            "prompts/layers/generated/skills/demo.md",
            "",
        );

        assert_eq!(logical.0, "base rules\n\ndemo body\n\nclaude patch");
        assert_eq!(legacy.0, logical.0);
        assert!(logical.1.contains("prompts/skills/demo.md"));
        assert!(legacy.1.contains("prompts/skills/demo.md"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_manifest_is_reloaded_after_file_change() {
        let root = temp_workspace("manifest_reload");
        write_file(
            &root,
            "prompts/layers/manifest.toml",
            r#"
[[prompts]]
logical_path = "prompts/test_prompt.md"
base = ["prompts/layers/base/test.md"]
overlay = ["prompts/layers/overlays/first.md"]
"#,
        );
        write_file(&root, "prompts/layers/base/test.md", "base");
        write_file(&root, "prompts/layers/overlays/first.md", "first");
        write_file(&root, "prompts/layers/overlays/second.md", "second");

        let first =
            load_prompt_template_for_vendor(&root, "openai", "prompts/test_prompt.md", "").0;
        assert_eq!(first, "base\n\nfirst");

        write_file(
            &root,
            "prompts/layers/manifest.toml",
            r#"
[[prompts]]
logical_path = "prompts/test_prompt.md"
base = ["prompts/layers/base/test.md"]
overlay = ["prompts/layers/overlays/second.md"]
"#,
        );

        let second =
            load_prompt_template_for_vendor(&root, "openai", "prompts/test_prompt.md", "").0;
        assert_eq!(second, "base\n\nsecond");

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn test_unregistered_prompt_no_longer_falls_back_to_vendor_tree() {
        let root = temp_workspace("no_vendor_fallback");
        write_file(&root, "prompts/vendors/default/legacy_only.md", "legacy");

        let resolved =
            resolve_prompt_rel_path_for_vendor(&root, "openai", "prompts/legacy_only.md");

        assert_eq!(resolved, "prompts/legacy_only.md");

        let _ = fs::remove_dir_all(root);
    }
}
