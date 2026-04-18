use claw_core::skill_registry::{Capability, SkillsRegistry, REQUIRED_BUILTIN_SKILLS};
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

fn parse_toml(path: &Path) -> toml::Value {
    toml::from_str(&fs::read_to_string(path).expect("read config")).expect("parse toml")
}

fn minimax_models(value: &toml::Value) -> Vec<String> {
    value["llm"]["minimax"]["models"]
        .as_array()
        .expect("minimax models")
        .iter()
        .filter_map(|item| item.as_str())
        .map(str::to_string)
        .collect()
}

fn minimax_default_model(value: &toml::Value) -> String {
    value["llm"]["minimax"]["model"]
        .as_str()
        .expect("minimax default model")
        .to_string()
}

#[test]
fn minimax_templates_allow_the_repo_default_model() {
    let root = workspace_root();
    let root_config = parse_toml(&root.join("configs/config.toml"));
    let docker_config = parse_toml(&root.join("docker/config/config.toml"));

    let selected_model = root_config["llm"]["selected_model"]
        .as_str()
        .expect("root selected model");
    let root_models = minimax_models(&root_config);
    let docker_models = minimax_models(&docker_config);

    assert!(
        root_models.iter().any(|model| model == selected_model),
        "root minimax models should include selected model {selected_model}, got {root_models:?}"
    );
    assert!(
        docker_models.iter().any(|model| model == selected_model),
        "docker minimax models should include selected model {selected_model}, got {docker_models:?}"
    );
    assert_eq!(
        minimax_default_model(&root_config),
        minimax_default_model(&docker_config),
        "root and docker minimax defaults should stay aligned",
    );
}

/// §P4.1: 历史上 `crates/clawd/src/skills.rs::canonical_skill_name`
/// 维护过一张 16 行硬编码 alias→canonical 表，与 `configs/skills_registry.toml`
/// 里 `[[skills]].aliases` 同时存在（双源真相）。本轮把那张表清空，
/// 让 registry 成为唯一别名权威。
///
/// 这条测试是\"防 alias 倒退\"的守底网：如果以后有人改 registry 时
/// 漏删了某个别名，CI 会立刻命中这里。每一条都对应迁移前 hardcoded 表里的
/// 一条 (alias → canonical) 对，**不要随便删**。
#[test]
fn registry_covers_all_legacy_hardcoded_aliases() {
    let cases: &[(&str, &str)] = &[
        // fs_search 簇（含历史拼写错容错 fs_rearch）
        ("fs_rearch", "fs_search"),
        ("fs-search", "fs_search"),
        ("filesystem_search", "fs_search"),
        ("file_search", "fs_search"),
        ("search_files", "fs_search"),
        // package_manager 簇
        ("package_install", "package_manager"),
        ("pkg_manager", "package_manager"),
        ("packages", "package_manager"),
        // install_module 簇
        ("module_install", "install_module"),
        ("install_modules", "install_module"),
        // process_basic 簇
        ("process", "process_basic"),
        ("process_manager", "process_basic"),
        // archive_basic 簇
        ("archive", "archive_basic"),
        ("archive_tool", "archive_basic"),
        // db_basic 簇
        ("database", "db_basic"),
        ("sqlite_tool", "db_basic"),
        // docker_basic 簇
        ("docker", "docker_basic"),
        ("docker_ops", "docker_basic"),
        // rss_fetch 簇
        ("rss", "rss_fetch"),
        ("rss_reader", "rss_fetch"),
        ("rss_fetcher", "rss_fetch"),
        // image_vision 簇
        ("image_vision_skill", "image_vision"),
        ("vision", "image_vision"),
        ("vision_image", "image_vision"),
        ("image-analyze", "image_vision"),
        // image_generate 簇
        ("image_generation", "image_generate"),
        ("generate_image", "image_generate"),
        ("draw_image", "image_generate"),
        ("text_to_image", "image_generate"),
        // image_edit 簇
        ("image_modify", "image_edit"),
        ("image_editor", "image_edit"),
        ("edit_image", "image_edit"),
        ("image_outpaint", "image_edit"),
        // crypto 簇
        ("coin", "crypto"),
        ("coins", "crypto"),
        ("crypto_trade", "crypto"),
        ("market_data", "crypto"),
        ("crypto_market", "crypto"),
        // chat 簇
        ("talk", "chat"),
        ("smalltalk", "chat"),
        ("joke", "chat"),
        ("chitchat", "chat"),
        // 单别名 → *_basic 簇
        ("git", "git_basic"),
        ("http", "http_basic"),
        ("system", "system_basic"),
    ];

    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");
        for (alias, expected_canonical) in cases {
            let resolved = registry.resolve_canonical(alias).unwrap_or_else(|| {
                panic!(
                    "{}: alias `{alias}` is no longer registered (expected canonical `{expected_canonical}`)",
                    path.display()
                )
            });
            assert_eq!(
                resolved,
                *expected_canonical,
                "{}: alias `{alias}` resolved to `{resolved}`, expected `{expected_canonical}`",
                path.display()
            );
        }
    }
}

