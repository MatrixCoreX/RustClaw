"""Keep intentional failure observations distinct from failed execution goals."""
import copy
import json
from pathlib import Path
import unittest

from manual_case_assertions import evaluate_expectations
from manual_case_assertions_tests import capability_group_load_step, result_with_steps, terminal_step


class FailureObservationTests(unittest.TestCase):
    def setUp(self):
        path = Path(__file__).parent / "cases/nl_cases_builtin_tool_skill_fresh_20260830.txt"
        case = next(line for line in path.read_text().splitlines()
                    if line.startswith("builtin_fresh_20261029|fresh_video_missing_input_observation_zh|"))
        _, _, self.tags, _, expectation = case.split("|", 4)
        self.expectation = expectation.removeprefix("expect=")
        self.error = {"status": "error", "error_code": "video_input_missing", "retryable": False}
        self.step = {
            "step_id": "step_2", "requested_action_type": "call_capability",
            "requested_capability": "video.extract_audio", "executed_skill": "video_transcribe",
            "status": "error", "error_code": "video_input_missing",
            "observed_evidence": {"items": [
                {"field": "error_code", "kind": "string", "excerpt": "video_input_missing"},
                {"field": "retryable", "kind": "bool", "excerpt": "false"},
                {"field": "failure_phase", "kind": "string", "excerpt": "execution_no_effect"},
            ]},
        }

    def check(self, steps=None, status="succeeded", expectation=None, tags=None):
        text = json.dumps(self.error)
        result = result_with_steps(steps if steps is not None else [self.step], text=text)
        result["data"]["status"] = status
        outcome, _ = evaluate_expectations(
            expectation or self.expectation, self.tags if tags is None else tags, result, status, text,
            result["data"]["result_json"], {})
        return outcome

    def test_failed_observation_can_complete_reporting_goal(self):
        self.assertEqual(self.check(), "pass")
        self.assertNotIn("allow_terminal_failure", self.tags)
        self.error["status"] = "failed"
        self.assertEqual(self.check(), "pass")

    def test_discovery_may_precede_business_dispatch(self):
        for discovery_status in ("ok", "error"):
            discovery = capability_group_load_step()
            discovery.update(status=discovery_status, executed_skill="load_capability_groups")
            self.assertEqual(self.check([discovery, self.step, terminal_step()]), "pass")

    def test_response_json_cannot_replace_execution(self):
        self.assertEqual(self.check([terminal_step()]), "fail")

    def test_no_cross_step_evidence_join(self):
        other = copy.deepcopy(self.step)
        self.step["observed_evidence"]["items"].pop()
        other["observed_evidence"]["items"].pop(0)
        self.assertEqual(self.check([self.step, other]), "fail")

    def test_duplicate_execution_is_rejected(self):
        self.assertEqual(self.check([self.step, copy.deepcopy(self.step)]), "fail")

    def test_unrelated_business_dispatch_is_rejected(self):
        other = copy.deepcopy(self.step)
        other.update(executed_skill="audio_transcribe", requested_capability="audio.transcribe")
        self.assertEqual(self.check([self.step, other]), "fail")

    def test_successful_tool_is_not_failed_observation(self):
        self.step["status"] = "ok"
        self.assertEqual(self.check(), "fail")

    def test_each_error_fact_must_be_attested(self):
        for index in range(3):
            step = copy.deepcopy(self.step)
            step["observed_evidence"]["items"].pop(index)
            self.assertEqual(self.check([step]), "fail")

    def test_wrong_error_phase_or_retry_flag_is_rejected(self):
        for index, value in ((0, "other_error"), (1, "true"), (2, "unknown")):
            step = copy.deepcopy(self.step)
            step["observed_evidence"]["items"][index]["excerpt"] = value
            self.assertEqual(self.check([step]), "fail")

    def test_redacted_evidence_is_not_a_proof(self):
        self.step["observed_evidence"]["items"][2]["redacted"] = True
        self.assertEqual(self.check(), "fail")

    def test_final_report_must_not_claim_success(self):
        for status in ("ok", "succeeded", "unknown", None, False, 0):
            self.error["status"] = status
            self.assertEqual(self.check(), "fail")

    def test_genuine_failed_goal_is_a_distinct_contract(self):
        self.assertEqual(self.check(status="failed"), "fail")
        failed_goal = self.expectation.replace("/data/status=succeeded", "/data/status=failed")
        tags = self.tags + ";allow_terminal_failure"
        self.assertEqual(self.check(status="failed", expectation=failed_goal, tags=tags), "pass")
        self.assertEqual(self.check(expectation=failed_goal, tags=tags), "fail")


if __name__ == "__main__":
    unittest.main()
