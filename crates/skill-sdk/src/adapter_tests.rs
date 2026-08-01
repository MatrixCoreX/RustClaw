use sha2::{Digest, Sha256};

use super::*;

fn artifact(os: &str, arch: &str, contents: &[u8]) -> PlatformArtifact {
    PlatformArtifact {
        os: os.to_string(),
        arch: arch.to_string(),
        sha256: hex::encode(Sha256::digest(contents)),
        source_path: Some("fixture.bin".to_string()),
        url: None,
        executable: true,
        size_bytes: Some(contents.len() as u64),
        archive: None,
    }
}

#[test]
fn prebuilt_selection_requires_an_exact_normalized_platform() {
    let platform = HostPlatform {
        os: "linux".to_string(),
        arch: "aarch64".to_string(),
        target: Some("aarch64-unknown-linux-gnu".to_string()),
    };
    let artifacts = vec![
        artifact("linux", "x86_64", b"x86"),
        artifact("darwin", "arm64", b"mac"),
        artifact("linux", "arm64", b"arm"),
    ];
    let selected = select_prebuilt_artifact(&artifacts, &platform).expect("exact artifact");
    assert_eq!(selected.sha256, hex::encode(Sha256::digest(b"arm")));
    assert_eq!(
        select_prebuilt_artifact(
            &artifacts,
            &HostPlatform {
                os: "linux".to_string(),
                arch: "armv7".to_string(),
                target: Some("armv7-unknown-linux-gnueabihf".to_string()),
            },
        )
        .expect_err("no approximate artifact fallback")
        .code,
        "prebuilt_platform_artifact_missing"
    );
}

#[test]
fn missing_and_unsafe_toolchain_names_are_structured_preflight_errors() {
    assert_eq!(
        find_program("definitely-not-a-agent-runtime-toolchain-7f34")
            .expect_err("missing toolchain")
            .code,
        "toolchain_missing"
    );
    assert_eq!(
        find_program("../bin/python")
            .expect_err("path override rejected")
            .code,
        "toolchain_name_invalid"
    );
}

#[test]
fn cargo_jobs_use_explicit_positive_override_or_detected_parallelism() {
    assert_eq!(cargo_jobs_for(Some("4"), 8), "4");
    assert_eq!(cargo_jobs_for(None, 8), "8");
    assert_eq!(cargo_jobs_for(Some("0"), 8), "8");
    assert_eq!(cargo_jobs_for(Some("invalid"), 8), "8");
    assert_eq!(cargo_jobs_for(None, 0), "1");
}

#[test]
fn cargo_cache_seed_tolerates_platform_specific_locked_package_gaps() {
    let temp = tempfile::tempdir().expect("tempdir");
    let source_home = temp.path().join("source");
    let private_home = temp.path().join("private");
    let source_index = source_home.join("registry/index/index-fixture");
    let source_cache = source_home.join("registry/cache/cache-fixture");
    let available_name = "available_fixture";
    let available_version = "1.2.3";
    let index_relative = crates_io_index_relative(available_name);

    fs::create_dir_all(
        source_index
            .join(".cache")
            .join(&index_relative)
            .parent()
            .expect("index parent"),
    )
    .expect("source index");
    fs::create_dir_all(&source_cache).expect("source cache");
    fs::write(source_index.join("config.json"), "{}").expect("index config");
    fs::write(
        source_index.join(".cache").join(&index_relative),
        b"index entry",
    )
    .expect("index entry");
    fs::write(
        source_cache.join(format!("{available_name}-{available_version}.crate")),
        b"crate archive",
    )
    .expect("crate archive");

    seed_locked_cargo_packages(
        &private_home,
        &source_home,
        &[
            (available_name.to_string(), available_version.to_string()),
            ("target_only_missing".to_string(), "9.9.9".to_string()),
        ],
    )
    .expect("available packages should be seeded without requiring target-only archives");

    assert!(private_home
        .join("registry/index/index-fixture/.cache")
        .join(index_relative)
        .is_file());
    assert!(private_home
        .join("registry/cache/cache-fixture")
        .join(format!("{available_name}-{available_version}.crate"))
        .is_file());
    assert!(!private_home
        .join("registry/cache/cache-fixture/target_only_missing-9.9.9.crate")
        .exists());
}

#[test]
fn toolchain_override_requires_an_absolute_executable() {
    let temp = tempfile::tempdir().expect("tempdir");
    let executable = temp.path().join("python3");
    fs::write(&executable, b"#!/bin/sh\nexit 0\n").expect("fixture executable");
    set_executable(&executable).expect("executable permission");

    assert_eq!(
        validate_program_override("APP_PYTHON_BIN", &executable).expect("valid override"),
        fs::canonicalize(&executable).expect("canonical fixture"),
    );
    assert_eq!(
        validate_program_override("APP_PYTHON_BIN", Path::new("python3"))
            .expect_err("relative overrides must fail")
            .code,
        "toolchain_override_invalid",
    );
    assert_eq!(
        validate_program_override("APP_PYTHON_BIN", &temp.path().join("missing"))
            .expect_err("missing overrides must fail")
            .code,
        "toolchain_override_invalid",
    );
}

#[test]
fn python_requirement_is_enforced_before_environment_creation() {
    assert!(validate_python_requirement(Path::new("/usr/bin/python3"), None).is_ok());
    assert_eq!(
        parse_minimum_runtime_version(">=3.13", "build.options.python").expect("minimum version"),
        [3, 13, 0]
    );
    assert_eq!(
        parse_minimum_runtime_version("3.13", "build.options.python")
            .expect_err("operator is required")
            .code,
        "manifest_adapter_option_invalid"
    );
}

