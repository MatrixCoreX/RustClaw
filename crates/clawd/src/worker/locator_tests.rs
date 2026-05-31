use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = std::env::temp_dir();
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        path.push(format!(
            "clawd_worker_locator_{prefix}_{}_{}",
            std::process::id(),
            nanos
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
fn concrete_locator_hint_detects_explicit_path_and_filename_not_directory_scope_words() {
    assert!(super::has_concrete_locator_hint(
        "read /home/guagua/test/README.md and summarize"
    ));
    assert!(super::has_concrete_locator_hint(
        "read scripts/nl_tests/fixtures/device_local/package.json"
    ));
    assert!(super::has_concrete_locator_hint(
        "preview ./document before classifying images"
    ));
    assert!(super::has_concrete_locator_hint("send Cargo.toml"));
    assert!(super::has_concrete_locator_hint(
        "open https://example.com/file.txt"
    ));
    assert!(!super::has_concrete_locator_hint(
        "发 document 目录下最后一个"
    ));
    assert!(!super::has_concrete_locator_hint(
        "列出 logs 目录下前 5 个文件名"
    ));
}

#[test]
fn concrete_locator_hint_rejects_deictic_without_locator() {
    assert!(!super::has_concrete_locator_hint(
        "读一下那个开头并用一句话总结"
    ));
    assert!(!super::has_concrete_locator_hint("that config please"));
}

#[test]
fn concrete_locator_hint_rejects_dotted_version_numbers() {
    assert!(!super::has_concrete_locator_hint("3.11 3.10"));
}

#[test]
fn filename_like_tokens_extracts_expected_items() {
    let tokens =
        super::extract_filename_like_tokens("please open Config.toml and README.md plus README");
    assert!(tokens.iter().any(|v| v == "Config.toml"));
    assert!(tokens.iter().any(|v| v == "README.md"));
    assert!(!tokens.iter().any(|v| v == "README"));
}

#[test]
fn filename_like_tokens_split_adjacent_cjk_punctuation() {
    let tokens = super::extract_filename_like_tokens(
        "查一下 definitely_missing_rustclaw_case_file_98765.txt，找不到就告诉我",
    );
    assert!(tokens
        .iter()
        .any(|v| v == "definitely_missing_rustclaw_case_file_98765.txt"));
}

#[test]
fn implicit_locator_does_not_anchor_on_keyword_inside_missing_filename() {
    let workspace = TempDirGuard::new("missing_filename_keyword");
    fs::write(workspace.path.join("rustclaw"), "binary placeholder").expect("write rustclaw");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.path.clone();
    state.skill_rt.default_locator_search_dir = workspace.path.clone();

    let out = super::try_resolve_implicit_locator_path(
        &state,
        "查一下当前目录有没有 definitely_missing_rustclaw_case_file_98765.txt，找不到就直接告诉我。",
        "",
        crate::OutputLocatorKind::Path,
        None,
    );

    assert!(out.is_none(), "unexpected locator resolution: {out:?}");
}

#[test]
fn locator_keywords_split_mixed_script_tokens() {
    let out = super::extract_locator_keywords("请在 doc目录 里找 App_Config.toml");
    assert!(out.iter().any(|v| v == "doc"));
    assert!(out.iter().any(|v| v == "目录"));
    assert!(out.iter().any(|v| v == "app_config.toml"));
}

#[test]
fn resolve_explicit_relative_locator_path_from_text_prefers_workspace_root() {
    let root = TempDirGuard::new("relative_explicit_path");
    let nested = root.path.join("fixtures").join("device_local");
    fs::create_dir_all(&nested).expect("create nested dirs");
    let file = nested.join("package.json");
    fs::write(&file, "{}").expect("write fixture");
    let out = super::resolve_explicit_locator_path_from_text(
        &root.path,
        &root.path,
        "去 fixtures/device_local/package.json 里找 name",
    );
    assert_eq!(
        out.as_deref(),
        Some(
            file.canonicalize()
                .expect("canonical file")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn resolve_dot_relative_locator_path_from_text() {
    let root = TempDirGuard::new("dot_relative_explicit_path");
    let dir = root.path.join("document");
    fs::create_dir_all(&dir).expect("create document dir");
    let out = super::resolve_explicit_locator_path_from_text(
        &root.path,
        &root.path,
        "Preview ./document before classifying images.",
    );
    assert_eq!(
        out.as_deref(),
        Some(
            dir.canonicalize()
                .expect("canonical dir")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn resolve_explicit_relative_locator_path_from_text_supports_case_mismatch() {
    let root = TempDirGuard::new("relative_explicit_path_case_mismatch");
    let nested = root.path.join("Fixtures").join("Device_Local");
    fs::create_dir_all(&nested).expect("create nested dirs");
    let file = nested.join("Package.JSON");
    fs::write(&file, "{}").expect("write fixture");
    let out = super::resolve_explicit_locator_path_from_text(
        &root.path,
        &root.path,
        "去 fixtures/device_local/package.json 里找 name",
    );
    assert_eq!(
        out.as_deref(),
        Some(
            file.canonicalize()
                .expect("canonical file")
                .to_string_lossy()
                .as_ref()
        )
    );
}

#[test]
fn direct_child_locator_prefers_project_root_exact_name() {
    let root = TempDirGuard::new("direct_child");
    fs::create_dir_all(root.path.join("scripts")).expect("create scripts");
    fs::create_dir_all(root.path.join("nested/scripts")).expect("create nested/scripts");
    let keywords = vec![
        "查看一下".to_string(),
        "scripts".to_string(),
        "目录下面文件".to_string(),
    ];

    let out = super::try_resolve_direct_child_locator(&root.path, &keywords);
    let expected = root
        .path
        .join("scripts")
        .canonicalize()
        .expect("canonical scripts")
        .display()
        .to_string();
    assert_eq!(out, Some(expected));
}

#[test]
fn direct_child_locator_resolves_unique_file_stem() {
    let root = TempDirGuard::new("direct_child_stem");
    fs::write(root.path.join("README.md"), "# title\n").expect("write README");
    fs::create_dir_all(root.path.join("UI")).expect("create UI");
    fs::write(root.path.join("UI/README.md"), "# UI\n").expect("write nested README");
    let keywords = vec!["readme".to_string()];

    let out = super::try_resolve_direct_child_locator(&root.path, &keywords);
    let expected = root
        .path
        .join("README.md")
        .canonicalize()
        .expect("canonical readme")
        .display()
        .to_string();
    assert_eq!(out, Some(expected));
}

#[test]
fn ci_filename_match_supports_bare_readme_stem() {
    let root = TempDirGuard::new("readme_stem");
    fs::write(root.path.join("README.MD"), "# title\n").expect("write README.MD");
    let files = super::collect_files_for_locator_scan(&root.path, 0, 50);
    let out = super::collect_case_insensitive_filename_matches(&files, "README");
    assert_eq!(out.len(), 1);
    assert!(out[0].ends_with("README.MD"));
}

#[test]
fn ci_filename_match_returns_multiple_for_same_stem() {
    let root = TempDirGuard::new("readme_stem_multi");
    fs::write(root.path.join("README.MD"), "# title\n").expect("write README.MD");
    fs::write(root.path.join("README.txt"), "title\n").expect("write README.txt");
    let files = super::collect_files_for_locator_scan(&root.path, 0, 50);
    let out = super::collect_case_insensitive_filename_matches(&files, "README");
    assert_eq!(out.len(), 2);
}

#[test]
fn implicit_locator_search_roots_use_context_then_default_then_workspace() {
    let workspace = TempDirGuard::new("layered_roots_workspace");
    let default_root = workspace.path.join("fixtures");
    let logs = workspace.path.join("logs");
    fs::create_dir_all(&default_root).expect("create default root");
    fs::create_dir_all(&logs).expect("create logs");
    fs::write(logs.join("act_plan.log"), "ok\n").expect("write act plan");

    let roots = super::implicit_locator_search_roots(
        &workspace.path,
        &default_root,
        Some("刚才列的是 logs/act_plan.log"),
    );

    assert_eq!(roots.len(), 3, "unexpected roots: {roots:?}");
    assert_eq!(
        roots[0],
        logs.canonicalize().expect("canonical logs"),
        "context root should come first"
    );
    assert_eq!(
        roots[1],
        default_root.canonicalize().expect("canonical default root"),
        "default root should be second"
    );
    assert_eq!(
        roots[2],
        workspace.path.canonicalize().expect("canonical workspace"),
        "workspace root should remain as last fallback"
    );
    assert!(roots.iter().all(|root| root != Path::new("/")));
}

#[test]
fn implicit_locator_search_roots_skip_system_root_default() {
    let workspace = TempDirGuard::new("skip_system_root_default");
    let roots = super::implicit_locator_search_roots(&workspace.path, Path::new("/"), None);
    assert_eq!(roots.len(), 1, "unexpected roots: {roots:?}");
    assert_eq!(
        roots[0],
        workspace.path.canonicalize().expect("canonical workspace"),
    );
}

#[test]
fn implicit_locator_falls_back_to_workspace_root_when_default_root_misses() {
    let workspace = TempDirGuard::new("workspace_root_hit");
    let default_root = workspace.path.join("fixtures");
    fs::create_dir_all(&default_root).expect("create default root");
    let target = workspace.path.join("Cargo.toml");
    fs::write(&target, "[package]\nname='demo'\n").expect("write Cargo.toml");
    let roots = super::implicit_locator_search_roots(&workspace.path, &default_root, None);
    let out = super::try_resolve_implicit_locator_path_in_roots(
        &roots,
        &["cargo.toml".to_string()],
        &["Cargo.toml".to_string()],
        0,
        50,
    );
    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert!(path.ends_with("Cargo.toml"));
        }
        other => panic!("expected direct match, got {other:?}"),
    }
}

#[test]
fn relative_explicit_locator_prefers_recent_context_ancestor_over_workspace_root() {
    let workspace = TempDirGuard::new("relative_context_workspace");
    let context_root = workspace.path.join("fixtures").join("device");
    let config_dir = context_root.join("configs");
    fs::create_dir_all(context_root.join("data")).expect("create context data");
    fs::create_dir_all(&config_dir).expect("create config dir");
    fs::create_dir_all(workspace.path.join("data")).expect("create workspace data");
    fs::write(
        config_dir.join("app.toml"),
        "db_path = \"data/value.sqlite\"\n",
    )
    .expect("write config");
    fs::write(context_root.join("data/value.sqlite"), "context-db\n").expect("write context db");
    fs::write(workspace.path.join("data/value.sqlite"), "workspace-db\n")
        .expect("write workspace db");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.path.clone();
    state.skill_rt.default_locator_search_dir = workspace.path.clone();
    let context_hint = format!(
        "recent file={} output=db_path = \"data/value.sqlite\"",
        config_dir.join("app.toml").display()
    );

    let out = super::try_resolve_implicit_locator_path(
        &state,
        "data/value.sqlite",
        "查看配置里 data/value.sqlite 指向的库",
        crate::OutputLocatorKind::Path,
        Some(&context_hint),
    );

    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert_eq!(
                PathBuf::from(path)
                    .canonicalize()
                    .expect("canonical resolved path"),
                context_root
                    .join("data/value.sqlite")
                    .canonicalize()
                    .expect("canonical context db")
            );
        }
        other => panic!("expected context-relative sqlite path, got {other:?}"),
    }
}

#[test]
fn relative_explicit_file_token_scans_exact_suffix_before_parent_directory_fallback() {
    let workspace = TempDirGuard::new("relative_suffix_workspace");
    let context_root = workspace.path.join("fixtures").join("device");
    fs::create_dir_all(context_root.join("data")).expect("create context data");
    fs::create_dir_all(workspace.path.join("data")).expect("create workspace data dir");
    fs::write(context_root.join("data/value.sqlite"), "context-db\n").expect("write context db");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.path.clone();
    state.skill_rt.default_locator_search_dir = workspace.path.clone();
    state.skill_rt.locator_scan_max_depth = 2;
    state.skill_rt.locator_scan_max_files = 200;

    let out = super::try_resolve_implicit_locator_path(
        &state,
        "data/value.sqlite",
        "检查配置文件中指定的 data/value.sqlite 数据库中的表名列表",
        crate::OutputLocatorKind::Path,
        None,
    );

    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert_eq!(
                PathBuf::from(path)
                    .canonicalize()
                    .expect("canonical resolved path"),
                context_root
                    .join("data/value.sqlite")
                    .canonicalize()
                    .expect("canonical context db")
            );
        }
        other => panic!("expected suffix-resolved sqlite path, got {other:?}"),
    }
}

#[test]
fn unresolved_relative_explicit_file_token_does_not_fallback_to_parent_directory() {
    let workspace = TempDirGuard::new("relative_missing_file_workspace");
    fs::create_dir_all(workspace.path.join("data")).expect("create workspace data dir");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.path.clone();
    state.skill_rt.default_locator_search_dir = workspace.path.clone();
    state.skill_rt.locator_scan_max_depth = 4;
    state.skill_rt.locator_scan_max_files = 100;

    let out = super::try_resolve_implicit_locator_path(
        &state,
        "data/missing.sqlite",
        "检查配置文件中指定的 data/missing.sqlite 数据库",
        crate::OutputLocatorKind::Path,
        None,
    );

    assert!(
        out.is_none(),
        "relative missing file should not resolve to parent directory: {out:?}"
    );
}

#[test]
fn unresolved_absolute_explicit_path_does_not_fallback_to_keyword_match() {
    let workspace = TempDirGuard::new("absolute_missing_file_workspace");
    fs::write(workspace.path.join("rustclaw"), "unrelated").expect("write unrelated child");
    let missing = workspace.path.join("NO_SUCH_RUSTCLAW_TEST_987654.txt");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.path.clone();
    state.skill_rt.default_locator_search_dir = workspace.path.clone();
    state.skill_rt.locator_scan_max_depth = 4;
    state.skill_rt.locator_scan_max_files = 100;

    let out = super::try_resolve_implicit_locator_path(
        &state,
        &format!("读取 {} 的第一行", missing.display()),
        "",
        crate::OutputLocatorKind::Path,
        None,
    );

    assert!(
        out.is_none(),
        "absolute missing file should not resolve by path segment: {out:?}"
    );
}

#[test]
fn implicit_locator_prefers_unique_direct_child_match_over_nested_stem_matches() {
    let root = TempDirGuard::new("prefer_direct_child_stem");
    fs::write(root.path.join("README.md"), "# root\n").expect("write root README");
    fs::create_dir_all(root.path.join("UI")).expect("create UI");
    fs::write(root.path.join("UI/README.md"), "# ui\n").expect("write UI README");
    let roots = vec![root.path.clone()];
    let out = super::try_resolve_implicit_locator_path_in_roots(
        &roots,
        &["readme".to_string()],
        &["README".to_string()],
        2,
        100,
    );
    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert!(
                path.ends_with("/README.md"),
                "unexpected direct path: {path}"
            );
            assert!(
                !path.contains("/UI/README.md"),
                "unexpected nested match: {path}"
            );
        }
        other => panic!("expected direct root README match, got {other:?}"),
    }
}