/// §P4.1 收尾：clawd 启动期会拒绝缺失或 kind 漂移的 builtin。这条测试是
/// CI 等价物 —— 把仓内两份 registry 用同一个 `integrity_report` 跑一遍，
/// 任何漏 builtin / kind 写错都会在 PR 阶段就红。
#[test]
fn registry_covers_all_required_builtins() {
    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");
        let report = registry.integrity_report();
        assert!(
            report.is_clean(),
            "{}: registry integrity check failed (REQUIRED_BUILTIN_SKILLS={:?}): {}",
            path.display(),
            REQUIRED_BUILTIN_SKILLS,
            report.into_human_message().unwrap_or_default()
        );
    }
}

/// §P4.1 主体：示范技能 image_generate 必须按 schema 声明 capabilities。
///
/// 本测试同时承担两个守底职责：
/// 1. 防止 image_generate 的 capabilities 被误改/误删（运行期会有策略层依赖）；
/// 2. 防止其他未声明能力的技能被偷偷加上能力 —— 当下没有第三方 audit 入口时，
///    这条测试是最便宜的"显式声明才能放开"门闸。新增带 capabilities 的技能时，
///    把它加到 `expected_with_caps` 列表里。
#[test]
fn registry_capabilities_declared_match_expected_demo_skill() {
    // (canonical, sorted-tokens) — sorted 顺序与 SkillsRegistry::load_from_path
    // 内部 dedup+sort 后的结果一致。
    let expected_with_caps: &[(&str, &[&str])] = &[
        // 首批示例：图像生成需要 LLM 网关 + 对外网络 + 写盘。
        ("image_generate", &["fs.write", "llm", "net"]),
    ];

    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");

        for (skill, expected) in expected_with_caps {
            let actual: Vec<String> = registry
                .capabilities(skill)
                .iter()
                .map(Capability::as_token)
                .collect();
            let want: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
            assert_eq!(
                actual,
                want,
                "{}: skill `{skill}` declared capabilities drift; expected {:?}, got {:?}",
                path.display(),
                want,
                actual,
            );
        }

        // 守底：除了 expected_with_caps 列表里的技能，其他任何技能都不应该有
        // capabilities 声明（确保新增技能的 capability 必须显式入这条测试，
        // 任何"偷偷加权限"的 PR 都会红）。
        let allowed: std::collections::HashSet<&str> =
            expected_with_caps.iter().map(|(s, _)| *s).collect();
        for name in registry.all_names() {
            if allowed.contains(name.as_str()) {
                continue;
            }
            let caps = registry.capabilities(&name);
            assert!(
                caps.is_empty(),
                "{}: skill `{name}` declares capabilities {:?} but is not in `expected_with_caps`; \
                 add it to the test allowlist if intentional",
                path.display(),
                caps.iter().map(Capability::as_token).collect::<Vec<_>>(),
            );
        }
    }
}
