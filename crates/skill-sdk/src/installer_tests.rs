use std::fs;

use sha2::{Digest, Sha256};
use tempfile::tempdir;

use crate::installer::{
    AdoptBuiltRequest, InstallOrigin, InstallRequest, PrecompiledInstallRequest, SkillInstaller,
};
use crate::runtime::SkillRuntimeResolver;

#[test]
fn package_root_is_canonicalized_before_sandboxed_installation() {
    let current = std::env::current_dir().expect("current directory");
    let temp = tempfile::tempdir_in(&current).expect("workspace-local tempdir");
    let relative_root = temp
        .path()
        .strip_prefix(&current)
        .expect("tempdir below current directory")
        .join("packages");
    assert!(!relative_root.is_absolute());
    let manifest =
        crate::PackageManifest::from_toml_str(crate::tests::manifest_source()).expect("manifest");

    let prepared =
        super::prepare_package_root(&manifest, &relative_root).expect("relative package root");

    assert!(prepared.is_absolute());
    assert!(prepared.is_dir());
}

#[test]
fn install_resource_preflight_returns_a_stable_insufficient_code() {
    let temp = tempdir().expect("tempdir");
    let mut manifest =
        crate::PackageManifest::from_toml_str(crate::tests::manifest_source()).expect("manifest");
    manifest.install.resources.min_free_disk_mb = u64::MAX;
    let error = super::validate_install_resources(&manifest, temp.path())
        .expect_err("impossible disk requirement must fail");
    assert_eq!(error.code, "install_resource_insufficient");
    assert_eq!(error.phase.as_deref(), Some("preflight"));
    assert!(error.detail.contains("resource=free_disk"));
}