#[test]
fn direct_child_filename_precheck_beats_context_root_for_unique_workspace_readme() {
    let workspace = TempDirGuard::new("direct_child_precheck_workspace");
    let fixture_dir = workspace.path.join("fixtures/device_local");
    fs::create_dir_all(&fixture_dir).expect("create fixture dir");
    fs::write(workspace.path.join("README.md"), "# root\n").expect("write root README");
    fs::write(fixture_dir.join("README.md"), "# fixture\n").expect("write fixture README");
    let mut state = crate::AppState::test_default_with_fixture_provider();
    state.skill_rt.workspace_root = workspace.path.clone();
    state.skill_rt.default_locator_search_dir = fixture_dir.clone();
    let out = super::try_resolve_implicit_locator_path(
        &state,
        "README.md",
        "README.md",
        crate::OutputLocatorKind::Filename,
        Some(fixture_dir.to_string_lossy().as_ref()),
    );
    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert!(
                path.ends_with("/README.md"),
                "unexpected direct path: {path}"
            );
            assert!(
                !path.contains("/fixtures/device_local/README.md"),
                "context root unexpectedly won: {path}"
            );
        }
        other => panic!("expected direct workspace README match, got {other:?}"),
    }
}

#[test]
fn implicit_locator_does_not_promote_keyword_fragment_over_missing_filename() {
    let root = TempDirGuard::new("missing_filename_keyword_fragment");
    fs::write(root.path.join("rustclaw"), "binary").expect("write rustclaw");
    let roots = vec![root.path.clone()];
    let out = super::try_resolve_implicit_locator_path_in_roots(
        &roots,
        &[
            "把".to_string(),
            "definitely_missing_named_file_rustclaw_001.txt".to_string(),
            "发给我".to_string(),
        ],
        &["definitely_missing_named_file_rustclaw_001.txt".to_string()],
        0,
        50,
    );
    assert!(out.is_none(), "unexpected locator resolution: {out:?}");
}

