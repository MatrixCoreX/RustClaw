#![cfg(any(target_os = "linux", target_os = "macos"))]

use std::fs;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::{json, Value};
use skill_sdk::{HostPlatform, InstallRequest, SkillInstaller, SkillRuntimeResolver};
use tokio::io::AsyncWriteExt;

struct InstalledFixture {
    _root: tempfile::TempDir,
    workspace: PathBuf,
    packages: PathBuf,
    private: Vec<PathBuf>,
    request: Value,
}

impl InstalledFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().join("workspace");
        let source = workspace.join("fixture");
        fs::create_dir_all(&source).unwrap();
        fs::write(workspace.join("public"), b"public-marker").unwrap();
        let python = std::env::split_paths(&std::env::var_os("PATH").unwrap())
            .map(|path| path.join("python3"))
            .find(|path| path.is_file())
            .unwrap()
            .canonicalize()
            .unwrap();
        let program = format!(
            r#"#!{}
import errno, json, os, pathlib, sys
r = json.loads(sys.stdin.readline())
def readable(path):
    try:
        pathlib.Path(path).read_bytes()
    except OSError as error:
        if error.errno in (errno.ENOENT, errno.EACCES, errno.EPERM):
            return False
        raise
    return True
extra = {{'readable': [readable(p) for p in r['args'].get('probe_paths', [])],
         'received_keys': sorted(r),
         'parent_claims': [os.environ.get(k) for k in ['APP_PARENT_SANDBOX_BACKEND', 'APP_PARENT_PRIVATE_DIRECTORIES_DIGEST']]}}
print(json.dumps({{'request_id':r['request_id'], 'status':'ok', 'text':'fixture', 'error_text':None, 'extra':extra}}))
"#,
            python.display()
        );
        fs::write(source.join("fixture.py"), program).unwrap();
        let platform = HostPlatform::current();
        let manifest = format!(
            r#"
schema_version = 1
[package]
name = "private_policy_fixture"
version = "1.0.0"
description = "Host exclusion integration fixture"
protocol = "agent-jsonl-v1"
supported_os = ["{os}"]
supported_arch = ["{arch}"]
license = "MIT"
[registry]
name = "private_policy_fixture"
[build]
adapter = "generic_process"
source_root = "fixture"
network = "deny"
[run]
launcher = "process"
entrypoint = "runtime/src/fixture.py"
working_directory = "."
timeout_seconds = 10
[security]
capability_policy_source = "registry"
sandbox = "required"
runtime_network = false
inherit_credentials = false
[storage]
kind = "none"
schema_version = 1
migration_owner = "private_policy_fixture"
"#,
            os = platform.os,
            arch = platform.arch
        );
        let manifest_path = source.join("skill.toml");
        fs::write(&manifest_path, manifest).unwrap();
        let packages = root.path().join("packages");
        let installed = SkillInstaller
            .install(&InstallRequest {
                manifest_path,
                workspace_root: workspace.clone(),
                package_root: packages.clone(),
                target: None,
                allow_network: false,
                control: None,
            })
            .unwrap();
        assert!(installed
            .phases
            .iter()
            .any(|phase| phase == "protocol_smoke"));
        let launch = SkillRuntimeResolver::new(&packages)
            .resolve(&installed.skill_name)
            .unwrap();
        let private = ["first", "second"]
            .map(|name| {
                let directory = root.path().join(name);
                fs::DirBuilder::new()
                    .mode(0o700)
                    .create(&directory)
                    .unwrap();
                fs::write(directory.join("marker"), b"non-secret-marker").unwrap();
                directory.canonicalize().unwrap()
            })
            .into();
        let request = json!({
            "request_id": "fixture",
            "user_id": 1,
            "chat_id": 1,
            "skill_name": installed.skill_name,
            "expected_skill_version": launch.version,
            "expected_manifest_digest": launch.manifest_digest,
            "expected_receipt_digest": launch.receipt_digest,
            "expected_registry_generation": 1,
            "args": {},
            "context": null,
            "host_private_directories": [],
        });
        Self {
            _root: root,
            workspace,
            packages,
            private,
            request,
        }
    }

    async fn run(&self, requests: &[Value], parent_digest: Option<&str>) -> Vec<Value> {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_skill-runner"));
        command
            .env_clear()
            .env("PATH", std::env::var_os("PATH").unwrap())
            .env("APP_SKILL_PACKAGES_ROOT", &self.packages)
            .env("WORKSPACE_ROOT", &self.workspace)
            .current_dir(&self.workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(digest) = parent_digest {
            command
                .env(skill_sdk::PARENT_SANDBOX_BACKEND_ENV, "bubblewrap")
                .env(skill_sdk::PARENT_PRIVATE_DIRECTORIES_DIGEST_ENV, digest);
        }
        self.exchange(command, requests).await
    }

    async fn exchange(
        &self,
        mut command: tokio::process::Command,
        requests: &[Value],
    ) -> Vec<Value> {
        let mut child = command.spawn().unwrap();
        let mut input = child.stdin.take().unwrap();
        for request in requests {
            input
                .write_all(request.to_string().as_bytes())
                .await
                .unwrap();
            input.write_all(b"\n").await.unwrap();
        }
        input.shutdown().await.unwrap();
        drop(input);
        let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
            .await
            .expect("fixture runner deadline")
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let responses: Vec<Value> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(responses.len(), requests.len());
        responses
    }
}

