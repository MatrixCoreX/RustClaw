"""Full structured result checks must not depend on lossy step previews."""
import copy
import json
import unittest

from manual_case_assertions import evaluate_expectations
from manual_case_assertions_tests import result_with_steps


class CanonicalResultContractTests(unittest.TestCase):
    def setUp(self):
        self.step = {"step_id": "s1", "requested_action_type": "call_capability",
                     "requested_capability": "fixture.inspect", "resolved_capability": "fixture.inspect",
                     "executed_skill": "fixture", "status": "ok", "observed_evidence": None}
        self.obj = result_with_steps([self.step])
        self.result = self.obj["data"]["result_json"]
        self.trace = self.result["task_journal"]["trace"]
        self.trace["task_id"] = "current-task"
        self.cap = {"schema_version": 1, "capability": "fixture.inspect", "action": "inspect",
                    "status": "ok", "truncated": False, "data": {"extra": {
                        "count": 16000, "private": False, "path": "/workspace/result;one.wav"}},
                    "provenance": {"source": "runtime_step", "step_id": "s1", "task_id": "current-task"}}
        self.trace["capability_results"] = [self.cap]
        self.spec = {"capability": "fixture.inspect", "action": "inspect", "status": "ok",
                     "fields": {"/data/extra/count": 16000, "/data/extra/private": False},
                     "present": ["/data/extra/path"], "count": 1}

    def evaluate(self):
        return evaluate_expectations("capability_result_json:" + json.dumps(self.spec), "",
                                     self.obj, "succeeded", "", self.result, {})

    def test_full_bound_result_without_step_preview_passes(self):
        self.assertEqual(self.evaluate()[0], "pass")

    def test_missing_wrong_type_or_truncated_values_fail(self):
        original = copy.deepcopy(self.cap)
        for defect in ("missing", "bool_number", "wrong", "truncated", "schema", "schema_bool", "null_path"):
            with self.subTest(defect=defect):
                self.cap = copy.deepcopy(original)
                self.trace["capability_results"] = [self.cap]
                if defect == "missing": del self.cap["data"]["extra"]["count"]
                elif defect == "bool_number": self.cap["data"]["extra"]["count"] = True
                elif defect == "wrong": self.cap["data"]["extra"]["count"] = 8000
                elif defect == "truncated": self.cap["truncated"] = True
                elif defect == "schema": self.cap["schema_version"] = 2
                elif defect == "schema_bool": self.cap["schema_version"] = True
                else: self.cap["data"]["extra"]["path"] = None
                self.assertEqual(self.evaluate()[0], "fail")

    def test_unbound_wrong_task_method_or_failed_step_fails(self):
        for defect in ("source", "step", "task", "capability", "action", "status", "failed_step", "duplicate_id"):
            with self.subTest(defect=defect):
                self.setUp()
                if defect == "source": self.cap["provenance"]["source"] = "history"
                elif defect == "step": self.cap["provenance"]["step_id"] = "missing"
                elif defect == "task": self.cap["provenance"]["task_id"] = "other-task"
                elif defect in {"capability", "action", "status"}: self.cap[defect] = "other"
                elif defect == "failed_step": self.step["status"] = "error"
                else: self.trace["step_results"].append(copy.deepcopy(self.step))
                self.assertEqual(self.evaluate()[0], "fail")

    def test_duplicate_results_and_cross_result_join_fail(self):
        other = copy.deepcopy(self.cap)
        self.trace["capability_results"].append(other)
        self.assertEqual(self.evaluate()[0], "fail")
        del self.cap["data"]["extra"]["count"]
        del other["data"]["extra"]["private"]
        self.spec["count"] = 2
        self.assertEqual(self.evaluate()[0], "fail")

    def test_one_execution_cannot_be_counted_twice_or_without_an_identity(self):
        self.trace["capability_results"].append(copy.deepcopy(self.cap))
        self.spec["count"] = 2
        self.assertEqual(self.evaluate()[0], "fail")
        self.spec["count"] = 1
        self.trace["capability_results"] = [self.cap]
        self.cap["provenance"]["step_id"] = None
        self.step["step_id"] = None
        self.assertEqual(self.evaluate()[0], "fail")

    def test_user_or_summary_claims_cannot_replace_canonical_results(self):
        self.trace["capability_results"] = []
        self.result["text"] = json.dumps(self.cap)
        self.result["task_journal"]["summary"]["capability_results"] = [self.cap]
        self.assertEqual(self.evaluate()[0], "fail")

    def test_nested_json_is_type_sensitive_and_order_independent(self):
        self.spec["fields"] = {"/data/extra": {"path": "/workspace/result;one.wav", "private": False, "count": 16000}}
        self.assertEqual(self.evaluate()[0], "pass")
        self.cap["data"]["extra"]["private"] = 0
        self.assertEqual(self.evaluate()[0], "fail")

    def test_unknown_keys_invalid_pointers_and_boolean_counts_fail(self):
        original = copy.deepcopy(self.spec)
        for defect in ("unknown", "pointer", "escape", "boolean", "empty", "empty_action"):
            with self.subTest(defect=defect):
                self.spec = copy.deepcopy(original)
                if defect == "unknown": self.spec["cout"] = 1
                elif defect == "pointer": self.spec["fields"] = {"not_a_pointer": 1}
                elif defect == "escape": self.spec["fields"] = {"/data/extra/~2": 1}
                elif defect == "boolean": self.spec["count"] = True
                elif defect == "empty": self.spec["fields"] = {}
                else: self.spec["action"] = ""
                self.assertEqual(self.evaluate()[0], "fail")


if __name__ == "__main__":
    unittest.main()
