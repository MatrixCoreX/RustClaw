"""Probe validation/self-tests; these never launch or compile a daemon."""
from copy import deepcopy
import io
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import tomllib
import unittest
from unittest import mock

import yaml
import memory_project_process_probe as probe


ROOT = Path(__file__).resolve().parents[2]


def valid_result():
    return dict(schema_version=1, source="a" * 40,
                cases=[dict(name=name, passed=True) for name in sorted(probe.CASES)],
                launches=5, processes_stopped=True, tasks_submitted=0,
                credential_environment_empty=True,
                binary_sha256={name: "b" * 64 for name in ["clawd", "skill-runner", "skillctl"]},
                full_goal_complete=False, provider_crash_recovery_proven=False,
                old_binary_upgrade_proven=False)


def required_job(job):
    steps = job.get("steps", [])
    build = [step for step in steps if step.get("name") == "Build exact native consumers"]
    execute = [step for step in steps if step.get("name") == "Probe real startup and restart"]
    if len(build) != 1 or len(execute) != 1:
        return False
    command = execute[0].get("run", "")
    return (job.get("strategy", {}).get("matrix", {}).get("os") == ["ubuntu-22.04", "macos-15"]
            and not job.get("continue-on-error", False)
            and all("if" not in step and not step.get("continue-on-error", False)
                    for step in build + execute)
            and "cargo build --locked" in build[0].get("run", "")
            and all(token in build[0]["run"] for token in ["-p clawd", "-p skill-runner", "-p agent-skill-sdk"])
            and "set -euo pipefail" in command and "|| true" not in command
            and 'python3 -B scripts/tests/memory_project_process_probe.py' in command
            and '--source "$GITHUB_SHA"' in command
            and all(flag in command for flag in ["--clawd", "--skill-runner", "--skillctl", "--output"])
            and steps.index(build[0]) < steps.index(execute[0]))


class ProcessProbeTests(unittest.TestCase):
    def test_complete_result_is_accepted(self):
        probe.validate_result(valid_result())

    def test_partial_duplicate_failed_or_overclaimed_results_are_rejected(self):
        mutations = [
            lambda value: value["cases"].pop(),
            lambda value: value["cases"].append(value["cases"][0]),
            lambda value: value["cases"][0].update(passed=False),
            lambda value: value.update(launches=4),
            lambda value: value.update(processes_stopped=False),
            lambda value: value.update(tasks_submitted=1),
            lambda value: value.update(credential_environment_empty=False),
            lambda value: value.update(source="branch-name"),
            lambda value: value["binary_sha256"].pop("skillctl"),
            lambda value: value["binary_sha256"].update(clawd="unverified"),
            lambda value: value.update(full_goal_complete=True),
            lambda value: value.update(provider_crash_recovery_proven=True),
            lambda value: value.update(old_binary_upgrade_proven=True),
        ]
        for mutate in mutations:
            value = valid_result()
            mutate(value)
            with self.subTest(mutation=mutate), self.assertRaises(AssertionError):
                probe.validate_result(value)

    def test_environment_is_allowlisted_and_credentials_are_not_inherited(self):
        def sources(_repository, _source, workspace):
            (workspace / "configs").mkdir()
        with tempfile.TemporaryDirectory(prefix="project-probe-unit-") as directory:
            binaries = {name: Path(sys.executable) for name in ["clawd", "skill-runner", "skillctl"]}
            with mock.patch.object(probe, "copy_sources", side_effect=sources), mock.patch.dict(
                    os.environ, {"TEST_PROVIDER_KEY": "fixture-not-inherited"}):
                host = probe.Host(ROOT, "a" * 40, binaries, Path(directory), "git")
            self.assertNotIn("TEST_PROVIDER_KEY", host.environment)
            self.assertEqual(set(host.environment), {"PATH", "WORKSPACE_ROOT", "APP_PRODUCT_IDENTITY_CONFIG",
                             "APP_INTERNAL_LISTEN", "APP_SKILL_RUNNER_PATH", "NO_PROXY", "RUST_LOG"})
            self.assertTrue(host.environment["APP_INTERNAL_LISTEN"].startswith("127.0.0.1:"))
            self.assertIsNone(host.process)
            with self.assertRaises(AssertionError):
                host.api("/v1/tasks")
            config = tomllib.loads(probe.fixture_config())
            self.assertEqual(config["llm"]["custom"]["api_key"], "")
            self.assertEqual(config["llm"]["custom"]["base_url"], "http://127.0.0.1:9/v1")

    def test_fixture_source_archive_rejects_path_escape_and_links(self):
        for name, kind in [("../escape", tarfile.REGTYPE), ("/absolute", tarfile.REGTYPE),
                           ("configs/link", tarfile.SYMTYPE)]:
            archive_bytes = io.BytesIO()
            with tarfile.open(fileobj=archive_bytes, mode="w") as archive:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.linkname = "../escape"
                archive.addfile(member)
            with tempfile.TemporaryDirectory(prefix="project-probe-archive-") as directory:
                with mock.patch.object(probe.subprocess, "check_output", side_effect=[
                        "configs/product_identity.toml\n", archive_bytes.getvalue()]), self.assertRaises(AssertionError):
                    probe.copy_sources(ROOT, "a" * 40, Path(directory))

    def test_both_native_process_probes_are_required_in_workflow(self):
        workflow = yaml.safe_load((ROOT / ".github/workflows/memory-project-process-ci.yml").read_text())
        triggers = workflow.get("on", workflow.get(True))
        self.assertEqual(triggers["push"]["branches"], ["main", "ci/memory-project-process-*"])
        self.assertIn("scripts/tests/*memory_project_process_probe.py", triggers["push"]["paths"])
        self.assertIn("pull_request", triggers)
        job = workflow["jobs"]["project-process"]
        self.assertTrue(required_job(job))
        mutations = [
            lambda value: value["strategy"]["matrix"].update(os=["ubuntu-22.04"]),
            lambda value: value.update(**{"continue-on-error": True}),
            lambda value: value["steps"].reverse(),
        ]
        for mutate in mutations:
            broken = deepcopy(job)
            mutate(broken)
            self.assertFalse(required_job(broken))
        for name in ["Build exact native consumers", "Probe real startup and restart"]:
            for field, value in [("if", "false"), ("continue-on-error", True), ("run", "echo skipped")]:
                broken = deepcopy(job)
                next(step for step in broken["steps"] if step.get("name") == name)[field] = value
                self.assertFalse(required_job(broken))


if __name__ == "__main__":
    unittest.main()