#[tokio::test]
async fn warm_installed_runner_enforces_current_policy_and_keeps_protocol_private() {
    let fixture = InstalledFixture::new();
    let probes: Vec<_> = fixture
        .private
        .iter()
        .map(|path| path.join("marker"))
        .chain(std::iter::once(fixture.workspace.join("public")))
        .collect();
    let mut requests = Vec::new();
    for (index, directories) in [
        vec![fixture.private[0].clone()],
        vec![fixture.private[1].clone()],
        vec![],
    ]
    .into_iter()
    .enumerate()
    {
        let mut request = fixture.request.clone();
        request["request_id"] = json!(format!("request-{index}"));
        request["host_private_directories"] = json!(directories);
        request["args"] = json!({"probe_paths": probes});
        requests.push(request);
    }
    let mut missing = fixture.request.clone();
    missing
        .as_object_mut()
        .unwrap()
        .remove("host_private_directories");
    requests.push(missing);
    let responses = fixture.run(&requests, None).await;
    for (index, readable) in [
        json!([false, true, true]),
        json!([true, false, true]),
        json!([true, true, true]),
    ]
    .into_iter()
    .enumerate()
    {
        let response = &responses[index];
        assert_eq!(response["status"], "ok", "{response}");
        assert_eq!(response["request_id"], requests[index]["request_id"]);
        assert_eq!(response["extra"]["readable"], readable);
        assert_eq!(
            response["extra"]["received_keys"],
            json!([
                "args",
                "chat_id",
                "context",
                "request_id",
                "user_id",
                "user_key"
            ])
        );
        assert_eq!(response["extra"]["parent_claims"], json!([null, null]));
        assert_eq!(
            response["extra"]["execution_binding"]["receipt_digest"],
            fixture.request["expected_receipt_digest"]
        );
    }
    assert_eq!(responses[3]["status"], "error");
    assert_eq!(responses[3]["error_code"], "invalid_input");
    for path in &fixture.private {
        assert_eq!(fs::read(path.join("marker")).unwrap(), b"non-secret-marker");
    }
}

#[tokio::test]
async fn warm_installed_runner_refuses_stale_parent_policy_before_child_access() {
    let fixture = InstalledFixture::new();
    let mut request = fixture.request.clone();
    request["host_private_directories"] = json!([fixture.private[0]]);
    let responses = fixture.run(&[request], Some("stale-policy")).await;
    assert_eq!(responses[0]["status"], "error");
    assert_eq!(responses[0]["error_code"], "child_launch_invalid");
    assert_eq!(
        responses[0]["error_text"],
        "sandbox_parent_private_directories_mismatch"
    );
}

#[tokio::test]
async fn installed_runner_inherits_a_real_parent_boundary_without_exposing_host_claims() {
    let fixture = InstalledFixture::new();
    let launch = SkillRuntimeResolver::new(&fixture.packages)
        .resolve("private_policy_fixture")
        .unwrap();
    let _lease = skill_sdk::InstallReceiptStore::new(&fixture.packages)
        .acquire_version_lease(&launch.skill_name, &launch.install_root)
        .unwrap();
    let directories = vec![fixture.private[0].clone()];
    let digest = skill_sdk::private_directories_digest(&directories).unwrap();
    let prepared = skill_sdk::prepare_sandboxed_invocation(
        Path::new(env!("CARGO_BIN_EXE_skill-runner")),
        &fixture.workspace,
        &[],
        skill_sdk::SandboxNetwork::Deny,
        &directories,
    )
    .unwrap();
    let backend = prepared.backend;
    let mut command = tokio::process::Command::from(prepared.into_command());
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap())
        .env("APP_SKILL_PACKAGES_ROOT", &fixture.packages)
        .env("APP_SKILL_VERSION_LEASE_MODE", "host_held")
        .env("WORKSPACE_ROOT", &fixture.workspace)
        .env(skill_sdk::PARENT_SANDBOX_BACKEND_ENV, backend)
        .env(skill_sdk::PARENT_PRIVATE_DIRECTORIES_DIGEST_ENV, digest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut request = fixture.request.clone();
    request["host_private_directories"] = json!(directories);
    request["args"] = json!({"probe_paths": [fixture.private[0].join("marker"), fixture.workspace.join("public")]});
    let responses = fixture.exchange(command, &[request]).await;
    assert_eq!(responses[0]["status"], "ok", "{}", responses[0]);
    assert_eq!(responses[0]["extra"]["readable"], json!([false, true]));
    assert_eq!(responses[0]["extra"]["parent_claims"], json!([null, null]));
}