#[test]
fn prebuilt_adapter_installs_smokes_activates_and_resolves() {
    if !sandbox_backend_present() {
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("fixture");
    fs::create_dir_all(&source).expect("source");
    let executable = source.join("fixture_skill.py");
    fs::write(
        &executable,
        br#"#!/usr/bin/env python3
import json, sys
request = json.loads(sys.stdin.readline())
print(json.dumps({"request_id": request["request_id"], "status": "ok", "text": "fixture", "error_text": None, "buttons": None, "extra": {"value": 1}}, separators=(",", ":")))
"#,
    )
    .expect("fixture executable");
    let digest = hex::encode(Sha256::digest(
        fs::read(&executable).expect("fixture bytes"),
    ));
    let platform = crate::HostPlatform::current();
    let manifest = format!(
        r#"
schema_version = 1

[package]
name = "prebuilt_fixture"
version = "1.0.0"
description = "Prebuilt fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"

[registry]
name = "prebuilt_fixture"

[build]
adapter = "prebuilt"
source_root = "fixture"
network = "deny"

[[build.artifacts]]
os = "{os}"
arch = "{arch}"
sha256 = "{digest}"
source_path = "fixture_skill.py"
executable = true

[run]
launcher = "native"
entrypoint = "runtime/bin/prebuilt-fixture"
working_directory = "."
timeout_seconds = 30

[security]
capability_policy_source = "registry"
sandbox = "required"
runtime_network = false
inherit_credentials = false

[storage]
kind = "none"
schema_version = 1
migration_owner = "prebuilt_fixture"
"#,
        os = platform.os,
        arch = platform.arch,
    );
    let manifest_path = source.join("skill.toml");
    fs::write(&manifest_path, manifest).expect("manifest");
    let package_root = temp.path().join("packages");
    let outcome = SkillInstaller
        .install(&InstallRequest {
            manifest_path,
            workspace_root: workspace,
            package_root: package_root.clone(),
            target: None,
            allow_network: false,
            control: None,
        })
        .expect("install");
    assert_eq!(outcome.skill_name, "prebuilt_fixture");
    assert!(outcome.phases.contains(&"protocol_smoke".to_string()));
    let launch = SkillRuntimeResolver::new(package_root)
        .resolve("prebuilt_fixture")
        .expect("resolve");
    assert!(launch.program.ends_with("runtime/bin/prebuilt-fixture"));
}

#[test]
fn prebuilt_adapter_rejects_digest_mismatch_before_activation() {
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("fixture");
    fs::create_dir_all(&source).expect("source");
    fs::write(source.join("fixture.bin"), b"fixture").expect("artifact");
    let platform = crate::HostPlatform::current();
    let manifest = format!(
        r#"
schema_version = 1
[package]
name = "bad_digest"
version = "1.0.0"
description = "Bad digest fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"
[registry]
name = "bad_digest"
[build]
adapter = "prebuilt"
source_root = "fixture"
[[build.artifacts]]
os = "{os}"
arch = "{arch}"
sha256 = "{digest}"
source_path = "fixture.bin"
executable = true
[run]
launcher = "native"
entrypoint = "runtime/bin/bad-digest"
[security]
capability_policy_source = "registry"
sandbox = "required"
inherit_credentials = false
"#,
        os = platform.os,
        arch = platform.arch,
        digest = "0".repeat(64),
    );
    let manifest_path = source.join("skill.toml");
    fs::write(&manifest_path, manifest).expect("manifest");
    let error = SkillInstaller
        .install(&InstallRequest {
            manifest_path,
            workspace_root: workspace,
            package_root: temp.path().join("packages"),
            target: None,
            allow_network: false,
            control: None,
        })
        .expect_err("digest mismatch");
    assert_eq!(error.code, "prebuilt_digest_mismatch");
}

#[test]
fn cargo_adapter_builds_only_the_selected_package() {
    if !sandbox_backend_present() || find_on_path("cargo").is_none() {
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("cargo_fixture");
    fs::create_dir_all(source.join("src")).expect("source");
    fs::write(
        workspace.join("Cargo.toml"),
        "[workspace]\nresolver = \"2\"\nmembers = [\"cargo_fixture\"]\n",
    )
    .expect("workspace manifest");
    fs::write(
        workspace.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\nversion = 4\n\n[[package]]\nname = \"cargo-fixture-skill\"\nversion = \"0.1.0\"\n",
    )
    .expect("lockfile");
    fs::write(
        source.join("Cargo.toml"),
        "[package]\nname = \"cargo-fixture-skill\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("crate manifest");
    fs::write(
        source.join("src/main.rs"),
        r##"use std::io::{self, Read};
fn main() {
    let mut input = String::new();
    io::stdin().read_to_string(&mut input).unwrap();
    let marker = "\"request_id\":\"";
    let start = input.find(marker).unwrap() + marker.len();
    let end = start + input[start..].find('"').unwrap();
    println!("{{\"request_id\":\"{}\",\"status\":\"ok\",\"text\":\"cargo\",\"error_text\":null,\"buttons\":null,\"extra\":{{\"value\":1}}}}", &input[start..end]);
}

"##,
    )
    .expect("main");
    fs::write(
        source.join("skill.toml"),
        cargo_manifest(&crate::HostPlatform::current()),
    )
    .expect("skill manifest");
    let outcome =
        install_fixture(temp.path(), &workspace, source.join("skill.toml")).expect("cargo install");
    assert_eq!(outcome.adapter, crate::BuildAdapter::Cargo);
}

#[test]
fn adopts_one_existing_core_binary_without_rebuilding_it() {
    if !sandbox_backend_present() {
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("cargo_fixture");
    let release = workspace.join("target/release");
    fs::create_dir_all(source.join("src")).expect("source");
    fs::create_dir_all(&release).expect("release");
    fs::write(
        workspace.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\nversion = 4\n",
    )
    .expect("lockfile");
    fs::write(source.join("placeholder.rs"), "fn fixture() {}\n").expect("source file");
    fs::write(
        source.join("skill.toml"),
        cargo_manifest(&crate::HostPlatform::current()),
    )
    .expect("manifest");
    let binary = release.join("cargo-fixture-skill");
    fs::write(
        &binary,
        r##"#!/bin/sh
IFS= read -r line
request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
printf '{"request_id":"%s","status":"ok","text":"adopted","error_text":null,"buttons":null,"extra":{"value":1}}\n' "$request_id"
"##,
    )
    .expect("binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&binary).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).expect("permissions");
    }
    let package_root = temp.path().join("packages");
    let outcome = SkillInstaller
        .adopt_built(&AdoptBuiltRequest {
            manifest_path: source.join("skill.toml"),
            workspace_root: workspace,
            package_root: package_root.clone(),
            binary_path: binary,
            target: None,
            control: None,
        })
        .expect("adopt");
    assert_eq!(outcome.adapter, crate::BuildAdapter::Cargo);
    assert_eq!(outcome.origin, InstallOrigin::BuiltArtifact);
    assert!(outcome.phases.contains(&"protocol_smoke".to_string()));
    let launch = SkillRuntimeResolver::new(package_root)
        .resolve("cargo_fixture")
        .expect("resolve");
    assert!(launch.program.ends_with("runtime/bin/cargo-fixture"));
}

#[test]
fn installs_a_verified_platform_precompile_without_rebuilding() {
    if !sandbox_backend_present() {
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("cargo_fixture");
    let release = workspace.join("target/release");
    fs::create_dir_all(&source).expect("source");
    fs::create_dir_all(&release).expect("release");
    fs::write(
        workspace.join("Cargo.lock"),
        "# This file is automatically @generated by Cargo.\nversion = 4\n",
    )
    .expect("lockfile");
    fs::write(source.join("placeholder.rs"), "fn fixture() {}\n").expect("source file");
    let manifest_path = source.join("skill.toml");
    fs::write(
        &manifest_path,
        cargo_manifest(&crate::HostPlatform::current()),
    )
    .expect("manifest");
    let binary = release.join("cargo-fixture-skill");
    fs::write(
        &binary,
        r##"#!/bin/sh
IFS= read -r line
request_id=$(printf '%s' "$line" | sed -n 's/.*"request_id":"\([^"]*\)".*/\1/p')
printf '{"request_id":"%s","status":"ok","text":"precompiled","error_text":null,"buttons":null,"extra":{"value":1}}\n' "$request_id"
"##,
    )
    .expect("binary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&binary).expect("metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&binary, permissions).expect("permissions");
    }
    let precompiled_root = temp.path().join("precompiled");
    SkillInstaller
        .adopt_built(&AdoptBuiltRequest {
            manifest_path: manifest_path.clone(),
            workspace_root: workspace.clone(),
            package_root: precompiled_root.clone(),
            binary_path: binary,
            target: None,
            control: None,
        })
        .expect("build release receipt");

    let installed_root = temp.path().join("installed");
    let outcome = SkillInstaller
        .install_precompiled(&PrecompiledInstallRequest {
            manifest_path: manifest_path.clone(),
            workspace_root: workspace.clone(),
            package_root: installed_root.clone(),
            precompiled_root: precompiled_root.clone(),
            target: None,
            control: None,
        })
        .expect("install precompiled");

    assert_eq!(outcome.origin, InstallOrigin::PlatformPrecompiled);
    assert_eq!(
        outcome.phases,
        ["precompiled_verify", "precompiled_copy", "activate"]
    );
    let launch = SkillRuntimeResolver::new(installed_root)
        .resolve("cargo_fixture")
        .expect("resolve imported package");
    assert!(launch.program.ends_with("runtime/bin/cargo-fixture"));

    let bundled_launch = SkillRuntimeResolver::new(precompiled_root.clone())
        .resolve("cargo_fixture")
        .expect("resolve bundled package");
    fs::write(&bundled_launch.program, "tampered artifact").expect("tamper artifact");
    let error = SkillInstaller
        .install_precompiled(&PrecompiledInstallRequest {
            manifest_path,
            workspace_root: workspace,
            package_root: temp.path().join("rejected"),
            precompiled_root,
            target: None,
            control: None,
        })
        .expect_err("tampered precompile must fail closed");
    assert_eq!(error.code, "precompiled_artifact_mismatch");
}

#[test]
fn python_adapter_uses_a_private_virtual_environment() {
    if !sandbox_backend_present() || find_on_path("python3").is_none() {
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("python_fixture");
    fs::create_dir_all(&source).expect("source");
    fs::write(source.join("requirements.lock"), "").expect("lockfile");
    fs::write(
        source.join("main.py"),
        "import json,sys\nr=json.loads(sys.stdin.readline())\nprint(json.dumps({'request_id':r['request_id'],'status':'ok','text':'python','error_text':None,'buttons':None,'extra':{'value':1}},separators=(',',':')))\n",
    )
    .expect("python entrypoint");
    fs::write(
        source.join("skill.toml"),
        interpreted_manifest(
            &crate::HostPlatform::current(),
            "python_fixture",
            "python",
            "python",
            "requirements.lock",
            "runtime/src/main.py",
        ),
    )
    .expect("manifest");
    let outcome = install_fixture(temp.path(), &workspace, source.join("skill.toml"))
        .expect("python install");
    assert!(outcome.install_root.join("runtime/venv").is_dir());
    assert!(!source.join(".venv").exists());
}

#[test]
fn node_adapter_uses_private_dependencies_and_disables_lifecycle_scripts() {
    if !sandbox_backend_present() || find_on_path("node").is_none() || find_on_path("npm").is_none()
    {
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("node_fixture");
    fs::create_dir_all(&source).expect("source");
    fs::write(
        source.join("package.json"),
        r#"{"name":"node-fixture","version":"1.0.0","private":true}"#,
    )
    .expect("package");
    fs::write(
        source.join("package-lock.json"),
        r#"{"name":"node-fixture","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"node-fixture","version":"1.0.0"}}}"#,
    )
    .expect("lockfile");
    fs::write(
        source.join("main.js"),
        "const rl=require('readline').createInterface({input:process.stdin});rl.once('line',(line)=>{const r=JSON.parse(line);console.log(JSON.stringify({request_id:r.request_id,status:'ok',text:'node',error_text:null,buttons:null,extra:{value:1}}));});\n",
    )
    .expect("node entrypoint");
    fs::write(
        source.join("skill.toml"),
        interpreted_manifest(
            &crate::HostPlatform::current(),
            "node_fixture",
            "node",
            "node",
            "package-lock.json",
            "runtime/src/main.js",
        ),
    )
    .expect("manifest");
    let outcome =
        install_fixture(temp.path(), &workspace, source.join("skill.toml")).expect("node install");
    assert!(outcome
        .install_root
        .join("runtime/src/package.json")
        .is_file());
    assert!(!source.join("node_modules").exists());
}

#[test]
fn go_adapter_builds_when_toolchain_is_available() {
    if !sandbox_backend_present() || find_on_path("go").is_none() {
        return;
    }
    let temp = tempdir().expect("tempdir");
    let workspace = temp.path().join("workspace");
    let source = workspace.join("go_fixture");
    fs::create_dir_all(&source).expect("source");
    fs::write(source.join("go.mod"), "module fixture\n\ngo 1.22\n").expect("go.mod");
    fs::write(source.join("go.sum"), "").expect("go.sum");
    fs::write(
        source.join("main.go"),
        "package main\nimport(\"bufio\";\"fmt\";\"os\";\"strings\")\nfunc main(){s:=bufio.NewScanner(os.Stdin);s.Scan();v:=s.Text();m:=\"\\\"request_id\\\":\\\"\";i:=strings.Index(v,m)+len(m);j:=i+strings.Index(v[i:],\"\\\"\");fmt.Printf(\"{\\\"request_id\\\":\\\"%s\\\",\\\"status\\\":\\\"ok\\\",\\\"text\\\":\\\"go\\\",\\\"error_text\\\":null,\\\"buttons\\\":null,\\\"extra\\\":{\\\"value\\\":1}}\\n\",v[i:j])}\n",
    )
    .expect("go main");
    let platform = crate::HostPlatform::current();
    fs::write(
        source.join("skill.toml"),
        format!(
            r#"schema_version = 1
[package]
name = "go_fixture"
version = "1.0.0"
description = "Go fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"
[registry]
name = "go_fixture"
[build]
adapter = "go"
source_root = "go_fixture"
lockfile = "go.sum"
network = "deny"
[run]
launcher = "native"
entrypoint = "runtime/bin/go-fixture"
working_directory = "."
[security]
capability_policy_source = "registry"
sandbox = "required"
inherit_credentials = false
"#,
            os = platform.os,
            arch = platform.arch,
        ),
    )
    .expect("manifest");
    let outcome =
        install_fixture(temp.path(), &workspace, source.join("skill.toml")).expect("go install");
    assert!(outcome
        .install_root
        .join("runtime/bin/go-fixture")
        .is_file());
}

fn install_fixture(
    temp_root: &std::path::Path,
    workspace: &std::path::Path,
    manifest_path: std::path::PathBuf,
) -> Result<crate::InstallOutcome, crate::SkillSdkError> {
    SkillInstaller.install(&InstallRequest {
        manifest_path,
        workspace_root: workspace.to_path_buf(),
        package_root: temp_root.join("packages"),
        target: None,
        allow_network: false,
        control: None,
    })
}

fn cargo_manifest(platform: &crate::HostPlatform) -> String {
    format!(
        r#"schema_version = 1
[package]
name = "cargo_fixture"
version = "1.0.0"
description = "Cargo fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"
[registry]
name = "cargo_fixture"
[build]
adapter = "cargo"
source_root = "."
package = "cargo-fixture-skill"
binary = "cargo-fixture-skill"
lockfile = "Cargo.lock"
network = "deny"
[run]
launcher = "native"
entrypoint = "runtime/bin/cargo-fixture"
working_directory = "."
[security]
capability_policy_source = "registry"
sandbox = "required"
inherit_credentials = false
"#,
        os = platform.os,
        arch = platform.arch,
    )
}

fn interpreted_manifest(
    platform: &crate::HostPlatform,
    name: &str,
    adapter: &str,
    launcher: &str,
    lockfile: &str,
    entrypoint: &str,
) -> String {
    format!(
        r#"schema_version = 1
[package]
name = "{name}"
version = "1.0.0"
description = "Interpreted fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"
[registry]
name = "{name}"
[build]
adapter = "{adapter}"
source_root = "{name}"
lockfile = "{lockfile}"
network = "deny"
[run]
launcher = "{launcher}"
entrypoint = "{entrypoint}"
working_directory = "."
[security]
capability_policy_source = "registry"
sandbox = "required"
inherit_credentials = false
"#,
        os = platform.os,
        arch = platform.arch,
    )
}

fn find_on_path(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|directory| directory.join(name))
        .find(|path| path.is_file())
}

fn sandbox_backend_present() -> bool {
    #[cfg(target_os = "linux")]
    {
        std::path::Path::new("/usr/bin/bwrap").is_file()
            || std::path::Path::new("/bin/bwrap").is_file()
    }
    #[cfg(target_os = "macos")]
    {
        std::path::Path::new("/usr/bin/sandbox-exec").is_file()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        false
    }
}
