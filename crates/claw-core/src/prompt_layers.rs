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
        "openai" | "mimo" | "xiaomi" => "openai".to_string(),
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
    let resolved = load_prompt_template_for_vendor_with_meta(
        workspace_root,
        vendor,
        rel_path,
        default_template,
    );
    (resolved.template, resolved.source)
}

/// §3.5a: 已解析的 prompt 模板及其元数据。
///
/// `source` 字段用作日志/审计标识；`version` 取自 prompt 文件中文档化的
/// `<!-- version: ... -->` 注释或 markdown YAML frontmatter `---\nversion: ...\n---`。
/// 拼接 layered prompt 时取**第一个有 version 标识的 part**作为整体版本号。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPromptTemplate {
    pub template: String,
    pub source: String,
    pub version: Option<String>,
}

/// §3.5a 加载入口（带元数据）。返回模板正文 + 解析路径 + 可选版本号。
///
/// 与 `load_prompt_template_for_vendor` 行为一致，仅多输出 `version`。
/// 老调用方继续用 `load_prompt_template_for_vendor` 不受影响。
pub fn load_prompt_template_for_vendor_with_meta(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
    default_template: &str,
) -> ResolvedPromptTemplate {
    let vendor_name = normalize_prompt_vendor_name(vendor);
    if let Some((template, source, version)) =
        load_layered_prompt_template_for_vendor_with_meta(workspace_root, &vendor_name, rel_path)
    {
        return ResolvedPromptTemplate {
            template,
            source,
            version,
        };
    }
    let resolved_path = resolve_prompt_rel_path_for_vendor(workspace_root, &vendor_name, rel_path);
    let (template, version) = match std::fs::read_to_string(workspace_root.join(&resolved_path)) {
        Ok(text) if !text.trim().is_empty() => {
            let version = extract_prompt_version(&text);
            (text, version)
        }
        _ => (
            default_template.to_string(),
            extract_prompt_version(default_template),
        ),
    };
    ResolvedPromptTemplate {
        template,
        source: resolved_path,
        version,
    }
}

/// §3.5a 提取 prompt 文件里声明的版本号。
///
/// 接受两种声明格式（按优先级）：
/// 1. HTML 注释：`<!-- version: 2026-04-17.1 -->`，可前后带空格 / 大小写不敏感的 `Version:`，
///    通常嵌入在 `<!-- Purpose: ... Version: ... -->` 这种 metadata 注释里。
/// 2. YAML frontmatter：文件开头 `---\n...\nversion: 2026-04-17.1\n...\n---`。
///
/// 解析只扫描文件**前 80 行**，避免大段 prompt body 里碰巧出现 "version:" 字样
/// 被误识别。版本号必须由 `[A-Za-z0-9._-]+` 组成，最长 64 字符。
///
/// 解析失败 / 未声明返回 `None`，**不**修改 template 内容（version 注释照常进 prompt
/// 文本，模型基本不在意 HTML 注释）。
pub fn extract_prompt_version(text: &str) -> Option<String> {
    let mut in_frontmatter = false;
    let mut in_html_comment = false;
    for (idx, line) in text.lines().take(80).enumerate() {
        let trimmed = line.trim();
        // YAML frontmatter 起止
        if idx == 0 && trimmed == "---" {
            in_frontmatter = true;
            continue;
        }
        if in_frontmatter {
            if trimmed == "---" {
                in_frontmatter = false;
                continue;
            }
            if let Some(v) = parse_version_kv_line(trimmed, false) {
                return Some(v);
            }
            continue;
        }
        // HTML 注释跨行处理
        let opened_here = trimmed.starts_with("<!--");
        let closes_here = trimmed.contains("-->");
        if opened_here {
            in_html_comment = !closes_here;
        }
        if in_html_comment || opened_here {
            if let Some(v) = parse_version_kv_line(trimmed, true) {
                return Some(v);
            }
            if closes_here {
                in_html_comment = false;
            }
            continue;
        }
        if closes_here && in_html_comment {
            // 注释中的最后一行如 `Version: x -->`
            if let Some(v) = parse_version_kv_line(trimmed, true) {
                return Some(v);
            }
            in_html_comment = false;
            continue;
        }
        // 不在任何 metadata 块里：单行 HTML 注释（不以 <!-- 开头但通过 contains 容错）
        if line.contains("<!--") && line.contains("version") {
            if let Some(v) = parse_version_kv_line(line, true) {
                return Some(v);
            }
        }
    }
    None
}

/// 解析"key: value"形式的一行；`allow_comment_prefix` 为 true 时允许前缀里出现 `<!--`/`#`。
fn parse_version_kv_line(line: &str, allow_comment_prefix: bool) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let key = "version:";
    let idx = lower.find(key)?;
    let prefix = &line[..idx];
    let allowed_chars: &[char] = if allow_comment_prefix {
        &[' ', '\t', '<', '!', '-', '#']
    } else {
        &[' ', '\t']
    };
    if !prefix.chars().all(|c| allowed_chars.contains(&c)) {
        return None;
    }
    let after = &line[idx + key.len()..];
    let value_segment = after.split("-->").next().unwrap_or(after);
    sanitize_version(value_segment)
}

