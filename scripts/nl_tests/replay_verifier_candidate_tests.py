import copy
import io
import json
import unittest
import urllib.error
from pathlib import Path
from unittest.mock import patch

from replay_verifier_candidate import matches_expectation, parse_verdict, revised_request, receive_response, render_full_template, compact_evidence_projection


class ReplayVerifierTests(unittest.TestCase):
    def test_compact_projection_preserves_distinct_data_and_original(self):
        data = {"extra":{"value":"x" * 5000}, "output":{"text":"kept", "extra":{"value":"x" * 5000}}}
        original = {"capability_result_evidence":[{"result":{"provenance":{"step_id":"s1"},"data":data}}]}
        result = compact_evidence_projection(original)
        item = result["capability_result_evidence"][0]
        self.assertEqual(item["step_id"], "s1")
        self.assertEqual(item["result"]["data"]["output"], {"text":"kept", "extra_reference":"data.extra"})
        self.assertIn("extra", data["output"])
        data["output"]["extra"] = {"different":True}
        preserved = compact_evidence_projection(original)["capability_result_evidence"][0]["result"]["data"]
        self.assertEqual(preserved, data)

    def test_outcome_only_check_requires_result_not_an_invented_method(self):
        check = {"requested_operation":"observe outcome", "evidence_step_ids":["s1"],
                 "required_dispatches":[], "method_observed":False, "result_observed":True}
        verdict = {"pass":True, "operation_checks":[check]}
        self.assertTrue(matches_expectation(verdict, True, None))
        check["method_observed"] = True
        self.assertFalse(matches_expectation(verdict, True, None))
        check["method_observed"] = False
        check["evidence_step_ids"] = []
        self.assertTrue(matches_expectation(verdict, True, None))

    def test_pass_boolean_cannot_override_contradictory_operation_checks(self):
        check = {"requested_operation":"conditional operation", "evidence_step_ids":[],
                 "required_dispatches":[{"action_type":"call_capability","action_ref":"fixture.verify"}],
                 "method_observed":False, "result_observed":False}
        verdict = {"pass":True, "operation_checks":[check]}
        self.assertFalse(matches_expectation(verdict, True, None))
        check.update(applicable=False, result_observed=True, evidence_step_ids=["s1"])
        self.assertTrue(matches_expectation(verdict, True, None))
        for field, value in (("applicable", True), ("blocked", True),
                             ("method_observed", True), ("result_observed", False),
                             ("evidence_step_ids", [])):
            invalid = copy.deepcopy(verdict)
            invalid["operation_checks"][0][field] = value
            self.assertFalse(matches_expectation(invalid, True, None))

    def test_schema_failure_is_not_an_accepted_verdict(self):
        schema = json.loads((Path(__file__).resolve().parents[2] /
                             "prompts/schemas/answer_verifier.schema.json").read_text())
        valid = {"operation_checks":[], "pass":True,"missing_evidence_fields":[],
                 "answer_incomplete_reason":"","should_retry":False,
                 "retry_instruction":"","confidence":0.9}
        self.assertTrue(matches_expectation(valid, True, None, schema))
        missing = dict(valid)
        del missing["operation_checks"]
        self.assertFalse(matches_expectation(missing, True, None, schema))
        valid["operation_checks"] = [{"requested_operation":"fixture", "evidence_step_ids":["s1"],
                                      "method_observed":True,"result_observed":True,"notes":"unallowed"}]
        self.assertFalse(matches_expectation(valid, True, None, schema))

    def test_full_template_keeps_data_placeholders_literal(self):
        source = ("User request:\n__OUTPUT_CONTRACT__\nRequest language hint:\nen\n"
                  "Evidence policy context:\npolicy\nOutput contract:\ncontract\n"
                  "Observed execution evidence:\nevidence\nCurrent task context:\ncontext\n"
                  "Agent/runtime identity:\n- The agent runtime identity is `fixture`.\n"
                  "Candidate final answer:\nanswer\nJudgment fields:\nold")
        rendered = render_full_template(source, "__USER_REQUEST__|__OUTPUT_CONTRACT__|__AGENT_RUNTIME_IDENTITY__|__EXECUTION_EVIDENCE__")
        self.assertEqual(rendered, "__OUTPUT_CONTRACT__|contract|fixture|evidence")
        with self.assertRaises(ValueError):
            render_full_template("unrelated", "__USER_REQUEST__")

    def test_quota_response_is_preserved_before_propagating_http_failure(self):
        raw = {"error":{"code":"quota_exhausted"}, "request_id":"fixture-request"}
        error = urllib.error.HTTPError("https://example.invalid",429,"quota",{},
                                       io.BytesIO(json.dumps(raw).encode()))
        log = io.StringIO('{"status":"requesting"}')
        with patch("urllib.request.urlopen", side_effect=error):
            with self.assertRaises(urllib.error.HTTPError):
                receive_response(object(), log, {"model":"fixture"})
        saved = json.loads(log.getvalue())
        self.assertEqual(saved["http_status"], 429)
        self.assertEqual(saved["raw_response"], raw)
        self.assertEqual(saved["status"], "provider_http_error")

    def test_operation_projection_is_bound_to_the_captured_task(self):
        record = {"task_id":"task-a", "request_payload":{"stream":False, "messages":[{
            "content":'Observed execution evidence:\n{"step_evidence":["unchanged"]}\nCurrent task context:\ncontext\nHard rejection checklist:\nold\nRules:\ntail'}]}}
        snapshot = {"data":{"task_id":"task-a", "result_json":{"task_journal":{"trace":{
            "step_results":[{"step_id":"step-1", "executed_skill":"fs_basic", "status":"ok",
                             "resolved_capability":"filesystem.stat_paths", "private":"not projected"}]
        }}}}}
        template = "Hard rejection checklist:\nnew\nRules:\n"
        payload = revised_request(record, template, execution_snapshot=snapshot)
        text = payload["messages"][0]["content"]
        evidence = json.loads(text.split("Observed execution evidence:\n",1)[1].split("\nCurrent task context:",1)[0])
        self.assertEqual(evidence["step_evidence"], ["unchanged"])
        self.assertEqual(evidence["executed_operations"]["operations"][0]["resolved_capability"], "filesystem.stat_paths")
        self.assertNotIn("private", text)
        snapshot["data"]["task_id"] = "other"
        with self.assertRaisesRegex(ValueError, "task_mismatch"):
            revised_request(record, template, execution_snapshot=snapshot)

    def test_expected_issue_is_explicit_and_exact(self):
        verdict = {"pass": False, "missing_evidence_fields": ["requested_result"]}
        self.assertTrue(matches_expectation(verdict, False, "requested_result"))
        self.assertFalse(matches_expectation(verdict, False, "unsupported_claims"))
        self.assertFalse(matches_expectation(verdict, True, "requested_result"))
        for value in ("requested_result", ["requested_result_extra"], None):
            self.assertFalse(matches_expectation(
                {"pass": False, "missing_evidence_fields": value}, False, "requested_result"))
        self.assertTrue(matches_expectation({"pass": True}, True, None))

    def test_verdict_payload_and_protocol_wrappers(self):
        for payload in ('{"pass":false}', '```json\n{"pass":false}\n```',
                        '```\n{"pass":false}\n```', '<think>private</think>{"pass":false}'):
            self.assertEqual(parse_verdict(payload), ('{"pass":false}', {"pass": False}))
        for payload in ('unknown', '{"pass":"true"}', '[]', 'answer: {"pass":false}'):
            with self.assertRaises(ValueError):
                parse_verdict(payload)

    def test_rules_change_preserves_evidence_candidate_and_transport(self):
        text = ("Evidence: actual-data\nCandidate final answer:\noriginal candidate"
                "\n\nJudgment fields:\ncontract\nHard rejection checklist:\nold rules\nRules:\ntail")
        record = {"request_payload": {"model": "fixture", "stream": False,
                  "messages": [{"role": "user", "content": text}]}}
        original = copy.deepcopy(record)
        result = revised_request(record, "Hard rejection checklist:\nnew rules\nRules:\nignored")
        self.assertEqual(result["messages"][0]["content"], text.replace("old rules", "new rules"))
        self.assertEqual(record, original)
        result = revised_request(record, "Hard rejection checklist:\nnew rules\nRules:\nignored", "corrected")
        self.assertEqual(result["messages"][0]["content"],
                         text.replace("old rules", "new rules").replace("original candidate", "corrected"))

    def test_incomplete_capture_rejected(self):
        for payload in ({"stream": True, "messages": []},
                        {"stream": False, "messages": [{"content": "wrong source"}]}):
            with self.assertRaises(ValueError):
                revised_request({"request_payload": payload}, "bad")

    def test_output_protocol_is_opt_in_and_preserves_captured_payload(self):
        record = {"request_payload":{"stream":False,"messages":[{
            "content":"Hard rejection checklist:\nold\nRules:\nevidence"}]}}
        result = revised_request(record, "Hard rejection checklist:\nnew\nRules:\n",
                                 output_protocol="fixture contract")
        self.assertTrue(result["messages"][0]["content"].endswith("Final verification output protocol:\nfixture contract"))
        self.assertNotIn("fixture contract", record["request_payload"]["messages"][0]["content"])


if __name__ == "__main__":
    unittest.main()
