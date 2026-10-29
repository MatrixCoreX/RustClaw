"""Guard artifact-only candidate builds without evaluating arbitrary CI expressions."""
from copy import deepcopy
from pathlib import Path
import unittest

import yaml


ROOT = Path(__file__).resolve().parents[2]
WORKFLOWS = ("ubuntu-x86_64-release.yml", "pi-aarch64-release.yml")
PUBLISH_GUARD = "${{ (github.event_name == 'push' && github.ref_type == 'tag') || (github.event_name == 'workflow_dispatch' && inputs.publish_release) }}"


def load_workflow(name):
    return yaml.safe_load((ROOT / ".github/workflows" / name).read_text(encoding="utf-8"))


def findings(workflow):
    problems = []
    flag = workflow.get("on", {}).get("workflow_dispatch", {}).get("inputs", {}).get("publish_release", {})
    if flag.get("type") != "boolean" or flag.get("default") is not True:
        problems.append("publish_input_not_typed_or_default_changed")
    publisher_count = 0
    contract_test_count = 0
    for job in workflow.get("jobs", {}).values():
        if "publish_release" in job.get("if", ""):
            problems.append("candidate_mode_gates_build_job")
        steps = job.get("steps", [])
        artifacts = [index for index, step in enumerate(steps)
                     if step.get("uses", "").startswith("actions/upload-artifact@")]
        attestations = [index for index, step in enumerate(steps)
                        if step.get("uses", "").startswith("actions/attest-build-provenance@")]
        for index, step in enumerate(steps):
            script = step.get("run", "")
            if "gh release " in script:
                publisher_count += 1
                if step.get("if") != PUBLISH_GUARD:
                    problems.append("release_writer_not_boolean_guarded")
                if not artifacts or max(artifacts) >= index:
                    problems.append("artifact_not_uploaded_before_publish")
                if not attestations or max(attestations) >= index:
                    problems.append("provenance_not_created_before_publish")
            elif "publish_release" in step.get("if", ""):
                problems.append("candidate_mode_gates_build_or_validation")
            if "cleanup-platform-releases" in script or "cancel-orphaned-platform-release" in script:
                problems.append("candidate_build_invokes_release_cleanup")
            if "release_candidate_workflow_tests.py" in script:
                contract_test_count += 1
                if 'test_python="${pythonLocation:?setup-python root missing}/bin/python3"' not in script:
                    problems.append("test_python_not_owned_by_setup_python")
                if '"${test_python}" -m pip install --requirement scripts/nl_tests/requirements.txt' not in script:
                    problems.append("test_dependencies_not_isolated_from_packaged_python")
    if publisher_count != 1:
        problems.append("unexpected_release_writer_count")
    if contract_test_count != 1:
        problems.append("candidate_contract_test_not_wired")
    return problems


class ReleaseCandidateWorkflowTests(unittest.TestCase):
    def test_release_workflows_offer_artifact_only_mode(self):
        for name in WORKFLOWS:
            with self.subTest(workflow=name):
                self.assertEqual(findings(load_workflow(name)), [])

    def test_unsafe_mutations_are_rejected(self):
        for name in WORKFLOWS:
            document = load_workflow(name)
            # Normalize the fixture's publish control so mutations isolate one
            # release boundary at a time.
            document["on"]["workflow_dispatch"]["inputs"]["publish_release"] = {
                "type": "boolean", "default": True,
            }
            for job in document["jobs"].values():
                for step in job.get("steps", []):
                    if "gh release " in step.get("run", ""):
                        step["if"] = PUBLISH_GUARD
            self.assertEqual(findings(document), [])
            with self.subTest(workflow=name, mutation="string_boolean"):
                changed = deepcopy(document)
                changed["on"]["workflow_dispatch"]["inputs"]["publish_release"]["default"] = "true"
                self.assertIn("publish_input_not_typed_or_default_changed", findings(changed))
            for guard in (None, "${{ github.event.inputs.publish_release }}",
                          "${{ github.event_name != 'workflow_dispatch' || inputs.publish_release }}"):
                with self.subTest(workflow=name, mutation=guard):
                    changed = deepcopy(document)
                    for job in changed["jobs"].values():
                        for step in job.get("steps", []):
                            if "gh release " in step.get("run", ""):
                                step["if"] = guard
                    self.assertIn("release_writer_not_boolean_guarded", findings(changed))
            with self.subTest(workflow=name, mutation="gated_artifact"):
                changed = deepcopy(document)
                for job in changed["jobs"].values():
                    for step in job.get("steps", []):
                        if step.get("uses", "").startswith("actions/upload-artifact@"):
                            step["if"] = "${{ inputs.publish_release }}"
                self.assertIn("candidate_mode_gates_build_or_validation", findings(changed))
            with self.subTest(workflow=name, mutation="gated_job"):
                changed = deepcopy(document)
                for job in changed["jobs"].values():
                    if job.get("steps"):
                        job["if"] = "${{ inputs.publish_release }}"
                self.assertIn("candidate_mode_gates_build_job", findings(changed))
            with self.subTest(workflow=name, mutation="missing_provenance"):
                changed = deepcopy(document)
                for job in changed["jobs"].values():
                    if job.get("steps"):
                        job["steps"] = [step for step in job["steps"]
                                        if not step.get("uses", "").startswith("actions/attest-build-provenance@")]
                self.assertIn("provenance_not_created_before_publish", findings(changed))
            with self.subTest(workflow=name, mutation="polluted_packaged_python"):
                changed = deepcopy(document)
                for job in changed["jobs"].values():
                    for step in job.get("steps", []):
                        script = step.get("run", "")
                        if "release_candidate_workflow_tests.py" in script:
                            step["run"] = script.replace('"${test_python}" -m pip', 'python3 -m pip')
                self.assertIn("test_dependencies_not_isolated_from_packaged_python", findings(changed))
            with self.subTest(workflow=name, mutation="missing_contract_test"):
                changed = deepcopy(document)
                for job in changed["jobs"].values():
                    for step in job.get("steps", []):
                        if "release_candidate_workflow_tests.py" in step.get("run", ""):
                            step["run"] = "true"
                self.assertIn("candidate_contract_test_not_wired", findings(changed))


if __name__ == "__main__":
    unittest.main()
