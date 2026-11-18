#!/usr/bin/env python3
"""Probe real daemon project startup/restart using disposable, offline fixtures."""

from __future__ import annotations

import argparse
from contextlib import closing
import hashlib
import io
import json
import os
from pathlib import Path
import re
import signal
import socket
import sqlite3
import subprocess
import tarfile
import tempfile
import time
import urllib.error
import urllib.request


CASES = {
    "git/startup-before-first-GET",
    "git/repeated-GET-no-repair",
    "git/restart-preserves-project",
    "git/kill-restart-preserves-project",
    "canonical/startup-before-first-GET",
    "canonical/repeated-GET-no-repair",
    "malformed/health-without-identity",
    "malformed/structured-GET-unavailable-no-repair",
}
TABLES = (
    "memory_project_identities", "memory_project_aliases",
    "runtime_schema_migrations", "auth_keys", "principals", "credential_bindings",
)


def require(value, message):
    if not value:
        raise AssertionError(message)


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def validate_result(result):
    require(result["schema_version"] == 1, "result_schema")
    names = [case["name"] for case in result["cases"]]
    require(set(names) == CASES and len(names) == len(CASES), "complete_unique_cases")
    require(all(case["passed"] is True for case in result["cases"]), "case_failed")
    require(result["launches"] == 5 and result["processes_stopped"] is True,
            "all_real_processes_reaped")
    require(result["tasks_submitted"] == 0 and result["credential_environment_empty"] is True,
            "fixture_not_offline")
    require(re.fullmatch(r"[a-f0-9]{40}", result["source"]), "exact_source")
    require(set(result["binary_sha256"]) == {"clawd", "skill-runner", "skillctl"}, "all_consumers")
    require(all(re.fullmatch(r"[a-f0-9]{64}", value)
                for value in result["binary_sha256"].values()), "binary_digests")
    require(result["full_goal_complete"] is False and result["provider_crash_recovery_proven"] is False
            and result["old_binary_upgrade_proven"] is False, "scope_not_broadened")


def copy_sources(repository, source, workspace):
    names = subprocess.check_output(
        ["git", "ls-tree", "-r", "--name-only", source], cwd=repository, text=True,
    ).splitlines()
    names = [name for name in names if name.startswith("prompts/")
             or (name.startswith("configs/") and not name.startswith("configs/channels/"))
             or name in {"Cargo.toml", "Cargo.lock"}
             or (name.startswith(("crates/skills/", "optional_skills/", "external_skills/"))
                 and name.endswith(("/skill.toml", "/INTERFACE.md")))]
    data = subprocess.check_output(["git", "archive", source, *names], cwd=repository)
    with tarfile.open(fileobj=io.BytesIO(data)) as archive:
        for member in archive.getmembers():
            path = Path(member.name)
            require(not path.is_absolute() and ".." not in path.parts
                    and (member.isdir() or member.isfile()), "fixture_source_escape_or_link")
        archive.extractall(workspace, filter="data")


def fixture_config():
    return '''[server]
request_timeout_seconds = 10
[database]
sqlite_path = "data/fixture.db"
audit_sqlite_path = "data/audit.db"
skill_data_root = "data/skills"
busy_timeout_ms = 5000
[worker]
concurrency = 1
poll_interval_ms = 100
[llm]
selected_vendor = "custom"
[llm.custom]
base_url = "http://127.0.0.1:9/v1"
api_key = ""
model = "fixture-model"
api_format = "openai_compat"
[skills]
registry_path = "configs/skills_registry.toml"
skills_list = []
skill_switches = {}
uninstalled_skills = []
runner_warm_pool_enabled = false
[memory]
config_path = "configs/fixture-memory.toml"
'''


