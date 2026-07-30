use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tempfile::tempdir;

use crate::process::{run_command_controlled, ProcessOutput};
use crate::{
    prepare_sandboxed_command, validate_response_line, BuildAdapter, HostPlatform,
    InstallReceiptStore, InstallRequest, ProtocolRequest, SandboxNetwork, SkillInstaller,
    SkillLaunchSpec, SkillRuntimeResolver,
};

const ADAPTERS: [(&str, BuildAdapter, &[&str]); 6] = [
    ("rust", BuildAdapter::Cargo, &["cargo"]),
    ("python", BuildAdapter::Python, &["python3"]),
    ("node", BuildAdapter::Node, &["node", "npm"]),
    ("go", BuildAdapter::Go, &["go"]),
    ("prebuilt", BuildAdapter::Prebuilt, &["python3"]),
    (
        "generic_process",
        BuildAdapter::GenericProcess,
        &["python3"],
    ),
];

#[test]
fn reference_languages_share_protocol_and_atomic_lifecycle() {
    let require_all = std::env::var_os("APP_REQUIRE_REFERENCE_ADAPTERS").is_some();
    if !sandbox_backend_present() {
        assert!(
            !require_all,
            "required sandbox backend is unavailable for reference conformance"
        );
        eprintln!("REFERENCE_CONFORMANCE skipped: required sandbox backend unavailable");
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("reference");
    copy_tree(&fixture_root(), &workspace).expect("copy fixtures");
    render_prebuilt_manifest(&workspace);
    let package_root = temp.path().join("packages");
    let output_root = temp.path().join("artifacts");
    fs::create_dir_all(&output_root).expect("artifact root");
    let mut exercised = BTreeSet::new();

    for (language, adapter, tools) in ADAPTERS {
        if tools.iter().any(|tool| find_on_path(tool).is_none()) {
            eprintln!("REFERENCE_CONFORMANCE adapter={language} skipped_missing_toolchain");
            continue;
        }
        let manifest_path = workspace.join(language).join("skill.toml");
        prove_network_approval_gate(&manifest_path, &workspace, &package_root);
        let first = install(&manifest_path, &workspace, &package_root).expect("install v1");
        assert_eq!(first.adapter, adapter);
        assert!(!adapter_source_was_mutated(&workspace.join(language)));
        let launch = SkillRuntimeResolver::new(&package_root)
            .resolve(&first.skill_name)
            .expect("resolve v1");
        prove_action_contract(language, &launch, &output_root);

        replace_once(&manifest_path, "version = \"1.0.0\"", "version = \"1.1.0\"");
        let second = install(&manifest_path, &workspace, &package_root).expect("install update");
        assert_ne!(first.receipt_digest, second.receipt_digest);
        let store = InstallReceiptStore::new(&package_root);
        let rolled_back = store.rollback(&first.skill_name).expect("rollback update");
        assert_eq!(rolled_back.receipt_digest, first.receipt_digest);

        replace_once(
            &manifest_path,
            "working_directory = \".\"",
            "working_directory = \"../escape\"",
        );
        assert!(install(&manifest_path, &workspace, &package_root).is_err());
        assert_eq!(
            store
                .current_pointer(&first.skill_name)
                .expect("current preserved")
                .receipt_digest,
            first.receipt_digest
        );

        replace_once(
            &manifest_path,
            "working_directory = \"../escape\"",
            "working_directory = \".\"",
        );
        let shared_toolchain = temp.path().join("shared-toolchains").join(language);
        fs::create_dir_all(&shared_toolchain).expect("shared toolchain fixture");
        fs::write(shared_toolchain.join("sentinel"), b"keep").expect("shared toolchain sentinel");
        store
            .remove_installed_versions(&first.skill_name)
            .expect("uninstall adapter fixture");
        assert!(
            SkillRuntimeResolver::new(&package_root)
                .resolve(&first.skill_name)
                .is_err(),
            "uninstalled {language} package must no longer resolve"
        );
        assert!(
            shared_toolchain.join("sentinel").is_file(),
            "uninstalling {language} must preserve shared toolchains"
        );
        let reinstalled =
            install(&manifest_path, &workspace, &package_root).expect("reinstall adapter fixture");
        let relaunched = SkillRuntimeResolver::new(&package_root)
            .resolve(&reinstalled.skill_name)
            .expect("resolve reinstalled adapter fixture");
        prove_action_contract(language, &relaunched, &output_root);
        exercised.insert(language);
        eprintln!("REFERENCE_CONFORMANCE adapter={language} ok");
    }

    let required = if require_all {
        &[
            "rust",
            "python",
            "node",
            "go",
            "prebuilt",
            "generic_process",
        ][..]
    } else {
        &["rust", "python", "node", "prebuilt", "generic_process"][..]
    };
    for required in required {
        assert!(
            exercised.contains(required),
            "required local adapter {required} was not exercised"
        );
    }
}

fn prove_action_contract(language: &str, launch: &SkillLaunchSpec, output_root: &Path) {
    let calculated = run_valid_action(
        launch,
        json!({"action":"calculate","a":7,"b":5}),
        output_root,
    );
    assert_eq!(calculated.extra.as_ref().unwrap()["result"]["value"], 12);

    let validation = run_valid_action(launch, json!({"action":"validation_error"}), output_root);
    assert_eq!(
        validation.extra.as_ref().unwrap()["error_code"],
        "fixture_invalid"
    );

    let artifact = output_root.join(format!("{language}.txt"));
    let artifact_response = run_valid_action(
        launch,
        json!({"action":"artifact","artifact_path":artifact}),
        output_root,
    );
    assert_eq!(
        artifact_response.extra.as_ref().unwrap()["artifact"]["created"],
        true
    );
    assert_eq!(
        fs::read_to_string(&artifact).expect("artifact"),
        "reference-artifact\n"
    );

    let waiting = run_valid_action(launch, json!({"action":"waiting"}), output_root);
    assert_eq!(
        waiting.extra.as_ref().unwrap()["continuation"]["state"],
        "waiting"
    );
    let needs_user = run_valid_action(launch, json!({"action":"needs_user"}), output_root);
    assert_eq!(
        needs_user.extra.as_ref().unwrap()["continuation"]["state"],
        "needs_user"
    );

    let stderr = run_raw_action(
        launch,
        json!({"action":"stderr"}),
        output_root,
        Duration::from_secs(2),
    )
    .expect("stderr action");
    assert!(!stderr.stderr.is_empty());
    validate_response_line(&stderr.stdout, "reference-request").expect("stderr keeps stdout pure");

    let malformed = run_raw_action(
        launch,
        json!({"action":"malformed"}),
        output_root,
        Duration::from_secs(2),
    )
    .expect("malformed process");
    assert_eq!(
        validate_response_line(&malformed.stdout, "reference-request")
            .expect_err("malformed rejected")
            .code,
        "protocol_response_invalid"
    );
    let multiple = run_raw_action(
        launch,
        json!({"action":"multiple"}),
        output_root,
        Duration::from_secs(2),
    )
    .expect("multiple process");
    assert_eq!(
        validate_response_line(&multiple.stdout, "reference-request")
            .expect_err("multiple rejected")
            .code,
        "protocol_multiple_stdout_records"
    );
    let oversized = run_raw_action(
        launch,
        json!({"action":"oversized"}),
        output_root,
        Duration::from_secs(3),
    )
    .expect_err("oversized output rejected");
    assert_eq!(oversized.code, "process_output_oversized");
    let timeout = run_raw_action(
        launch,
        json!({"action":"timeout"}),
        output_root,
        Duration::from_millis(150),
    )
    .expect_err("timeout enforced");
    assert_eq!(timeout.code, "process_timeout");
}

fn run_valid_action(
    launch: &SkillLaunchSpec,
    args: Value,
    output_root: &Path,
) -> crate::ProtocolResponse {
    let output = run_raw_action(launch, args, output_root, Duration::from_secs(2))
        .expect("reference action");
    assert!(output.status.success());
    validate_response_line(&output.stdout, "reference-request").expect("valid response")
}

fn run_raw_action(
    launch: &SkillLaunchSpec,
    args: Value,
    output_root: &Path,
    timeout: Duration,
) -> Result<ProcessOutput, crate::SkillSdkError> {
    let mut prepared = prepare_sandboxed_command(
        &launch.program,
        &launch.working_directory,
        &[output_root.to_path_buf()],
        SandboxNetwork::Deny,
    )?;
    prepared.command.args(&launch.args);
    prepared.command.env_clear();
    prepared.command.env("PATH", "/usr/local/bin:/usr/bin:/bin");
    prepared.command.envs(&launch.environment);
    let request = ProtocolRequest {
        request_id: "reference-request".to_string(),
        args,
        context: Some(json!({"reference_conformance":true})),
        user_id: 0,
        chat_id: 0,
        user_key: None,
    };
    let mut input = request.to_line()?.into_bytes();
    input.push(b'\n');
    run_command_controlled(
        &mut prepared.command,
        Some(&input),
        timeout,
        "reference",
        None,
    )
}

fn prove_network_approval_gate(manifest_path: &Path, workspace: &Path, package_root: &Path) {
    let original = fs::read_to_string(manifest_path).expect("manifest");
    fs::write(
        manifest_path,
        original.replacen("network = \"deny\"", "network = \"approval_required\"", 1),
    )
    .expect("approval manifest");
    let error = install(manifest_path, workspace, package_root).expect_err("approval required");
    assert_eq!(error.code, "build_network_approval_required");
    fs::write(manifest_path, original).expect("restore manifest");
}

fn install(
    manifest_path: &Path,
    workspace: &Path,
    package_root: &Path,
) -> Result<crate::InstallOutcome, crate::SkillSdkError> {
    SkillInstaller.install(&InstallRequest {
        manifest_path: manifest_path.to_path_buf(),
        workspace_root: workspace.to_path_buf(),
        package_root: package_root.to_path_buf(),
        target: None,
        allow_network: false,
        control: None,
    })
}

fn render_prebuilt_manifest(workspace: &Path) {
    let root = workspace.join("prebuilt");
    let script = fs::read(root.join("skill.py")).expect("prebuilt script");
    let platform = HostPlatform::current();
    let manifest = fs::read_to_string(root.join("skill.toml.in"))
        .expect("manifest template")
        .replace("@OS@", &platform.os)
        .replace("@ARCH@", &platform.arch)
        .replace("@SHA256@", &hex::encode(Sha256::digest(script)));
    fs::write(root.join("skill.toml"), manifest).expect("prebuilt manifest");
}

fn adapter_source_was_mutated(root: &Path) -> bool {
    ["target", ".venv", "venv", "node_modules", ".gocache"]
        .iter()
        .any(|name| root.join(name).exists())
}

fn replace_once(path: &Path, before: &str, after: &str) {
    let raw = fs::read_to_string(path).expect("read replace target");
    assert!(raw.contains(before), "replacement source missing: {before}");
    fs::write(path, raw.replacen(before, after, 1)).expect("write replacement");
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/reference")
}

fn copy_tree(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|value| std::env::split_paths(&value).collect::<Vec<_>>())
        .map(|root| root.join(name))
        .find(|candidate| candidate.is_file())
}

fn sandbox_backend_present() -> bool {
    #[cfg(target_os = "linux")]
    return Path::new("/usr/bin/bwrap").is_file() || Path::new("/bin/bwrap").is_file();
    #[cfg(target_os = "macos")]
    return Path::new("/usr/bin/sandbox-exec").is_file();
    #[allow(unreachable_code)]
    false
}
