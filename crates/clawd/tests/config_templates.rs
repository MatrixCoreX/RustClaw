use claw_core::secrets::{provision_secret_envs, SecretValue, SecretsBroker, SecretsError};
use claw_core::skill_registry::{
    Capability, PlannerCapabilityKind, SkillsRegistry, REQUIRED_BUILTIN_SKILLS,
};
use std::collections::BTreeSet;
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

fn mimo_models(value: &toml::Value) -> Vec<String> {
    value["llm"]["mimo"]["models"]
        .as_array()
        .expect("mimo models")
        .iter()
        .filter_map(|item| item.as_str())
        .map(str::to_string)
        .collect()
}

fn mimo_default_model(value: &toml::Value) -> String {
    value["llm"]["mimo"]["model"]
        .as_str()
        .expect("mimo default model")
        .to_string()
}

fn prompts_strict_validation(value: &toml::Value) -> bool {
    value["prompts"]["strict_validation_at_startup"]
        .as_bool()
        .expect("prompts.strict_validation_at_startup")
}

fn strict_json_overlay_prompt_files() -> BTreeSet<String> {
    let overlay_dir = workspace_root().join("prompts/layers/overlays");
    let markers = [
        "Output JSON only",
        "Return JSON only",
        "strict JSON only",
        "Always output valid JSON",
        "Return valid JSON only",
        "valid JSON only",
        "return compact valid JSON",
        "return valid JSON matching this schema",
    ];
    let mut hits = BTreeSet::new();

    for entry in fs::read_dir(&overlay_dir).expect("read overlay dir") {
        let entry = entry.expect("overlay entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let raw = fs::read_to_string(&path).expect("read overlay prompt");
        if markers.iter().any(|marker| raw.contains(marker)) {
            hits.insert(
                path.file_name()
                    .and_then(|name| name.to_str())
                    .expect("overlay file name")
                    .to_string(),
            );
        }
    }

    hits
}

#[test]
fn minimax_templates_allow_the_repo_default_model() {
    let root = workspace_root();
    let root_config = parse_toml(&root.join("configs/config.toml"));
    let docker_config = parse_toml(&root.join("docker/config/config.toml"));

    let root_model = minimax_default_model(&root_config);
    let root_models = minimax_models(&root_config);
    let docker_models = minimax_models(&docker_config);

    assert!(
        root_models.iter().any(|model| model == &root_model),
        "root minimax models should include default model {root_model}, got {root_models:?}"
    );
    assert!(
        docker_models.iter().any(|model| model == &root_model),
        "docker minimax models should include root default model {root_model}, got {docker_models:?}"
    );
    assert_eq!(
        root_model,
        minimax_default_model(&docker_config),
        "root and docker minimax defaults should stay aligned",
    );
}

#[test]
fn mimo_templates_allow_the_repo_default_model() {
    let root = workspace_root();
    let root_config = parse_toml(&root.join("configs/config.toml"));
    let docker_config = parse_toml(&root.join("docker/config/config.toml"));

    let root_model = mimo_default_model(&root_config);
    let root_models = mimo_models(&root_config);
    let docker_models = mimo_models(&docker_config);

    assert!(
        root_models.iter().any(|model| model == &root_model),
        "root mimo models should include default model {root_model}, got {root_models:?}"
    );
    assert!(
        docker_models.iter().any(|model| model == &root_model),
        "docker mimo models should include root default model {root_model}, got {docker_models:?}"
    );
    assert_eq!(
        root_model,
        mimo_default_model(&docker_config),
        "root and docker mimo defaults should stay aligned",
    );
}

#[test]
fn prompt_validation_templates_enable_strict_startup_gate_consistently() {
    let root = workspace_root();
    let root_config = parse_toml(&root.join("configs/config.toml"));
    let docker_config = parse_toml(&root.join("docker/config/config.toml"));

    assert!(
        prompts_strict_validation(&root_config),
        "repo config should enable prompts.strict_validation_at_startup"
    );
    assert_eq!(
        prompts_strict_validation(&root_config),
        prompts_strict_validation(&docker_config),
        "root and docker prompt validation strictness should stay aligned",
    );
}

#[test]
fn docker_contract_matrix_template_stays_in_sync() {
    let root = workspace_root();
    let main_matrix = root.join("configs/task_contract_matrix.toml");
    let docker_matrix = root.join("docker/config/task_contract_matrix.toml");

    assert!(
        docker_matrix.exists(),
        "docker mounted config template must include task_contract_matrix.toml"
    );
    assert_eq!(
        fs::read_to_string(&main_matrix).expect("read main contract matrix"),
        fs::read_to_string(&docker_matrix).expect("read docker contract matrix"),
        "docker task_contract_matrix.toml should stay identical to configs/task_contract_matrix.toml"
    );

    let entrypoint =
        fs::read_to_string(root.join("docker/docker-entrypoint.sh")).expect("read entrypoint");
    assert!(
        entrypoint.contains("MOUNTED_CONTRACT_MATRIX_FILE"),
        "docker entrypoint must sync mounted task_contract_matrix.toml overrides"
    );
}

/// §3.5c-D14 守底：所有带“strict JSON / JSON only”输出约束的 overlay prompt
/// 都必须被显式归类为：
/// 1. 已有固定 schema 守底；或
/// 2. 明确排除（legacy / 动态 user-schema / 混合 dispatcher）。
///
/// 这样后续新增 prompt 时，如果它落入“严格 JSON 输出”集合但没人更新 schema
/// inventory / 计划口径，CI 会直接红，避免 §3.5c backlog 再次漂移。
#[test]
fn strict_json_overlay_prompts_are_classified_in_schema_inventory() {
    let observed = strict_json_overlay_prompt_files();

    let schema_backed: BTreeSet<String> = [
        "image_reference_resolver_prompt.md",
        "image_vision_action_compare.md",
        "image_vision_action_describe.md",
        "image_vision_action_screenshot_summary.md",
        "language_infer_prompt.md",
        "observed_answer_fallback_prompt.md",
        "schedule_intent_prompt.md",
        "voice_mode_intent_prompt.md",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    let intentionally_excluded: BTreeSet<String> = [
        "image_vision_action_extract_default.md",
        "image_vision_action_extract_with_schema.md",
        "image_vision_action_fallback.md",
        "image_vision_prompt.md",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();

    let classified = schema_backed
        .union(&intentionally_excluded)
        .cloned()
        .collect::<BTreeSet<_>>();

    assert_eq!(
        observed, classified,
        "strict-JSON overlay inventory drifted; every matching prompt must be classified as schema-backed or intentionally excluded.\nobserved={observed:?}\nclassified={classified:?}"
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

/// §P4.2：仓内两份 registry 必须满足 capability ↔ shape 一致性
/// （exec.sudo 必须 confirm + high；fs.write/exec 不允许显式 side_effect=false）。
/// 这里在 CI 跑一遍，确保 dev 改 registry 时漏配立刻红。
#[test]
fn registry_capability_shape_consistency_is_clean() {
    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");
        let violations = registry.validate_shape_consistency();
        assert!(
            violations.is_empty(),
            "{}: capability shape consistency violations:\n  - {}",
            path.display(),
            violations.join("\n  - ")
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
    let main_expected_with_caps: &[(&str, &[&str])] = &[
        // 主配置中 image_edit / image_vision 可复用同厂商全局 key，不声明专用
        // secrets capability；image_generate 仍显式要求专用生成 key。
        ("audio_synthesize", &["fs.write", "llm", "net"]),
        ("audio_transcribe", &["fs.read", "llm", "net"]),
        ("browser_web", &["fs.write", "net"]),
        ("config_guard", &["fs.read"]),
        ("config_edit", &["fs.read", "fs.write"]),
        ("doc_parse", &["fs.read"]),
        ("extension_manager", &["llm"]),
        ("config_basic", &["fs.read"]),
        ("crypto", &["net"]),
        ("fs_basic", &["fs.read", "fs.write"]),
        ("fs_search", &["fs.read"]),
        ("http_basic", &["net"]),
        ("image_edit", &["fs.write", "llm", "net"]),
        ("list_dir", &["fs.read"]),
        ("log_analyze", &["fs.read"]),
        ("map_merchant", &["net"]),
        ("make_dir", &["fs.write"]),
        ("remove_file", &["fs.write"]),
        ("read_file", &["fs.read"]),
        // 注意 sort 顺序：`fs.write` < `llm` < `net` < `secrets.*`（按字典序）。
        // 新增 vendor 段时（openai/google/qwen/...），同步加 secrets.<usage>_<vendor>_api_key。
        (
            "image_generate",
            &[
                "fs.write",
                "llm",
                "net",
                "secrets.image_generation_minimax_api_key",
            ],
        ),
        ("image_vision", &["llm", "net"]),
        ("invest_copy", &["llm"]),
        ("kb", &["fs.read", "fs.write"]),
        ("rss_fetch", &["net"]),
        ("stock", &["llm", "net"]),
        ("task_control", &["net"]),
        ("weather", &["net"]),
        ("web_search_extract", &["net"]),
        ("write_file", &["fs.write"]),
    ];
    let docker_expected_with_caps: &[(&str, &[&str])] = &[
        // Docker 模板保持专用 image_edit / image_vision secret 声明。
        ("audio_synthesize", &["fs.write", "llm", "net"]),
        ("audio_transcribe", &["fs.read", "llm", "net"]),
        ("browser_web", &["fs.write", "net"]),
        ("config_guard", &["fs.read"]),
        ("config_edit", &["fs.read", "fs.write"]),
        ("doc_parse", &["fs.read"]),
        ("extension_manager", &["llm"]),
        ("config_basic", &["fs.read"]),
        ("crypto", &["net"]),
        ("fs_basic", &["fs.read", "fs.write"]),
        ("fs_search", &["fs.read"]),
        ("http_basic", &["net"]),
        (
            "image_edit",
            &[
                "fs.write",
                "llm",
                "net",
                "secrets.image_edit_minimax_api_key",
            ],
        ),
        (
            "image_generate",
            &[
                "fs.write",
                "llm",
                "net",
                "secrets.image_generation_minimax_api_key",
            ],
        ),
        (
            "image_vision",
            &["llm", "net", "secrets.image_vision_minimax_api_key"],
        ),
        ("invest_copy", &["llm"]),
        ("kb", &["fs.read", "fs.write"]),
        ("list_dir", &["fs.read"]),
        ("log_analyze", &["fs.read"]),
        ("map_merchant", &["net"]),
        ("make_dir", &["fs.write"]),
        ("read_file", &["fs.read"]),
        ("remove_file", &["fs.write"]),
        ("rss_fetch", &["net"]),
        ("stock", &["llm", "net"]),
        ("task_control", &["net"]),
        ("weather", &["net"]),
        ("web_search_extract", &["net"]),
        ("write_file", &["fs.write"]),
    ];

    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");
        let expected_with_caps = if path.to_string_lossy().contains("/docker/config/") {
            docker_expected_with_caps
        } else {
            main_expected_with_caps
        };

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

#[test]
fn registry_entries_have_group_and_tools_have_platform_metadata() {
    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");
        for name in registry.all_names() {
            let manifest = registry
                .manifest(&name)
                .unwrap_or_else(|| panic!("{}: missing manifest for `{name}`", path.display()));
            assert!(
                manifest.group.is_some(),
                "{}: skill `{name}` must declare registry group metadata",
                path.display()
            );
            if manifest.planner_kind == PlannerCapabilityKind::Tool {
                assert!(
                    !manifest.supported_os.is_empty(),
                    "{}: planner tool `{name}` must declare supported_os",
                    path.display()
                );
            }
        }
        let list_dir = registry
            .manifest("list_dir")
            .unwrap_or_else(|| panic!("{}: missing list_dir manifest", path.display()));
        assert_eq!(
            list_dir.runtime_skill.as_deref(),
            Some("system_basic"),
            "{}: list_dir runtime mapping should stay registry-driven",
            path.display()
        );
        assert_eq!(
            list_dir.runtime_action.as_deref(),
            Some("inventory_dir"),
            "{}: list_dir runtime action should stay registry-driven",
            path.display()
        );
        assert!(
            !list_dir.runtime_rewrite_arg_keys.is_empty()
                && !list_dir.runtime_rewrite_semantic_kinds.is_empty(),
            "{}: list_dir runtime mapping must declare structured rewrite triggers",
            path.display()
        );
    }
}

/// planner-first 收敛：已移除的技能不应再通过 registry 重新暴露。
#[test]
fn removed_skill_stubs_are_absent_from_registries() {
    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];
    let removed_skills = ["chat"];

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");
        for removed in removed_skills {
            assert!(
                registry.get(removed).is_none(),
                "{}: removed skill `{removed}` must not be reintroduced as a registry stub",
                path.display()
            );
        }
    }
}

/// §E1.c 守底：spawn 路径按 manifest 注入的 secrets env 必须等于期望集合。
///
/// 设计取舍：用一个**永远返回 Some(<placeholder>)** 的 mock broker —— 我们
/// 在这里要测的是"manifest → env 名翻译"的契约，而不是"CI 跑测试时机器上
/// 真有没有 IMAGE_GENERATION_MINIMAX_API_KEY"。后者属于运维/secrets 配置层。
///
/// 维护规则：
/// - manifest 给某个 skill 加 / 减 `secrets.<name>` → 同步改本测试 `expected_secrets_envs`；
/// - 没在表里的 skill 默认期望"零 secrets env 注入"；
/// - 永远不要把这里改成读真实 env，否则 CI 会变成"机器配置依赖测试"。
#[test]
fn provision_secret_envs_matches_manifest_expectation() {
    use std::collections::HashMap;

    /// 测试专用 broker：所有合法 secret 名都返回非空占位值；
    /// 非法名（broker 自己 validate_secret_name 拦下）会返回 Err，与生产
    /// 行为对齐 —— 这样测试既验"翻译契约"又验"命名规范运行期 enforcement"。
    struct AlwaysFoundBroker;
    impl SecretsBroker for AlwaysFoundBroker {
        fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
            claw_core::secrets::validate_secret_name(name)?;
            Ok(Some(SecretValue::new(format!("<test-{name}>"))))
        }
        fn label(&self) -> &str {
            "always-found-mock"
        }
    }

    // 期望：skill canonical name -> 子进程应当看到的 ENV_VAR_NAME 集合（已排序）。
    // 主配置中 image_edit / image_vision 可复用全局 provider key，因此不在
    // manifest 声明专用 secrets capability；Docker 模板仍声明专用 secret。
    let main_expected_secrets_envs: HashMap<&str, Vec<&str>> = HashMap::from([
        // §E1.c：image_generate 当前默认 default_vendor=minimax（见 configs/image.toml）。
        ("image_generate", vec!["IMAGE_GENERATION_MINIMAX_API_KEY"]),
    ]);
    let docker_expected_secrets_envs: HashMap<&str, Vec<&str>> = HashMap::from([
        ("image_generate", vec!["IMAGE_GENERATION_MINIMAX_API_KEY"]),
        // §E1.d：image_edit / image_vision 同样默认 default_vendor=minimax。
        // 新启用别的 vendor 段时，同步加对应 ENV_VAR_NAME。
        ("image_edit", vec!["IMAGE_EDIT_MINIMAX_API_KEY"]),
        ("image_vision", vec!["IMAGE_VISION_MINIMAX_API_KEY"]),
    ]);

    let registry_paths = [
        workspace_root().join("configs/skills_registry.toml"),
        workspace_root().join("docker/config/skills_registry.toml"),
    ];
    let broker = AlwaysFoundBroker;

    for path in registry_paths.iter() {
        let registry = SkillsRegistry::load_from_path(path).expect("load registry");
        let expected_secrets_envs = if path.to_string_lossy().contains("/docker/config/") {
            &docker_expected_secrets_envs
        } else {
            &main_expected_secrets_envs
        };
        for name in registry.all_names() {
            let caps = registry.capabilities(&name).to_vec();
            let provisioned = provision_secret_envs(&broker, &caps).unwrap_or_else(|err| {
                panic!(
                    "{}: skill `{name}` provisioning failed despite mock broker always returning Some \
                     — capability declaration likely violates secret-name rules: {err}",
                    path.display()
                );
            });
            let actual: Vec<&str> = provisioned.iter().map(|(n, _)| n.as_str()).collect();
            let expected_owned = expected_secrets_envs
                .get(name.as_str())
                .cloned()
                .unwrap_or_default();
            assert_eq!(
                actual,
                expected_owned,
                "{}: skill `{name}` provisioned {:?}, expected {:?}; \
                 update `expected_secrets_envs` in this test if intentional",
                path.display(),
                actual,
                expected_owned,
            );
        }
    }
}

/// §E1.c 配套：manifest 声明了 secrets.* 但 broker 找不到时，spawn 路径必须
/// 选择 fail-loud（不能 spawn 然后让 skill 拿空字符串去打 vendor）。
///
/// 这条测试不依赖具体 spawn 路径，只验 `provision_secret_envs` 在 broker 找
/// 不到声明的 secret 时返回 `MissingSecrets`，且包含完整的 missing 清单。
#[test]
fn provision_secret_envs_fails_loud_when_broker_lacks_declared_secret() {
    /// always-empty broker：任何合法 secret 名都返回 None（模拟生产环境忘
    /// 设 env 的情况）。
    struct AlwaysMissingBroker;
    impl SecretsBroker for AlwaysMissingBroker {
        fn lookup(&self, name: &str) -> Result<Option<SecretValue>, SecretsError> {
            claw_core::secrets::validate_secret_name(name)?;
            Ok(None)
        }
        fn label(&self) -> &str {
            "always-missing-mock"
        }
    }

    let path = workspace_root().join("configs/skills_registry.toml");
    let registry = SkillsRegistry::load_from_path(&path).expect("load registry");
    let caps = registry.capabilities("image_generate").to_vec();
    assert!(
        caps.iter().any(|c| matches!(c, Capability::Secrets(_))),
        "image_generate must declare at least one secrets.* capability after §E1.c"
    );

    let err = provision_secret_envs(&AlwaysMissingBroker, &caps)
        .expect_err("missing broker backing must surface as ProvisionError, not silent Ok");
    let msg = format!("{err}");
    assert!(
        msg.contains("image_generation_minimax_api_key"),
        "error must name the missing secret to help operators: {msg}"
    );
}
