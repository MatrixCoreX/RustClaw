#!/usr/bin/env python3
"""Run explicitly authorized NL in a disposable Docker host, never the workstation.

Only Git-visible source and declared runtime artifacts enter the container.
No host filesystem mounts, ports, Docker socket or production state are exposed.
The ordinary NL runner and its assertion/trace contracts remain unchanged.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tempfile
import tomllib
import uuid

from materialize_isolated_workspace import git_visible_paths

ROOT = Path(__file__).resolve().parents[2]
SOURCE_ROOTS = {"configs", "prompts", "scripts", "crates", "docs", ".cargo",
                "optional_skills", "external_skills"}
SOURCE_FILES = {"Cargo.toml", "Cargo.lock", "rust-toolchain.toml", ".gitignore"}


def redact_config_credentials(source: str) -> str:
    lines = []
    for line in source.splitlines():
        key, sep, value = line.partition("=")
        token = key.strip()
        sensitive = token in {"password", "secret", "token", "private_key", "api_key"} or token.endswith(
            ("_password", "_secret", "_token", "_private_key", "_api_key"))
        if sep and sensitive and re.fullmatch(r"[a-z_]+", token):
            try:
                parsed = tomllib.loads("value=" + value).get("value")
            except tomllib.TOMLDecodeError as error:
                raise ValueError("credential_field_not_scalar") from error
            if not isinstance(parsed, str):
                raise ValueError("credential_field_not_string")
            if not (parsed.startswith("${") and parsed.endswith("}")):
                line = key + '= ""'
        lines.append(line)
    output = "\n".join(lines) + "\n"
    tomllib.loads(output)
    return output


def docker_create_args(image: str, name: str, nested_sandbox: bool = False) -> list[str]:
    args = ["create", "--name", name, "--init", "--memory", "3g", "--cpus", "2",
            "--pids-limit", "256", "--cap-drop", "ALL", "--cap-add", "CHOWN",
            "--cap-add", "DAC_OVERRIDE", "--cap-add", "FOWNER", "--cap-add", "SETUID",
            "--cap-add", "SETGID", "--security-opt", "no-new-privileges:true",
            "--workdir", "/workspace"]
    if nested_sandbox:
        # Only the unmounted disposable container gains namespace setup rights;
        # each tool still runs inside its policy-selected Bubblewrap sandbox.
        args += ["--cap-add", "SYS_ADMIN", "--cap-add", "SYS_CHROOT", "--cap-add", "NET_ADMIN",
                 "--security-opt", "seccomp=unconfined", "--security-opt", "apparmor=unconfined"]
    return args + [image, "sleep", "infinity"]


def test_host_config(source: str) -> str:
    """Change only the copied host policy; callers never overwrite source config."""
    data = tomllib.loads(source)
    if data["tools"]["sandbox_mode"] != "workspace_write":
        raise ValueError("unexpected_source_sandbox")
    section = ""
    lines = []
    changed = 0
    for line in source.splitlines():
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            section = stripped[1:-1]
        if section == "tools" and stripped.startswith("sandbox_mode ="):
            line = 'sandbox_mode = "danger_full"'
            changed += 1
        lines.append(line)
    if changed != 1:
        raise ValueError("sandbox_field_count_invalid")
    output = "\n".join(lines) + "\n"
    expected = dict(data["tools"], sandbox_mode="danger_full")
    if tomllib.loads(output)["tools"] != expected:
        raise ValueError("unexpected_policy_change")
    return output


def stage_source(source: Path, destination: Path, case_file: Path) -> None:
    for relative in git_visible_paths(source):
        if relative.parts[0] not in SOURCE_ROOTS and str(relative) not in SOURCE_FILES:
            continue
        src = source / relative
        if src.is_symlink() or not src.is_file():
            raise ValueError(f"source_not_regular:{relative}")
        dst = destination / relative
        dst.parent.mkdir(parents=True, exist_ok=True)
        if relative.parts[0] == "configs" and src.suffix == ".toml" and "i18n" not in relative.parts:
            dst.write_text(redact_config_credentials(src.read_text()), encoding="utf-8")
        else:
            shutil.copy2(src, dst)
    target = destination / "target/release"
    target.mkdir(parents=True)
    # Receipt adoption uses manifest-selected binaries, not a second skill list.
    from sys import path
    path.insert(0, str(source / "scripts"))
    from skill_store_packages import runner_specs
    names = {"clawd", "skill-runner", "skillctl"}
    for spec in runner_specs(source / "configs/skills_registry.toml"):
        if spec.install_mode != "on_demand" and spec.adapter == "cargo":
            names.add(spec.runner)
    for name in sorted(names):
        src = source / "target/release" / name
        if not src.is_file():
            raise ValueError(f"runtime_binary_missing:{name}")
        shutil.copy2(src, target / name)
    subprocess.run(["python3", str(source / "scripts/project_skill_receipts.py"),
                    "--sdk-cli", str(source / "target/release/skillctl"),
                    "--precompiled-root", str(source / "data/skill-packages"),
                    "--package-root", str(destination / "precompiled")],
                   check=True, stdout=subprocess.DEVNULL)
    shutil.copy2(case_file, destination / "cases.txt")
    config = destination / "configs/config.toml"
    config.write_text(test_host_config(config.read_text()), encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--case-file", type=Path, required=True)
    parser.add_argument("--log-root", type=Path, required=True)
    parser.add_argument("--image", default="agent-nl-disposable:20260915")
    parser.add_argument("--sudo-docker", action="store_true")
    parser.add_argument("--authorize-disposable-host", action="store_true")
    parser.add_argument("--build-image", action="store_true")
    parser.add_argument("--preinstall", choices=["sl"])
    parser.add_argument("--nested-sandbox", action="store_true",
                        help="Explicitly allow Bubblewrap namespaces inside the disposable container; never mounts the host")
    args = parser.parse_args()
    if not args.authorize_disposable_host:
        parser.error("--authorize-disposable-host is required; production policy is not changed")
    if not os.environ.get("MINIMAX_API_KEY"):
        parser.error("MINIMAX_API_KEY is required; no other host credentials are forwarded")
    docker = ["sudo", "-n", "docker"] if args.sudo_docker else ["docker"]
    def interrupted(signum, frame):
        raise InterruptedError(f"disposable_run_interrupted:{signum}")
    signal.signal(signal.SIGTERM, interrupted)
    run_id = "nl-disposable-" + uuid.uuid4().hex[:12]
    output = args.log_root.resolve() / run_id
    output.mkdir(parents=True)
    output.chmod(0o700)
    config_path = ROOT / "configs/config.toml"
    config_hash = hashlib.sha256(config_path.read_bytes()).hexdigest()
    manifest = {"run_id": run_id, "source_config_sha256": config_hash,
                "case_sha256": hashlib.sha256(args.case_file.read_bytes()).hexdigest(),
                "mounts": [], "published_ports": [], "production_state_access": False,
                "preinstall": args.preinstall, "authorization": "disposable_host_only"}
    manifest["nested_sandbox"] = args.nested_sandbox
    log_path = output / "run.log"
    print(json.dumps({"run_id": run_id, "log": str(log_path)}), flush=True)
    created = False
    returncode = 1
    with log_path.open("w") as log:
        def call(command: list[str], *, check: bool = True, input: str | None = None):
            return subprocess.run(docker + command, stdout=log, stderr=subprocess.STDOUT,
                                  input=input, text=True, check=check)
        try:
            if args.build_image:
                call(["build", "-t", args.image, str(ROOT / "scripts/nl_tests/fixtures/disposable_host")])
            call(docker_create_args(args.image, run_id, args.nested_sandbox))
            created = True
            call(["start", run_id])
            probe = call(["exec", run_id, "bwrap", "--ro-bind", "/", "/", "--proc", "/proc",
                          "--dev", "/dev", "--unshare-net", "--", "/bin/true"], check=False)
            if probe.returncode:
                raise RuntimeError("disposable_nested_sandbox_unavailable: choose an isolated host with namespace support")
            with tempfile.TemporaryDirectory(prefix="nl-disposable-stage-", dir="/tmp") as raw:
                stage = Path(raw)
                stage_source(ROOT, stage, args.case_file.resolve())
                manifest["runtime_sha256"] = hashlib.sha256((stage / "target/release/clawd").read_bytes()).hexdigest()
                call(["cp", str(stage) + "/.", run_id + ":/workspace"])
            call(["exec", run_id, "chown", "-R", "root:root", "/workspace"])
            call(["exec", run_id, "test", "-f", "/workspace/configs/config.toml"])
            call(["exec", run_id, "test", "-x", "/workspace/target/release/clawd"])
            # Input is piped, not a command argument, Docker env or persisted log.
            call(["exec", "-i", run_id, "bash", "-c",
                  "umask 077; mkdir -p /run/nl; python3 -c 'import sys,json; json.dump(json.load(sys.stdin),open(\"/run/nl/provider.json\",\"w\"))'"],
                 input=json.dumps({"MINIMAX_API_KEY": os.environ["MINIMAX_API_KEY"],
                                   "GITHUB_GIT_TOKEN": "disposable-invalid-test-credential"}))
            call(["exec", run_id, "git", "init", "-q", "/workspace"])
            call(["exec", run_id, "git", "-C", "/workspace", "add", "configs", "prompts", "scripts", "crates", "Cargo.toml", "Cargo.lock"])
            if args.preinstall:
                call(["exec", run_id, "apt-get", "install", "-y", args.preinstall])
            # Existing runner owns all task submission, raw LLM numbering and cleanup.
            result = call(["exec", run_id, "python3", "-c",
                "import json,os; os.environ.update(json.load(open('/run/nl/provider.json'))); "
                "os.execvp('bash',['bash','scripts/nl_tests/run_all_nl_with_server.sh',"
                "'--no-runtime-env','--suite','manual','--keep-isolated-state','--no-prompt-reply-only',"
                "'--precompiled-root','/workspace/precompiled',"
                "'--wait-seconds','1800','--provider-retries','0',"
                "'--log-dir','/workspace/scripts/nl_suite_logs/disposable',"
                "'--','--case-file','/workspace/cases.txt','--log-root',"
                "'/workspace/scripts/nl_suite_logs/disposable/cases','--full-text'])"], check=False)
            returncode = result.returncode
        finally:
            if created:
                # Save evidence, not installed executables or provider secrets.
                call(["exec", run_id, "bash", "-c",
                      "mkdir -p /evidence; cp -a scripts/nl_suite_logs/. /evidence/; "
                      "find tmp -type f \\( -name '*.sqlite' -o -name '*.sqlite-wal' -o -name '*.sqlite-shm' -o -name 'model_io.log' \\) -exec cp --parents '{}' /evidence/ \\;; "
                      "cp -a /var/log/dpkg.log /evidence/; dpkg-query -W -f='${Status}' sl > /evidence/sl-final-state.txt 2>/dev/null || true"], check=False)
                call(["exec", run_id, "chown", "-R", f"{os.getuid()}:{os.getgid()}", "/evidence"], check=False)
                collected = call(["cp", "-a", run_id + ":/evidence/.", str(output)], check=False)
                if args.sudo_docker and collected.returncode == 0:
                    subprocess.run(["sudo", "-n", "chown", "-hR",
                                    f"{os.getuid()}:{os.getgid()}", str(output)], check=True)
                removed = call(["rm", "-f", run_id], check=False)
                manifest["evidence_collected"] = collected.returncode == 0
                manifest["container_removed"] = removed.returncode == 0
                if collected.returncode or removed.returncode:
                    returncode = 1
            manifest["exit_code"] = returncode
            manifest["production_config_unchanged"] = hashlib.sha256(config_path.read_bytes()).hexdigest() == config_hash
            if not manifest["production_config_unchanged"]:
                returncode = manifest["exit_code"] = 1
            (output / "environment.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps(manifest), flush=True)
    return returncode


if __name__ == "__main__":
    raise SystemExit(main())
