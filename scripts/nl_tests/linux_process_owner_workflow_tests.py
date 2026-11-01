"""Require actual Linux owner/SDK acceptance without bypassing identity gates."""
from copy import deepcopy
import json
from pathlib import Path
import re
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/linux-process-owner-ci.yml"
OWNER_CASES = (
    "local_process_owner::entry::tests::entry_executable_protocol_acceptance",
    "local_process_owner::authority_entry::tests::entry_executable_authority_acceptance",
    "skills::runner::owner_custody_tests::entry_executable_sdk_custody_acceptance",
    "skills::runner::owner_pre_ready_tests::entry_executable_pre_ready_acceptance",
    "skills::runner::owner_preparation_tests::entry_executable_ready_abort_acceptance",
    "skills::runner::owner_submission_tests::entry_executable_async_submit_acceptance",
    "skills::runner::owner_recovery_tests::entry_executable_inventory_recovery_acceptance",
)


def validate(workflow):
    if re.search(r"\$\{\{[^}]*\bsecrets\s*(?:\.|\[)", json.dumps(workflow)):
        return False
    if workflow.get("permissions") != {"contents": "read"}:
        return False
    events = workflow.get("on", {})
    if set(events) != {"push", "pull_request", "workflow_dispatch"}:
        return False
    if events["push"].get("branches") != ["main", "ci/runtime-hardening-*"]:
        return False
    for event in ("push", "pull_request"):
        if not {"crates/claw-core/**", "crates/skill-sdk/**", "scripts/clean_dev_artifacts.py",
                "scripts/tests/test_clean_dev_artifacts.py", "scripts/check-secrets.sh",
                "scripts/check_staged_secrets.py", "scripts/tests/test_check_staged_secrets.py"}.issubset(events[event].get("paths", [])):
            return False
    if set(workflow.get("jobs", {})) != {"process-owner-linux"}:
        return False
    job = workflow["jobs"]["process-owner-linux"]
    if job.get("runs-on") != "ubuntu-22.04" or "if" in job or "permissions" in job:
        return False
    if job.get("env", {}).get("CARGO_TARGET_DIR") != "target":
        return False
    if job.get("continue-on-error"):
        return False
    commands = []
    python_setups = 0
    for step in job["steps"]:
        if step.get("continue-on-error") or "if" in step:
            return False
        if "uses" in step:
            if step["uses"].startswith("actions/checkout@"):
                if step.get("with", {}).get("persist-credentials") is not False:
                    return False
            elif step["uses"] == "actions/setup-python@5fda3b95a4ea91299a34e894583c3862153e4b97":
                if step.get("with", {}).get("python-version") != "3.11":
                    return False
                python_setups += 1
            else:
                return False
        if "run" in step:
            lines = step["run"].strip().splitlines()
            if step.get("shell") != "bash" or lines[0] != "set -euo pipefail":
                return False
            if any("||" in line for line in lines):
                return False
            commands.extend(lines[1:])
    if python_setups != 1:
        return False
    required = [
        "python3 scripts/check_github_action_pins.py",
        "python3 scripts/nl_tests/linux_process_owner_workflow_tests.py",
        "python3 -m unittest discover -s scripts/tests -p test_clean_dev_artifacts.py -v",
        "python3 -m unittest discover -s scripts/tests -p test_check_staged_secrets.py -v",
        "cargo test --locked -p agent-skill-sdk",
        "cargo test --locked -p skill-runner",
        "cargo test --locked -p claw-core",
        "cargo build --locked -p clawd --bin clawd",
        *[f'APP_OWNER_ENTRY_TEST_BINARY="$PWD/target/debug/clawd" cargo test --locked -p clawd --bin clawd {case} -- --exact --ignored --test-threads=1 --nocapture'
          for case in OWNER_CASES],
        "cargo test --locked -p clawd --bin clawd workspace_ -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd runtime::host_filesystem::directory::tests -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd local_process_owner:: -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd skills::runner:: -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd cancellation_observation -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd resource_observation -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd local_process_job::tests -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd worker::async_poll_executor::tests -- --test-threads=1 --nocapture",
        "cargo test --locked -p clawd --bin clawd resource_scheduler:: -- --test-threads=1 --nocapture",
    ]
    if any(commands.count(command) != 1 for command in required):
        return False
    positions = [commands.index(command) for command in required]
    return positions == sorted(positions)


class LinuxProcessOwnerWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = yaml.safe_load(WORKFLOW.read_text(encoding="utf-8"))

    def test_actual_source_binary_and_all_explicit_cases_precede_identity_gate(self):
        self.assertTrue(validate(self.workflow))

    def test_skipping_ignored_case_or_using_a_test_binary_is_rejected(self):
        for original, replacement in (
            (" --ignored", ""),
            ('"$PWD/target/debug/clawd"', '"$PWD/target/debug/deps/clawd-test"'),
            (OWNER_CASES[2], "local_process_owner::"),
            ("cargo build --locked -p clawd --bin clawd", "true"),
            ("local_process_job::tests", "unrelated_zero_test_filter"),
            ("resource_observation", "unrelated_zero_test_filter"),
        ):
            changed = deepcopy(self.workflow)
            for step in changed["jobs"]["process-owner-linux"]["steps"]:
                if "run" in step:
                    step["run"] = step["run"].replace(original, replacement)
            with self.subTest(mutation=original):
                self.assertFalse(validate(changed))

    def test_failure_bypasses_and_privilege_expansion_are_rejected(self):
        for location, value in (
            ("continue-on-error", True), ("if", "always()"),
            ("permissions", {"contents": "write"}),
        ):
            changed = deepcopy(self.workflow)
            changed["jobs"]["process-owner-linux"][location] = value
            with self.subTest(mutation=location):
                self.assertFalse(validate(changed))
        for mutation in ("continue", "conditional", "shell", "failure", "credentials", "secret_context"):
            changed = deepcopy(self.workflow)
            steps = changed["jobs"]["process-owner-linux"]["steps"]
            if mutation == "continue":
                steps[-1]["continue-on-error"] = True
            elif mutation == "conditional":
                steps[-1]["if"] = "false"
            elif mutation == "shell":
                steps[-1]["run"] = steps[-1]["run"].replace("set -euo pipefail", "set +e")
            elif mutation == "failure":
                steps[-1]["run"] += "true || true\n"
            elif mutation == "credentials":
                steps[0]["with"]["persist-credentials"] = True
            else:
                steps[-1]["env"] = {"TOKEN": "${{ secrets.PROVIDER_TOKEN }}"}
            with self.subTest(mutation=mutation):
                self.assertFalse(validate(changed))

    def test_reordered_or_untriggered_sdk_acceptance_is_rejected(self):
        changed = deepcopy(self.workflow)
        steps = changed["jobs"]["process-owner-linux"]["steps"]
        steps[-1], steps[-2] = steps[-2], steps[-1]
        self.assertFalse(validate(changed))
        changed = deepcopy(self.workflow)
        steps = changed["jobs"]["process-owner-linux"]["steps"]
        steps[:] = [step for step in steps if step.get("name") != "Setup Python"]
        self.assertFalse(validate(changed))
        for event in ("push", "pull_request"):
            changed = deepcopy(self.workflow)
            changed["on"][event]["paths"].remove("crates/skill-sdk/**")
            self.assertFalse(validate(changed))

    def test_cleanup_regression_and_its_path_triggers_cannot_be_removed(self):
        command = "python3 -m unittest discover -s scripts/tests -p test_clean_dev_artifacts.py -v"
        changed = deepcopy(self.workflow)
        for step in changed["jobs"]["process-owner-linux"]["steps"]:
            if "run" in step:
                step["run"] = step["run"].replace(command, "")
        self.assertFalse(validate(changed))
        for event in ("push", "pull_request"):
            for path in ("scripts/clean_dev_artifacts.py", "scripts/tests/test_clean_dev_artifacts.py"):
                changed = deepcopy(self.workflow)
                changed["on"][event]["paths"].remove(path)
                self.assertFalse(validate(changed))

    def test_candidate_trigger_full_core_and_python_setup_cannot_be_weakened(self):
        changed = deepcopy(self.workflow)
        changed["on"]["push"]["branches"] = ["main"]
        self.assertFalse(validate(changed))

    def test_core_suite_cannot_be_removed_filtered_or_forced_serial(self):
        command = "cargo test --locked -p claw-core"
        for replacement in ("", "true", command + " model_catalog::",
                            command + " -- --test-threads=1"):
            changed = deepcopy(self.workflow)
            for step in changed["jobs"]["process-owner-linux"]["steps"]:
                if "run" in step:
                    step["run"] = step["run"].replace(command, replacement)
            with self.subTest(replacement=replacement):
                self.assertFalse(validate(changed))
        for event in ("push", "pull_request"):
            changed = deepcopy(self.workflow)
            changed["on"][event]["paths"].remove("crates/claw-core/**")
            self.assertFalse(validate(changed))

    def test_staged_guard_selection_and_all_triggers_are_required(self):
        command = "python3 -m unittest discover -s scripts/tests -p test_check_staged_secrets.py -v"
        changed = deepcopy(self.workflow)
        for step in changed["jobs"]["process-owner-linux"]["steps"]:
            if "run" in step:
                step["run"] = step["run"].replace(command, "")
        self.assertFalse(validate(changed))
        for event in ("push", "pull_request"):
            for path in ("scripts/check-secrets.sh", "scripts/check_staged_secrets.py",
                         "scripts/tests/test_check_staged_secrets.py"):
                changed = deepcopy(self.workflow)
                changed["on"][event]["paths"].remove(path)
                self.assertFalse(validate(changed))
        for original in (
            "cargo test --locked -p clawd --bin clawd workspace_ -- --test-threads=1 --nocapture",
            "cargo test --locked -p clawd --bin clawd runtime::host_filesystem::directory::tests -- --test-threads=1 --nocapture",
            "cargo test --locked -p clawd --bin clawd -- --test-threads=1 --nocapture",
            OWNER_CASES[-1],
        ):
            changed = deepcopy(self.workflow)
            for step in changed["jobs"]["process-owner-linux"]["steps"]:
                if "run" in step:
                    step["run"] = step["run"].replace(original, "missing_contract")
            self.assertFalse(validate(changed))
        changed = deepcopy(self.workflow)
        for step in changed["jobs"]["process-owner-linux"]["steps"]:
            if step.get("name") == "Setup Python":
                step["with"]["python-version"] = "3.10"
        self.assertFalse(validate(changed))


if __name__ == "__main__":
    unittest.main()