#[cfg(unix)]
#[test]
fn redundant_python_lib64_alias_is_removed_without_touching_other_links() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().expect("tempdir");
    let venv = temp.path().join("venv");
    let lib = venv.join("lib");
    let other = venv.join("other");
    fs::create_dir_all(&lib).expect("lib");
    fs::create_dir_all(&other).expect("other");
    symlink("lib", venv.join("lib64")).expect("lib64 alias");
    symlink("other", venv.join("other-alias")).expect("unrelated alias");

    remove_redundant_python_venv_aliases(&venv).expect("normalize venv aliases");

    assert!(!venv.join("lib64").exists());
    assert!(fs::symlink_metadata(venv.join("other-alias"))
        .expect("unrelated alias")
        .file_type()
        .is_symlink());
}

#[test]
fn python_bytecode_caches_are_removed_without_touching_sources() {
    let temp = tempfile::tempdir().expect("tempdir");
    let runtime = temp.path().join("runtime");
    let cache = runtime.join("package/__pycache__");
    fs::create_dir_all(&cache).expect("cache");
    fs::write(runtime.join("module.py"), b"value = 1\n").expect("source");
    fs::write(cache.join("module.cpython-313.pyc"), b"cache").expect("cache file");
    fs::write(runtime.join("legacy.pyo"), b"legacy cache").expect("legacy cache");

    remove_python_bytecode_caches(&runtime).expect("remove bytecode caches");

    assert!(runtime.join("module.py").is_file());
    assert!(!cache.exists());
    assert!(!runtime.join("legacy.pyo").exists());
}

#[cfg(unix)]
#[test]
fn python_console_scripts_remain_executable_after_staging_commit() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let python = find_program("python3").expect("python3");
    let temp = tempfile::tempdir().expect("tempdir");
    let staging = temp.path().join("staging");
    let bin = staging.join("runtime/venv/bin");
    fs::create_dir_all(&bin).expect("venv bin");
    symlink(python, bin.join("python")).expect("venv python");
    let script = bin.join("fixture-command");
    fs::write(
        &script,
        format!("#!{}/python\nprint('relocated-ok')\n", bin.display()),
    )
    .expect("console script");
    let mut permissions = fs::metadata(&script).expect("metadata").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("executable");
    let pip_wrapper = bin.join("pip-wrapper-command");
    fs::write(
        &pip_wrapper,
        format!(
            "#!/bin/sh\n'''exec' {}/python \"$0\" \"$@\"\n' '''\nprint('pip-wrapper-relocated-ok')\n",
            bin.display()
        ),
    )
    .expect("pip console wrapper");
    let mut permissions = fs::metadata(&pip_wrapper)
        .expect("pip wrapper metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&pip_wrapper, permissions).expect("pip wrapper executable");

    assert_eq!(
        rewrite_python_console_scripts_for_relocation(&staging.join("runtime/venv"))
            .expect("rewrite console scripts"),
        2
    );
    let installed = temp.path().join("versions/fixture");
    fs::create_dir_all(installed.parent().expect("versions parent")).expect("versions");
    fs::rename(&staging, &installed).expect("commit staging directory");

    let output = Command::new(installed.join("runtime/venv/bin/fixture-command"))
        .output()
        .expect("run relocated console script");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "relocated-ok"
    );
    let output = Command::new(installed.join("runtime/venv/bin/pip-wrapper-command"))
        .output()
        .expect("run relocated pip console wrapper");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "pip-wrapper-relocated-ok"
    );
}

#[test]
fn http_json_adapter_prepares_a_pinned_https_launch_without_local_toolchains() {
    let source = crate::tests::manifest_source()
        .replace("adapter = \"cargo\"", "adapter = \"http_json\"")
        .replace("package = \"sample-weather-skill\"\n", "")
        .replace("binary = \"sample-weather-skill\"\n", "")
        .replace("lockfile = \"Cargo.lock\"\n", "")
        .replace("network = \"deny\"", "network = \"approval_required\"")
        .replace(
            "network = \"approval_required\"\n\n[run]",
            "network = \"approval_required\"\noptions = { endpoint = \"https://api.example.invalid/v1/skill\" }\n\n[run]",
        )
        .replace("launcher = \"native\"", "launcher = \"http_json\"")
        .replace(
            "entrypoint = \"runtime/bin/sample-weather-skill\"",
            "entrypoint = \"runtime/http-json-adapter\"",
        )
        .replace("runtime_network = false", "runtime_network = true");
    let manifest = PackageManifest::from_toml_str(&source).expect("valid HTTP manifest");
    let temp = tempfile::tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let staging = temp.path().join("staging");
    let cache = temp.path().join("cache");
    fs::create_dir_all(&workspace).expect("workspace");
    let platform = HostPlatform::current();

    let prepared = prepare_package(&AdapterContext {
        manifest: &manifest,
        workspace_root: &workspace,
        manifest_dir: &workspace,
        staging_root: &staging,
        cache_root: &cache,
        platform: &platform,
        target: None,
        allow_network: true,
        control: None,
    })
    .expect("prepare HTTP adapter");

    assert_eq!(prepared.adapter_version, "http-json-v1");
    assert_eq!(prepared.launch.launcher, LauncherKind::HttpJson);
    assert_eq!(
        prepared.launch.remote_endpoint.as_deref(),
        Some("https://api.example.invalid/v1/skill")
    );
    assert_eq!(prepared.artifacts.len(), 1);
    assert!(staging.join("runtime/http-json-adapter").is_file());
}