class Host:
    def __init__(self, repository, source, binary_paths, output, kind):
        self.root = Path(tempfile.mkdtemp(prefix=kind + "-", dir=output))
        self.root.chmod(0o700)
        self.workspace = self.root / "workspace"
        self.workspace.mkdir(mode=0o700)
        copy_sources(repository, source, self.workspace)
        if kind == "git":
            (self.workspace / ".git").mkdir()
        elif kind == "malformed":
            (self.workspace / ".git").write_text("invalid fixture marker\n")
        else:
            require(not any((parent / ".git").exists() for parent in self.workspace.parents),
                    "canonical_fixture_inside_a_repository")
        (self.workspace / "configs/config.toml").write_text(fixture_config())
        (self.workspace / "configs/fixture-memory.toml").write_text(
            "[memory]\nlong_term_enabled=false\nhybrid_recall_enabled=false\n"
            "reindex_on_startup=false\nllm_preference_fallback_enabled=false\n")
        self.binaries = binary_paths
        self.binary_hashes = {name: digest(path) for name, path in binary_paths.items()}
        with socket.socket() as reservation:
            reservation.bind(("127.0.0.1", 0))
            self.port = reservation.getsockname()[1]
        # Deliberate allowlist: no inherited keys, proxies, channels, NNI state or product overrides.
        self.environment = {
            "PATH": "/usr/bin:/bin", "WORKSPACE_ROOT": str(self.workspace),
            "APP_PRODUCT_IDENTITY_CONFIG": str(self.workspace / "configs/product_identity.toml"),
            "APP_INTERNAL_LISTEN": "127.0.0.1:" + str(self.port),
            "APP_SKILL_RUNNER_PATH": str(binary_paths["skill-runner"]),
            "NO_PROXY": "127.0.0.1,localhost", "RUST_LOG": "info",
        }
        self.db = self.workspace / "data/fixture.db"
        self.opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.process = None
        self.launches = 0
        self.admin = None

    def sql(self, query):
        require(self.db.is_file() and not self.db.is_symlink(), "private_fixture_database_missing")
        require(self.db.resolve().is_relative_to(self.root.resolve()), "database_scope_escape")
        with closing(sqlite3.connect(self.db.as_uri() + "?mode=ro", uri=True, timeout=5)) as db:
            return db.execute(query).fetchall()

    def snapshot(self):
        rows = {}
        for table in TABLES:
            columns = [row[1] for row in self.sql("PRAGMA table_info(" + table + ")")
                       if not (table == "auth_keys" and row[1] == "last_used_at")]
            require(columns, "missing_snapshot_table:" + table)
            rows[table] = sorted(self.sql("SELECT " + ",".join(columns) + " FROM " + table), key=repr)
        return rows

    def api(self, path):
        require(path in {"/v1/health", "/v1/memory/projects/current"}, "probe_route_not_allowlisted")
        request = urllib.request.Request(
            "http://127.0.0.1:" + str(self.port) + path,
            headers={"X-Agent-Key": self.admin}, method="GET",
        )
        try:
            response = self.opener.open(request, timeout=5)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.status, json.load(response)

    def start(self):
        require(self.process is None or self.process.poll() is not None, "duplicate_live_process")
        require(all(digest(path) == self.binary_hashes[name] for name, path in self.binaries.items()),
                "binary_changed_before_launch")
        self.launches += 1
        log = self.root / ("process-" + str(self.launches) + ".log")
        with log.open("x") as stream:
            self.process = subprocess.Popen(
                [str(self.binaries["clawd"]), "--config", "configs/config.toml"],
                cwd=self.workspace, env=self.environment, stdout=stream,
                stderr=subprocess.STDOUT, start_new_session=True,
            )
        # This is only the probe's readiness observation bound, not a runtime task timeout.
        deadline = time.monotonic() + 90
        while time.monotonic() < deadline:
            require(self.process.poll() is None, "daemon_exited_before_readiness:" + str(log))
            try:
                if self.db.exists():
                    keys = self.sql("SELECT user_key FROM auth_keys WHERE role='admin' AND enabled=1")
                    self.admin = keys[0][0] if len(keys) == 1 else None
                if self.admin:
                    status, body = self.api("/v1/health")
                    if status == 200 and body.get("data", {}).get("worker_state") == "running":
                        return
            except (sqlite3.Error, OSError, urllib.error.URLError):
                pass
            time.sleep(0.1)
        raise AssertionError("fixture_readiness_observation_expired:" + str(log))

    def stop(self, kill=False):
        if self.process is not None and self.process.poll() is None:
            os.killpg(self.process.pid, signal.SIGKILL if kill else signal.SIGTERM)
            try:
                self.process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                os.killpg(self.process.pid, signal.SIGKILL)
                self.process.wait(timeout=5)
        require(self.process is None or self.process.poll() is not None, "fixture_process_not_reaped")

    def project(self):
        rows = self.sql("SELECT project_ref FROM memory_project_identities WHERE status='active'")
        require(len(rows) == 1 and re.fullmatch(r"project_[a-f0-9]{32}", rows[0][0]),
                "startup_did_not_initialize_one_opaque_project_before_first_GET")
        return rows[0][0]

    def verify_reads(self, expected):
        before = self.snapshot()
        for _ in range(3):
            status, body = self.api("/v1/memory/projects/current")
            require(status == 200 and body["ok"] is True
                    and body["data"]["scope_ref"] == expected, "wrong_project_HTTP_response")
            rendered = json.dumps(body)
            require(self.admin not in rendered and str(self.workspace) not in rendered
                    and "canonical_locator" not in rendered, "project_response_private_metadata")
        require(self.snapshot() == before, "GET_repaired_project_schema_or_credentials")
        require(self.sql("SELECT COUNT(*) FROM tasks") == [(0,)], "unexpected_fixture_task")