#[test]
fn explicit_workspace_child_does_not_bind_filename_fragment_to_existing_file() {
    let root = TempDirGuard::new("explicit_workspace_child_missing_filename_fragment");
    fs::write(root.path.join("rustclaw"), "binary").expect("write rustclaw");
    let query = "把 definitely_missing_named_file_rustclaw_001.txt 发给我";
    let keywords = super::extract_locator_keywords(query);
    let filename_tokens = super::extract_filename_like_tokens(query);
    let out = super::resolve_explicit_workspace_child_target(
        &root.path,
        &root.path,
        &keywords,
        &filename_tokens,
    );
    assert!(
        out.is_none(),
        "unexpected explicit child resolution: {out:?}"
    );
}

#[test]
fn current_workspace_locator_binds_to_workspace_root() {
    let root = TempDirGuard::new("workspace_scope");
    let out = super::resolve_workspace_root_path(&root.path);
    assert_eq!(
        Some(out),
        Some(
            root.path
                .canonicalize()
                .expect("canonical root")
                .display()
                .to_string()
        )
    );
}

#[test]
fn current_workspace_locator_prefers_direct_child_target_when_present() {
    let root = TempDirGuard::new("workspace_direct_child");
    let logs = root.path.join("logs");
    fs::create_dir_all(&logs).expect("create logs");
    fs::write(logs.join("act_plan.log"), "ok\n").expect("write act_plan");

    let out =
        super::resolve_current_workspace_target(&root.path, &["logs".to_string()], &[], 2, 100);

    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert_eq!(
                path,
                logs.canonicalize()
                    .expect("canonical logs")
                    .display()
                    .to_string()
            );
        }
        other => panic!("expected direct logs path, got {other:?}"),
    }
}

