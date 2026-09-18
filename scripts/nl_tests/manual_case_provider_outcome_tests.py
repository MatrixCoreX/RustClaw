#!/usr/bin/env python3
"""Provider-error acceptance requires evidence from an actual skill invocation."""
import copy
import json
import unittest

from manual_case_assertions import skill_outcome_json_assertion
from manual_case_assertions_tests import capability_step, result_with_steps


class ProviderOutcomeTests(unittest.TestCase):
    def setUp(self):
        self.contract = json.dumps({"skill": "fixture_vision", "success_fields": ["description"]})
        self.final = {"status": "error", "error_code": "provider_unavailable",
                      "failure_phase": "provider_request", "provider": "fixture", "status_code": 500}
        self.step = capability_step(capability="fixture_vision.extract", dry_run=False,
                                    observed_fields=self.final)
        self.step.update(status="error", executed_skill="fixture_vision")

    def check(self, steps=None, final=None, contract=None):
        result = result_with_steps(steps if steps is not None else [self.step])["data"]["result_json"]
        return skill_outcome_json_assertion(contract or self.contract,
                                           json.dumps(self.final if final is None else final), result)

    def test_genuine_provider_error(self):
        checked = self.check()
        self.assertTrue(checked["ok"])
        self.assertEqual(checked["acceptance_path"], "provider_error")

    def test_success_needs_actual_successful_skill(self):
        self.step["status"] = "ok"
        self.step["observed_evidence"] = {"items": []}
        self.assertTrue(self.check(final={"description": "fixture description"})["ok"])
        self.assertFalse(self.check(final={"description": None})["ok"])
        self.assertFalse(self.check(final={"other": "description"})["ok"])

    def test_no_invocation_cannot_pass(self):
        self.assertFalse(self.check(steps=[])["ok"])

    def test_response_step_cannot_spoof_invocation(self):
        self.step["executed_skill"] = "respond"
        self.assertFalse(self.check()["ok"])

    def test_other_skill_cannot_pass(self):
        self.step["executed_skill"] = "fixture_other"
        self.assertFalse(self.check()["ok"])

    def test_unobserved_error_fields_cannot_pass(self):
        for field, wrong in (("error_code", "invented"), ("provider", "other"), ("status_code", 429)):
            with self.subTest(field=field):
                self.assertFalse(self.check(final={**self.final, field: wrong})["ok"])

    def test_preflight_error_is_not_provider_error(self):
        self.assertFalse(self.check(final={**self.final, "failure_phase": "preflight_verification"})["ok"])

    def test_failed_invocation_cannot_pass_as_success(self):
        self.assertFalse(self.check(final={"description": "invented"})["ok"])

    def test_fields_must_come_from_same_invocation(self):
        other = copy.deepcopy(self.step)
        self.step["observed_evidence"]["items"] = self.step["observed_evidence"]["items"][:2]
        other["observed_evidence"]["items"] = other["observed_evidence"]["items"][2:]
        self.assertFalse(self.check(steps=[self.step, other])["ok"])

    def test_malformed_final_and_contract_fail(self):
        self.assertFalse(self.check(final=[self.final])["ok"])
        for contract in ("null", "{}", '{"skill":"fixture_vision","success_fields":[]}'):
            self.assertFalse(self.check(contract=contract)["ok"])
        self.assertFalse(skill_outcome_json_assertion(self.contract, "{", {})["ok"])


if __name__ == "__main__":
    unittest.main()
