"""Keep native Mac verification on supported runners without changing target OS floors."""
from copy import deepcopy
from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
EXPECTED = (
    ("macos-polyglot-skill-ci.yml", "polyglot-skill-macos", "macos-15", None),
    ("macos-arm64-release-artifact.yml", "build-macos",
     "${{ inputs.architecture == 'x86_64' && 'macos-15-intel' || 'macos-15' }}", "13.0"),
    ("release-python-wheels.yml", "macos-wheels",
     "${{ startsWith(inputs.target, 'aarch64') && 'macos-15' || 'macos-15-intel' }}", "13.0"),
    ("desktop-native.yml", "desktop",
     "${{ matrix.platform == 'windows-x64' && 'windows-2022' || matrix.platform == 'macos-arm64' && 'macos-15' || 'macos-15-intel' }}", "13.0"),
)


def load(name):
    return yaml.safe_load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"))


def valid_job(job, runner, target):
    return job.get("runs-on") == runner and (
        target is None or str(job.get("env", {}).get("MACOSX_DEPLOYMENT_TARGET")) == target
    )


class MacosRunnerWorkflowTests(unittest.TestCase):
    def test_cache_cleanup_regression_is_selected_on_native_macos(self):
        workflow = load("macos-polyglot-skill-ci.yml")
        for event in ("push", "pull_request"):
            self.assertTrue({"scripts/clean_dev_artifacts.py", "scripts/tests/test_clean_dev_artifacts.py"}
                            .issubset(workflow["on"][event]["paths"]))
        command = "python3 -m unittest discover -s scripts/tests -p test_clean_dev_artifacts.py -v"
        steps = workflow["jobs"]["polyglot-skill-macos"]["steps"]
        self.assertEqual(sum(command in step.get("run", "") for step in steps), 1)

    def test_supported_runners_preserve_architecture_and_deployment_floor(self):
        for name, job_id, runner, target in EXPECTED:
            with self.subTest(workflow=name, job=job_id):
                self.assertTrue(valid_job(load(name)["jobs"][job_id], runner, target))

    def test_retired_runner_wrong_architecture_and_newer_floor_are_rejected(self):
        for name, job_id, runner, target in EXPECTED:
            job = load(name)["jobs"][job_id]
            for changed_runner in [runner.replace("macos-15", "macos-14"), "macos-15-intel"]:
                changed = deepcopy(job)
                changed["runs-on"] = changed_runner
                with self.subTest(workflow=name, runner=changed_runner):
                    self.assertFalse(valid_job(changed, runner, target))
            if target:
                changed = deepcopy(job)
                changed["env"]["MACOSX_DEPLOYMENT_TARGET"] = "15.0"
                with self.subTest(workflow=name, mutation="deployment_floor"):
                    self.assertFalse(valid_job(changed, runner, target))

    def test_runner_contract_is_in_native_ci(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        command = "python3 scripts/nl_tests/macos_runner_workflow_tests.py"
        self.assertEqual(sum(command in step.get("run", "") for step in steps), 1)

    def test_request_and_abort_admission_are_selected_before_known_process_authority_gate(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        command = "cargo test -p clawd --bin clawd skills::runner::startup:: -- --test-threads=1 --nocapture"
        selected = [step["run"] for step in steps if command in step.get("run", "")]
        self.assertEqual(len(selected), 1)
        self.assertLess(selected[0].index(command), selected[0].index("local_process_job::tests"))

    def test_owner_entry_uses_a_built_non_test_executable_and_explicit_fixture_selection(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        command = 'APP_OWNER_ENTRY_TEST_BINARY="$PWD/target/debug/clawd" cargo test -p clawd --bin clawd local_process_owner::entry::tests::entry_executable_protocol_acceptance -- --exact --ignored --test-threads=1 --nocapture'
        selected = [step["run"] for step in steps if command in step.get("run", "")]
        self.assertEqual(len(selected), 1)
        self.assertLess(selected[0].index("cargo build -p clawd --bin clawd"), selected[0].index(command))
        self.assertLess(selected[0].index(command), selected[0].index("local_process_job::tests"))

    def test_typed_producer_and_raw_environment_are_selected_before_known_authority_gate(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        for selection in ["skills::builtin::builtin_run_cmd::launch_tests::",
                          "skills::runner::launch_spec::",
                          "skills::tests::env_isolation::", "process_launch::"]:
            command = f"cargo test -p clawd --bin clawd {selection} -- --test-threads=1 --nocapture"
            selected = [step["run"] for step in steps if command in step.get("run", "")]
            self.assertEqual(len(selected), 1)
            self.assertLess(selected[0].index(command), selected[0].index("local_process_job::tests"))

    def test_sdk_and_runner_suites_precede_the_owner_authority_gate(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        runs = [step.get("run", "") for step in steps]
        gate = next(index for index, run in enumerate(runs) if "local_process_job::tests" in run)
        for command in ["cargo test -p agent-skill-sdk", "cargo test -p skill-runner"]:
            selected = [index for index, run in enumerate(runs) if command in run]
            self.assertEqual(len(selected), 1)
            self.assertLess(selected[0], gate)

    def test_cancellation_observation_precedes_the_known_identity_gate(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        command = "cargo test -p clawd --bin clawd cancellation_observation -- --test-threads=1 --nocapture"
        selected = [step["run"] for step in steps if command in step.get("run", "")]
        self.assertEqual(len(selected), 1)
        self.assertLess(selected[0].index(command), selected[0].index("local_process_job::tests"))

    def test_resource_observation_precedes_the_known_identity_gate(self):
        steps = load("macos-polyglot-skill-ci.yml")["jobs"]["polyglot-skill-macos"]["steps"]
        command = "cargo test -p clawd --bin clawd resource_observation -- --test-threads=1 --nocapture"
        selected = [step["run"] for step in steps if command in step.get("run", "")]
        self.assertEqual(len(selected), 1)
        self.assertLess(selected[0].index(command), selected[0].index("local_process_job::tests"))


if __name__ == "__main__":
    unittest.main()