#[test]
fn explicit_workspace_child_directory_beats_context_root() {
    let root = TempDirGuard::new("explicit_child_beats_context");
    let workspace_logs = root.path.join("logs");
    let fixture_logs = root.path.join("fixtures").join("logs");
    fs::create_dir_all(&workspace_logs).expect("create workspace logs");
    fs::create_dir_all(&fixture_logs).expect("create fixture logs");
    fs::write(workspace_logs.join("clawd.log"), "ok\n").expect("write workspace log");
    fs::write(fixture_logs.join("app.log"), "ok\n").expect("write fixture log");

    let out = super::resolve_explicit_workspace_child_target(
        &root.path,
        &root.path,
        &["logs".to_string()],
        &[],
    );

    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert_eq!(
                path,
                workspace_logs
                    .canonicalize()
                    .expect("canonical workspace logs")
                    .display()
                    .to_string()
            );
        }
        other => panic!("expected workspace logs path, got {other:?}"),
    }
}

#[test]
fn contextual_locator_root_prefers_recent_directory_scope() {
    let root = TempDirGuard::new("context_root");
    let logs = root.path.join("logs");
    fs::create_dir_all(&logs).expect("create logs");
    fs::write(logs.join("act_plan.log"), "ok\n").expect("write act_plan");

    let out = super::resolve_contextual_locator_root(
        &root.path,
        &root.path,
        Some("### RECENT_EXECUTION_EVENTS\n- request=先列出 logs 目录下前 5 个文件名 result=act_plan.log"),
    );

    assert_eq!(
        out.map(|v| v.canonicalize().unwrap_or(v).display().to_string()),
        Some(
            logs.canonicalize()
                .expect("canonical logs")
                .display()
                .to_string()
        )
    );
}

