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
fn prompt_vendor_normalization_groups_openai_compatible_models() {
    assert_eq!(normalize_prompt_vendor_name("openai"), "openai");
    assert_eq!(normalize_prompt_vendor_name("mimo"), "openai");
    assert_eq!(normalize_prompt_vendor_name("xiaomi"), "openai");
    assert_eq!(normalize_prompt_vendor_name("custom"), "openai");
    assert_eq!(normalize_prompt_vendor_name("minimax"), "minimax");
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

    let logical = load_prompt_template_for_vendor(&root, "claude", "prompts/skills/demo.md", "");
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

    let first = load_prompt_template_for_vendor(&root, "openai", "prompts/test_prompt.md", "").0;
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

    let second = load_prompt_template_for_vendor(&root, "openai", "prompts/test_prompt.md", "").0;
    assert_eq!(second, "base\n\nsecond");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn test_layered_prompt_appends_vendor_patch_after_overlay() {
    let root = temp_workspace("vendor_patch_order");
    write_file(
        &root,
        "prompts/layers/manifest.toml",
        r#"
[[prompts]]
logical_path = "prompts/test_prompt.md"
base = ["prompts/layers/base/test.md"]
overlay = ["prompts/layers/overlays/test.md"]
vendor_patch = "routing/common.md"
"#,
    );
    write_file(&root, "prompts/layers/base/test.md", "base");
    write_file(&root, "prompts/layers/overlays/test.md", "overlay");
    write_file(
        &root,
        "prompts/layers/vendor_patches/minimax/routing/common.md",
        "vendor patch",
    );

    let rendered =
        load_prompt_template_for_vendor(&root, "minimax", "prompts/test_prompt.md", "").0;

    assert_eq!(rendered, "base\n\noverlay\n\nvendor patch");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn test_unregistered_prompt_no_longer_falls_back_to_vendor_tree() {
    let root = temp_workspace("no_vendor_fallback");
    write_file(&root, "prompts/vendors/default/legacy_only.md", "legacy");

    let resolved = resolve_prompt_rel_path_for_vendor(&root, "openai", "prompts/legacy_only.md");

    assert_eq!(resolved, "prompts/legacy_only.md");

    let _ = fs::remove_dir_all(root);
}

// ============================================================
// §3.5a prompt version 提取测试
// ============================================================

#[test]
fn version_extracts_from_html_comment_single_line() {
    let v = extract_prompt_version("<!-- version: 2026-04-17.1 -->\n\nBody text.");
    assert_eq!(v.as_deref(), Some("2026-04-17.1"));
}

#[test]
fn version_extracts_from_html_comment_within_metadata_block() {
    let text = "<!--\nPurpose: foo\nComponent: bar\nVersion: 2026.4.17-rc1\n-->\n\nBody.";
    assert_eq!(
        extract_prompt_version(text).as_deref(),
        Some("2026.4.17-rc1")
    );
}

#[test]
fn version_extracts_from_yaml_frontmatter() {
    let text = "---\ntitle: foo\nversion: 1.0.0\n---\n\nBody.";
    assert_eq!(extract_prompt_version(text).as_deref(), Some("1.0.0"));
}

#[test]
fn version_extraction_returns_none_when_absent() {
    assert!(extract_prompt_version("just plain prompt body").is_none());
    assert!(extract_prompt_version("<!-- Purpose: foo -->\nBody.").is_none());
}

#[test]
fn version_extraction_ignores_late_occurrences() {
    // 第 200+ 行才出现的 "version:" 不被匹配
    let mut text = String::new();
    for _ in 0..100 {
        text.push_str("filler line\n");
    }
    text.push_str("<!-- version: shouldnotmatch -->\n");
    assert!(extract_prompt_version(&text).is_none());
}

#[test]
fn version_extraction_rejects_invalid_chars() {
    // 含空格 / 中文 / 特殊符号的版本号被拒
    assert!(extract_prompt_version("<!-- version: 1.0 with notes -->").is_none());
    assert!(extract_prompt_version("<!-- version: 版本一 -->").is_none());
    assert!(extract_prompt_version("<!-- version: 1.0/2.0 -->").is_none());
}

#[test]
fn version_extraction_rejects_too_long() {
    let long = "a".repeat(80);
    let text = format!("<!-- version: {long} -->");
    assert!(extract_prompt_version(&text).is_none());
}

#[test]
fn version_extraction_strips_quotes() {
    assert_eq!(
        extract_prompt_version(r#"<!-- version: "v1.2.3" -->"#).as_deref(),
        Some("v1.2.3")
    );
    assert_eq!(
        extract_prompt_version("---\nversion: 'v1.2.3'\n---").as_deref(),
        Some("v1.2.3")
    );
}

#[test]
fn with_meta_returns_version_from_disk() {
    let root = temp_workspace("with_meta_disk");
    write_file(
        &root,
        "prompts/layers/manifest.toml",
        r#"
[[prompts]]
logical_path = "prompts/versioned.md"
base = ["prompts/layers/base/versioned.md"]
overlay = []
"#,
    );
    write_file(
        &root,
        "prompts/layers/base/versioned.md",
        "<!-- version: 2026-04-17.1 -->\n\nBody",
    );

    let resolved =
        load_prompt_template_for_vendor_with_meta(&root, "openai", "prompts/versioned.md", "");
    assert_eq!(resolved.version.as_deref(), Some("2026-04-17.1"));
    assert!(resolved.template.contains("Body"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn with_meta_picks_first_versioned_part_in_layered() {
    let root = temp_workspace("with_meta_layered");
    write_file(
        &root,
        "prompts/layers/manifest.toml",
        r#"
[[prompts]]
logical_path = "prompts/multi.md"
base = ["prompts/layers/base/multi.md"]
overlay = ["prompts/layers/overlays/multi_o.md"]
"#,
    );
    // base 无 version，overlay 有 → 应取 overlay 的
    write_file(&root, "prompts/layers/base/multi.md", "base body");
    write_file(
        &root,
        "prompts/layers/overlays/multi_o.md",
        "<!-- version: o1 -->\noverlay body",
    );

    let resolved =
        load_prompt_template_for_vendor_with_meta(&root, "openai", "prompts/multi.md", "");
    assert_eq!(resolved.version.as_deref(), Some("o1"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn with_meta_returns_none_when_no_part_versioned() {
    let root = temp_workspace("with_meta_no_version");
    write_file(
        &root,
        "prompts/layers/manifest.toml",
        r#"
[[prompts]]
logical_path = "prompts/plain.md"
base = ["prompts/layers/base/plain.md"]
overlay = []
"#,
    );
    write_file(&root, "prompts/layers/base/plain.md", "plain body");

    let resolved =
        load_prompt_template_for_vendor_with_meta(&root, "openai", "prompts/plain.md", "");
    assert!(resolved.version.is_none());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn with_meta_falls_back_to_default_template_version() {
    // 磁盘缺该文件时使用默认 template 并提取其中 version
    let root = temp_workspace("with_meta_default");
    let resolved = load_prompt_template_for_vendor_with_meta(
        &root,
        "openai",
        "prompts/missing.md",
        "<!-- version: builtin1 -->\nbuiltin body",
    );
    assert_eq!(resolved.version.as_deref(), Some("builtin1"));
    assert!(resolved.template.contains("builtin body"));

    let _ = fs::remove_dir_all(root);
}