fn sanitize_version(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| matches!(c, '"' | '\'' | ',' | ';'));
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() > 64 {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
    {
        return None;
    }
    Some(trimmed.to_string())
}

fn load_layered_prompt_template_for_vendor_with_meta(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> Option<(String, String, Option<String>)> {
    if !rel_path.trim().starts_with("prompts/") {
        return None;
    }
    if let Some(skill_prompt) =
        load_layered_skill_prompt_with_meta(workspace_root, vendor, rel_path)
    {
        return Some(skill_prompt);
    }
    let manifest = prompt_layer_manifest(workspace_root)?;
    let entry = layered_prompt_entry(&manifest, rel_path.trim())?;
    let mut parts: Vec<String> = Vec::new();
    let mut version: Option<String> = None;
    for part in &entry.base {
        let raw = read_prompt_part_raw(workspace_root, part)?;
        if version.is_none() {
            version = extract_prompt_version(&raw);
        }
        parts.push(normalize_prompt_part_body(&raw));
    }
    for part in &entry.overlay {
        let raw = read_prompt_part_raw(workspace_root, part)?;
        if version.is_none() {
            version = extract_prompt_version(&raw);
        }
        parts.push(normalize_prompt_part_body(&raw));
    }
    if let Some(patch_rel) = entry.vendor_patch.as_deref() {
        for candidate in vendor_patch_candidates(vendor, patch_rel) {
            if let Some(raw) = read_optional_prompt_part_raw(workspace_root, &candidate) {
                if version.is_none() {
                    version = extract_prompt_version(&raw);
                }
                parts.push(normalize_prompt_part_body(&raw));
                break;
            }
        }
    }
    let parts_filtered: Vec<String> = parts.into_iter().filter(|s| !s.is_empty()).collect();
    let rendered = compose_prompt_parts(parts_filtered)?;
    Some((
        rendered,
        format!("{LAYERED_PROMPT_SOURCE_PREFIX}{rel_path}#vendor={vendor}"),
        version,
    ))
}

fn load_layered_skill_prompt_with_meta(
    workspace_root: &Path,
    vendor: &str,
    rel_path: &str,
) -> Option<(String, String, Option<String>)> {
    let logical_skill_rel = canonical_skill_prompt_logical_path(rel_path)?;
    let default_skill_rel = canonical_skill_prompt_body_rel_path(rel_path)?;
    let skill_name = default_skill_rel
        .trim_start_matches(&format!("{SKILL_LAYER_BODY_DIR}/"))
        .to_string();
    let mut parts: Vec<String> = Vec::new();
    let mut version: Option<String> = None;
    if let Some(base_raw) = read_optional_prompt_part_raw(workspace_root, SKILL_LAYER_BASE_PATH) {
        if version.is_none() {
            version = extract_prompt_version(&base_raw);
        }
        parts.push(normalize_prompt_part_body(&base_raw));
    }
    let default_skill_raw = read_prompt_part_raw(workspace_root, &default_skill_rel)?;
    if version.is_none() {
        version = extract_prompt_version(&default_skill_raw);
    }
    parts.push(normalize_prompt_part_body(&default_skill_raw));
    for patch_rel in [
        "skills/common.md".to_string(),
        format!("skills/{skill_name}"),
    ] {
        for candidate in vendor_patch_candidates(vendor, &patch_rel) {
            if let Some(patch_raw) = read_optional_prompt_part_raw(workspace_root, &candidate) {
                if version.is_none() {
                    version = extract_prompt_version(&patch_raw);
                }
                parts.push(normalize_prompt_part_body(&patch_raw));
                break;
            }
        }
    }
    let parts_filtered: Vec<String> = parts.into_iter().filter(|s| !s.is_empty()).collect();
    let rendered = compose_prompt_parts(parts_filtered)?;
    Some((
        rendered,
        format!("{LAYERED_PROMPT_SOURCE_PREFIX}{logical_skill_rel}#vendor={vendor}#skill"),
        version,
    ))
}

/// 读 prompt 文件原文，**不**剥离 vendor tuning，**不** trim。
/// 用于提取 `<!-- version: ... -->` 等需要原始文本的元数据；
/// 调用方再各自 `strip_legacy_vendor_tuning(&text).trim()` 拿正文。
fn read_prompt_part_raw(workspace_root: &Path, rel_path: &str) -> Option<String> {
    std::fs::read_to_string(workspace_root.join(rel_path))
        .ok()
        .filter(|text| !text.trim().is_empty())
}

fn read_optional_prompt_part_raw(workspace_root: &Path, rel_path: &str) -> Option<String> {
    read_prompt_part_raw(workspace_root, rel_path)
}

fn normalize_prompt_part_body(text: &str) -> String {
    strip_legacy_vendor_tuning(text).trim().to_string()
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
#[path = "prompt_layers_tests.rs"]
mod tests;
