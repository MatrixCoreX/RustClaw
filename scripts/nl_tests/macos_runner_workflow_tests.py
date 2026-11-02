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

    def test_focused_ui_mode_cannot_disable_automatic_full_runtime_checks(self):
        workflow = load("macos-polyglot-skill-ci.yml")
        option = workflow["on"]["workflow_dispatch"]["inputs"]["verification_scope"]
        self.assertEqual(option["default"], "all")
        self.assertEqual(option["options"], ["all", "ui"])
        self.assertEqual(workflow["jobs"]["polyglot-skill-macos"]["if"],
                         "${{ github.event_name != 'workflow_dispatch' || inputs.verification_scope != 'ui' }}")
        focused = workflow["jobs"]["ui-browser-reliability"]
        self.assertEqual(focused["if"],
                         "${{ github.event_name == 'workflow_dispatch' && inputs.verification_scope == 'ui' }}")
        self.assertEqual(focused["uses"], "./.github/workflows/ui-browser-reliability-ci.yml")

    def test_browser_preparation_is_followed_by_the_runtime_signature_selector(self):
        steps = load("macos-arm64-release-artifact.yml")["jobs"]["build-macos"]["steps"]
        script = next(step["run"] for step in steps if step.get("name") == "Test signed system browser and isolated skill profiles")
        prepare = script.index("python3 scripts/prepare_macos_ci_browser.py")
        self.assertLess(prepare, script.index('executable="$(bash scripts/macos-system-browser.sh --resolve-executable)"'))


if __name__ == "__main__":
    unittest.main()
