"""Typed finite-choice assertions must not confuse absence, types or JSON text."""
import json
from pathlib import Path
import unittest

from manual_case_assertions import evaluate_expectations


class JsonOneOfTests(unittest.TestCase):
    def check(self, text, expected='["error","failed"]', pointer="/status"):
        result = {"text": text}
        status, details = evaluate_expectations(
            f"result_text_json_one_of:{pointer}={expected}", "", {}, "succeeded", text, result, {})
        self.assertEqual(details[-1]["kind"], "result_text_json_one_of")
        return status

    def test_each_declared_value(self):
        for value in ("error", "failed"):
            self.assertEqual(self.check(json.dumps({"status": value})), "pass")

    def test_success_or_unknown_is_not_failure(self):
        for value in ("ok", "succeeded", "unknown", "ERROR", " failed ", False, 0, None):
            self.assertEqual(self.check(json.dumps({"status": value})), "fail")

    def test_missing_is_distinct_from_null(self):
        self.assertEqual(self.check("{}", "[null]"), "fail")
        self.assertEqual(self.check('{"status":null}', "[null]"), "pass")

    def test_strict_json_types(self):
        for actual, expected, wanted in ((True, "[1]", "fail"), (1, "[true]", "fail"),
                                         ("1", "[1]", "fail"), (1, "[1]", "pass"),
                                         (1.0, "[1]", "fail")):
            self.assertEqual(self.check(json.dumps({"status": actual}), expected), wanted)

    def test_invalid_or_empty_choices(self):
        for choices in ("[]", "{}", '"error"', "null", "true", "invalid", "[NaN]", '["error",NaN]'):
            self.assertEqual(self.check('{"status":"error"}', choices), "fail")

    def test_invalid_response_is_rejected(self):
        for text in ("not json", "", 'status="error"', '```json\n{"status":"error"}\n```'):
            self.assertEqual(self.check(text), "fail")

    def test_nested_pointer(self):
        self.assertEqual(self.check('{"outer":{"status":"failed"}}', pointer="/outer/status"), "pass")

    def test_exact_case_sensitive_pointer(self):
        self.assertEqual(self.check('{"Status":"failed"}'), "fail")

    def test_object_order_insignificant_array_order_preserved(self):
        self.assertEqual(self.check('{"status":{"b":2,"a":1}}', '[{"a":1,"b":2}]'), "pass")
        self.assertEqual(self.check('{"status":[2,1]}', '[[1,2]]'), "fail")

    def test_missing_separator_is_rejected(self):
        outcome, _ = evaluate_expectations("result_text_json_one_of:/status", "", {},
            "succeeded", '{"status":"error"}', {"text": '{"status":"error"}'}, {})
        self.assertEqual(outcome, "fail")

    def test_history_fixture_checks_a_finite_set_of_source_tokens(self):
        path = Path(__file__).parent / "cases/nl_cases_canonical_archive_20261029.txt"
        row = next(line for line in path.read_text().splitlines()
                   if line.startswith("canonical_archive|recall_archived_exit_code|"))
        expectation = row.split("|", 4)[4].removeprefix("expect=")
        clause = next(value for value in expectation.split(";")
                      if value.startswith("result_text_json_one_of:"))
        for source, wanted in (("retained_trace", "pass"), ("canonical_archive", "pass"),
                               ("model_memory", "fail"), ("not_selected", "fail"), (None, "fail")):
            text = json.dumps({"evidence_source": source})
            outcome, _ = evaluate_expectations(clause, "", {}, "succeeded", text, {"text": text}, {})
            self.assertEqual(outcome, wanted)


if __name__ == "__main__":
    unittest.main()
