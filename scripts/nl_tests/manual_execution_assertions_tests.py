import copy
import json
import unittest
from manual_execution_assertions import step_contract_assertion


class ExecutionContractTests(unittest.TestCase):
    def setUp(self):
        self.spec = {"skill": "fixture", "status": "ok", "fields": {
            "extra.package": "fixture-package", "extra.dry_run": False, "extra.exit_code": 0},
            "present": ["extra.operation_receipt"]}
        self.step = {"step_id": "s1", "executed_skill": "fixture", "status": "ok",
                     "observed_evidence": {"items": [
                         {"field": "extra.package", "kind": "string", "excerpt": "fixture-package"},
                         {"field": "extra.dry_run", "kind": "bool", "excerpt": "false"},
                         {"field": "extra.exit_code", "kind": "number", "excerpt": "0"},
                         {"field": "extra.operation_receipt", "kind": "object"}]}}

    def check(self, steps=None):
        return step_contract_assertion(json.dumps(self.spec), steps if steps is not None else [self.step])["ok"]

    def test_success(self):
        self.assertTrue(self.check())

    def test_missing_execution(self):
        self.assertFalse(self.check([]))

    def test_failed_install(self):
        self.step["status"] = "error"
        self.assertFalse(self.check())

    def test_wrong_package(self):
        self.step["observed_evidence"]["items"][0]["excerpt"] = "other"
        self.assertFalse(self.check())

    def test_no_receipt(self):
        self.step["observed_evidence"]["items"].pop()
        self.assertFalse(self.check())

    def test_dry_run(self):
        self.step["observed_evidence"]["items"][1]["excerpt"] = "true"
        self.assertFalse(self.check())

    def test_boolean_is_not_exit_code(self):
        self.step["observed_evidence"]["items"][2].update(kind="bool", excerpt="false")
        self.assertFalse(self.check())

    def test_no_cross_step_join(self):
        other = copy.deepcopy(self.step)
        self.step["observed_evidence"]["items"].pop(0)
        other["observed_evidence"]["items"].pop(1)
        self.assertFalse(self.check([self.step, other]))

    def test_duplicate_execution(self):
        self.assertFalse(self.check([self.step, copy.deepcopy(self.step)]))

    def test_total_calls_rejects_an_additional_unrelated_dispatch(self):
        self.spec["total_calls"] = 1
        self.assertTrue(self.check())
        other = {"step_id":"s2", "executed_skill":"other_fixture", "status":"ok"}
        self.assertFalse(self.check([self.step, other]))

    def test_total_calls_is_a_strict_non_boolean_count(self):
        for value in (True, False, "1", 0, -1, None):
            self.spec["total_calls"] = value
            self.assertFalse(self.check())

    def test_error_contract_can_bind_one_failed_dispatch(self):
        self.spec = {"skill":"fixture", "status":"error", "count":1, "total_calls":1,
                     "fields":{"error_code":"input_missing", "retryable":False}}
        self.step["status"] = "error"
        self.step["observed_evidence"] = {"items":[
            {"field":"error_code", "kind":"string", "excerpt":"input_missing"},
            {"field":"retryable", "kind":"bool", "excerpt":"false"}]}
        self.assertTrue(self.check())
        self.step["observed_evidence"]["items"].pop()
        self.assertFalse(self.check())

    def test_model_claims_cannot_replace_evidence(self):
        self.step["observed_evidence"] = {"items": []}
        self.step["output_excerpt"] = json.dumps(self.spec["fields"])
        self.assertFalse(self.check())

    def test_malformed(self):
        for spec in ("null", "[]", "{}", "invalid"):
            self.assertFalse(step_contract_assertion(spec, [self.step])["ok"])

    def test_runner_wiring(self):
        from manual_case_assertions import evaluate_expectations
        from manual_case_assertions_tests import result_with_steps
        self.step.update(requested_action_type="call_skill", requested_capability="fixture.install")
        obj = result_with_steps([self.step])
        status, _ = evaluate_expectations("step_contract_json:" + json.dumps(self.spec), "", obj,
                                          "succeeded", "", obj["data"]["result_json"], {})
        self.assertEqual(status, "pass")


if __name__ == "__main__":
    unittest.main()