#[test]
fn implicit_locator_uses_contextual_directory_root_for_followup_file() {
    let root = TempDirGuard::new("contextual_followup");
    let logs = root.path.join("logs");
    fs::create_dir_all(&logs).expect("create logs");
    fs::write(logs.join("act_plan.log"), "line1\nline2\n").expect("write act_plan");
    fs::write(root.path.join("README.md"), "hello").expect("write readme");

    let roots = vec![logs.clone(), root.path.clone()];
    let out = super::try_resolve_implicit_locator_path_in_roots(
        &roots,
        &["读取".to_string(), "act_plan.log".to_string()],
        &["act_plan.log".to_string()],
        3,
        128,
    );

    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert_eq!(
                PathBuf::from(path)
                    .canonicalize()
                    .expect("canonical resolved follow-up"),
                logs.join("act_plan.log")
                    .canonicalize()
                    .expect("canonical act_plan")
            );
        }
        other => panic!("expected direct follow-up file resolution, got {other:?}"),
    }
}

#[test]
fn implicit_filename_locator_finds_nested_target_before_deep_noise_exhausts_budget() {
    let root = TempDirGuard::new("filename_nested_target");
    let alpha = root.path.join("alpha");
    let scripts = root.path.join("scripts");
    fs::create_dir_all(&alpha).expect("create alpha");
    fs::create_dir_all(&scripts).expect("create scripts");
    for idx in 0..8 {
        fs::write(alpha.join(format!("noise_{idx}.txt")), "x\n").expect("write noise");
    }
    let target = scripts.join("restart_clawd_latest.sh");
    fs::write(&target, "#!/bin/sh\necho ok\n").expect("write target");

    let out = super::try_resolve_implicit_locator_path_in_roots(
        &[root.path.clone()],
        &["restart_clawd_latest.sh".to_string()],
        &["restart_clawd_latest.sh".to_string()],
        2,
        10,
    );

    match out {
        Some(super::LocatorAutoResolution::Direct(path)) => {
            assert_eq!(
                PathBuf::from(path)
                    .canonicalize()
                    .expect("canonical resolved target")
                    .display()
                    .to_string(),
                target
                    .canonicalize()
                    .expect("canonical target")
                    .display()
                    .to_string()
            );
        }
        other => panic!("expected direct nested filename match, got {other:?}"),
    }
}