def run(repository, source, binaries, output):
    require(re.fullmatch(r"[a-f0-9]{40}", source), "exact_source_required")
    actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repository, text=True).strip()
    require(source == actual, "checkout_source_mismatch")
    require(not subprocess.check_output(["git", "diff", "HEAD", "--"], cwd=repository), "dirty_checkout")
    output = output.resolve(strict=True)
    require(not output.is_relative_to(repository.resolve()), "evidence_inside_source_workspace")
    binaries = {name: path.resolve(strict=True) for name, path in binaries.items()}
    require(all(path.is_file() and os.access(path, os.X_OK) for path in binaries.values()), "consumer_missing")
    hosts, cases = [], []
    def passed(name):
        cases.append({"name": name, "passed": True})
        print(json.dumps(cases[-1]), flush=True)
    try:
        for kind in ["git", "canonical", "malformed"]:
            host = Host(repository, source, binaries, output, kind)
            hosts.append(host)
            host.start()
            if kind == "malformed":
                require(host.sql("SELECT COUNT(*) FROM memory_project_identities") == [(0,)],
                        "malformed_marker_created_identity")
                passed("malformed/health-without-identity")
                before = host.snapshot()
                for _ in range(3):
                    status, body = host.api("/v1/memory/projects/current")
                    require(status == 503 and body["ok"] is False
                            and body["error"] == "memory_project_lookup_unavailable", "wrong_unavailable_response")
                    require(str(host.workspace) not in json.dumps(body), "unavailable_response_path_leak")
                require(host.snapshot() == before, "unavailable_GET_repaired_identity")
                require(host.sql("SELECT COUNT(*) FROM tasks") == [(0,)], "unexpected_fixture_task")
                passed("malformed/structured-GET-unavailable-no-repair")
            else:
                project = host.project()  # Deliberately before any project HTTP request.
                passed(kind + "/startup-before-first-GET")
                host.verify_reads(project)
                passed(kind + "/repeated-GET-no-repair")
                if kind == "git":
                    snapshot = host.snapshot()
                    for killed, name in [(False, "restart-preserves-project"),
                                         (True, "kill-restart-preserves-project")]:
                        host.stop(kill=killed)
                        host.start()
                        require(host.project() == project and host.snapshot() == snapshot,
                                "restart_changed_existing_project_or_identity")
                        host.verify_reads(project)
                        passed("git/" + name)
            host.stop()
    finally:
        for host in hosts:
            host.stop()
    result = dict(schema_version=1, source=source, platform=os.uname().sysname,
                  cases=cases, launches=sum(host.launches for host in hosts), processes_stopped=True,
                  binary_sha256={name: digest(path) for name, path in binaries.items()},
                  tasks_submitted=0, credential_environment_empty=True,
                  provider_crash_recovery_proven=False, old_binary_upgrade_proven=False,
                  full_goal_complete=False)
    validate_result(result)
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, sort_keys=True))
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", type=Path, required=True)
    parser.add_argument("--source", required=True)
    parser.add_argument("--clawd", type=Path, required=True)
    parser.add_argument("--skill-runner", type=Path, required=True)
    parser.add_argument("--skillctl", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    run(args.repository, args.source, {"clawd": args.clawd, "skill-runner": args.skill_runner,
                                     "skillctl": args.skillctl}, args.output)
