"""Keep the bounded diagnostic independent from mandatory runtime acceptance."""
from copy import deepcopy
import json
from pathlib import Path
import re
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = ROOT / ".github/workflows/sdk-process-diagnostics.yml"
BUILD = "cargo test --locked -p agent-skill-sdk --lib --no-run"
OBSERVE = 'if timeout --signal=TERM --kill-after=10s 180s cargo test --locked -p agent-skill-sdk --lib reference_conformance_tests::reference_languages_share_protocol_and_atomic_lifecycle -- --exact --nocapture --test-threads=1 > "$RUNNER_TEMP/sdk-process-diagnostics/reference.log" 2>&1; then'
OBSERVATION_LINES = [
    "set -euo pipefail", "result=0", OBSERVE, "  result=0", "else", "  result=$?", "fi",
    'printf \'%s\\n\' "$result" > "$RUNNER_TEMP/sdk-process-diagnostics/exit-code.txt"',
    'ps -e -o pid,ppid,pgid,sid,stat,comm > "$RUNNER_TEMP/sdk-process-diagnostics/processes.txt"',
    'cat "$RUNNER_TEMP/sdk-process-diagnostics/reference.log"', 'exit "$result"',
]
PARALLEL_OBSERVE = 'if timeout --signal=TERM --kill-after=10s 180s cargo test --locked -p agent-skill-sdk -- --nocapture > "$RUNNER_TEMP/sdk-process-diagnostics/parallel.log" 2>&1; then'
PARALLEL_LINES = [
    "set -euo pipefail", "result=0", PARALLEL_OBSERVE, "  result=0", "else", "  result=$?", "fi",
    'printf \'%s\\n\' "$result" > "$RUNNER_TEMP/sdk-process-diagnostics/parallel-exit-code.txt"',
    'ps -e -o pid,ppid,pgid,sid,stat,comm > "$RUNNER_TEMP/sdk-process-diagnostics/parallel-processes.txt"',
    'cat "$RUNNER_TEMP/sdk-process-diagnostics/parallel.log"', 'exit "$result"',
]
UPLOAD = "actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02"


def valid(workflow):
    if workflow.get("permissions") != {"contents": "read"}:
        return False
    if re.search(r"\$\{\{[^}]*\bsecrets\s*(?:\.|\[)", json.dumps(workflow)):
        return False
    if workflow.get("on") != {
        "workflow_dispatch": None,
        "push": {"branches": ["ci/runtime-hardening-*"], "paths": [
            ".github/workflows/sdk-process-diagnostics.yml",
            "scripts/nl_tests/sdk_process_diagnostics_workflow_tests.py",
        ]},
    }:
        return False
    if set(workflow.get("jobs", {})) != {"sdk-process-diagnostics"}:
        return False
    job = workflow["jobs"]["sdk-process-diagnostics"]
    if (job.get("runs-on") != "ubuntu-22.04" or job.get("timeout-minutes") != 20
            or job.get("env", {}).get("APP_REQUIRE_REFERENCE_ADAPTERS") != "1"
            or "continue-on-error" in job or "if" in job):
        return False
    commands = []
    uploads = []
    observations = []
    parallel = []
    for step in job["steps"]:
        if "continue-on-error" in step:
            return False
        if "if" in step and step.get("uses") != UPLOAD:
            return False
        if step.get("uses", "").startswith("actions/checkout@"):
            if step.get("with", {}).get("persist-credentials") is not False:
                return False
        if step.get("uses") == UPLOAD:
            uploads.append(step)
        if "run" in step:
            lines = step["run"].strip().splitlines()
            if step.get("shell") != "bash" or lines[0] != "set -euo pipefail":
                return False
            if OBSERVE in lines:
                observations.append(lines)
            if PARALLEL_OBSERVE in lines:
                parallel.append(lines)
            commands.extend(lines[1:])
    if observations != [OBSERVATION_LINES] or parallel != [PARALLEL_LINES]:
        return False
    required = [
        'git rev-parse HEAD > "$RUNNER_TEMP/sdk-process-diagnostics/source-sha.txt"',
        "python3 scripts/nl_tests/sdk_process_diagnostics_workflow_tests.py",
        BUILD, OBSERVE,
        'printf \'%s\\n\' "$result" > "$RUNNER_TEMP/sdk-process-diagnostics/exit-code.txt"',
        'ps -e -o pid,ppid,pgid,sid,stat,comm > "$RUNNER_TEMP/sdk-process-diagnostics/processes.txt"',
        'cat "$RUNNER_TEMP/sdk-process-diagnostics/reference.log"',
        PARALLEL_OBSERVE,
        'cat "$RUNNER_TEMP/sdk-process-diagnostics/parallel.log"',
    ]
    if any(commands.count(command) != 1 for command in required):
        return False
    if [commands.index(command) for command in required] != sorted(commands.index(command) for command in required):
        return False
    if len(uploads) != 1 or uploads[0].get("if") != "${{ always() }}":
        return False
    return uploads[0].get("with") == {
        "name": "sdk-process-diagnostics-${{ github.sha }}",
        "path": "${{ runner.temp }}/sdk-process-diagnostics",
        "if-no-files-found": "error", "retention-days": 7,
    }


class SdkProcessDiagnosticsWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.workflow = yaml.safe_load(WORKFLOW.read_text())

    def test_exact_sdk_lifecycle_and_failure_evidence_are_selected(self):
        self.assertTrue(valid(self.workflow))

    def test_missing_deadline_or_changed_scope_is_not_a_valid_diagnostic(self):
        for old, new in [("180s", ""), ("--kill-after=10s", ""),
                         (" --exact", ""), (BUILD, "true"),
                         ("reference_conformance_tests::reference_languages_share_protocol_and_atomic_lifecycle", "nonexistent_filter")]:
            changed = deepcopy(self.workflow)
            for step in changed["jobs"]["sdk-process-diagnostics"]["steps"]:
                if "run" in step:
                    step["run"] = step["run"].replace(old, new)
            with self.subTest(mutation=old):
                self.assertFalse(valid(changed))

    def test_timeout_cannot_be_reported_as_success(self):
        for old, new in [('  result=$?', '  result=0'), ('exit "$result"', 'exit 0')]:
            changed = deepcopy(self.workflow)
            for step in changed["jobs"]["sdk-process-diagnostics"]["steps"]:
                if "run" in step:
                    step["run"] = step["run"].replace(old, new)
            self.assertFalse(valid(changed))
        changed = deepcopy(self.workflow)
        for step in changed["jobs"]["sdk-process-diagnostics"]["steps"]:
            if "run" in step:
                step["run"] = step["run"].replace('exit "$result"', 'result=0\nexit "$result"')
        self.assertFalse(valid(changed))

    def test_full_sdk_cannot_be_filtered_or_forced_to_run_serially(self):
        for replacement in (
            "cargo test --locked -p agent-skill-sdk reference_conformance_tests:: -- --nocapture",
            "cargo test --locked -p agent-skill-sdk -- --nocapture --test-threads=1",
        ):
            changed = deepcopy(self.workflow)
            for step in changed["jobs"]["sdk-process-diagnostics"]["steps"]:
                if "run" in step:
                    step["run"] = step["run"].replace(
                        "cargo test --locked -p agent-skill-sdk -- --nocapture", replacement)
            self.assertFalse(valid(changed))

    def test_full_arguments_or_environment_must_not_replace_process_names(self):
        changed = deepcopy(self.workflow)
        for step in changed["jobs"]["sdk-process-diagnostics"]["steps"]:
            if "run" in step:
                step["run"] = step["run"].replace("pid,ppid,pgid,sid,stat,comm", "pid,args")
        self.assertFalse(valid(changed))

    def test_bypass_permissions_and_missing_failure_upload_are_rejected(self):
        for mutation in ("permissions", "continue", "conditional", "upload"):
            changed = deepcopy(self.workflow)
            job = changed["jobs"]["sdk-process-diagnostics"]
            if mutation == "permissions":
                changed["permissions"] = {"contents": "write"}
            elif mutation == "continue":
                job["continue-on-error"] = True
            elif mutation == "conditional":
                job["steps"][-2]["if"] = "false"
            else:
                job["steps"][-1].pop("if")
            self.assertFalse(valid(changed))


if __name__ == "__main__":
    unittest.main()
