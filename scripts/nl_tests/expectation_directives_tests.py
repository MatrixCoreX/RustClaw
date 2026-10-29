"""JSON assertion literals must survive every runner boundary unchanged."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

from expectation_directives import separate_confirmation, split_expectation_directives
from manual_case_assertions import evaluate_expectations


ROOT = Path(__file__).resolve().parent


def run_embedded(script, function, *args):
    source = (ROOT / script).read_text()
    block = source.split(f"{function}() {{\n", 1)[1]
    code = block.split("<<'PY'\n", 1)[1].split("\nPY", 1)[0]
    return subprocess.run([sys.executable, "-", *map(str, args)], input=code,
                          capture_output=True, text=True, check=False)


class ExpectationDirectivesTests(unittest.TestCase):
    def test_ordinary_and_empty_segments(self):
        self.assertEqual(split_expectation_directives(" ; contains:one ; ;json_eq:/n=2;"),
                         ["contains:one", "json_eq:/n=2"])

    def test_plain_text_quotes_do_not_hide_a_separator(self):
        self.assertEqual(split_expectation_directives('contains:unclosed "quote;contains:two'),
                         ['contains:unclosed "quote', 'contains:two'])

    def test_nested_json_and_escaped_characters(self):
        for prefix in ("step_contract_json:", "skill_outcome_json:", "workspace_file_cycle:",
                       "result_text_json_records_eq:", "result_text_json_one_of:/x=",
                       "result_text_json_eq:/x=", "json_eq:/x="):
            payload = {"items": ['false; printf \'%s\' $?', 'quote";backslash\\;', {";": "; "}]}
            directive = prefix + json.dumps(payload)
            self.assertEqual(split_expectation_directives(directive + ";contains:done"),
                             [directive, "contains:done"])

    def test_string_and_array_json_values(self):
        for value in ("a; b", ["a; b", {"value": ";"}]):
            directive = "result_text_json_eq:/x=" + json.dumps(value)
            self.assertEqual(split_expectation_directives(directive + ";json_exists:/x"),
                             [directive, "json_exists:/x"])

    def test_confirmation_is_only_a_top_level_directive(self):
        directive = 'step_contract_json:{"command":"a; confirm:wrong; b"}'
        self.assertEqual(separate_confirmation(directive + ";confirm:yes"), (directive, "yes"))

    def test_malformed_json_and_trailing_garbage_fail_closed(self):
        for spec in ('step_contract_json:{"x":";contains:yes',
                     'result_text_json_eq:/x="broken;contains:yes',
                     'result_text_json_eq:/x={}garbage;contains:yes'):
            with self.assertRaises(ValueError):
                split_expectation_directives(spec)
            status, details = evaluate_expectations(spec, "", {}, "succeeded", "yes", {}, {})
            self.assertEqual(status, "fail")
            self.assertEqual(details[-1]["kind"], "invalid_expectation")

    def test_strict_string_numeric_comparison_is_retained(self):
        for expected, wanted in (('"1"', "pass"), ("1", "fail")):
            status, _ = evaluate_expectations("result_text_json_eq:/value=" + expected,
                "", {}, "succeeded", '{"value":"1"}', {"text": '{"value":"1"}'}, {})
            self.assertEqual(status, wanted)

    def test_exact_semicolon_value_is_checked_not_just_split(self):
        text = json.dumps({"value": "a; b"})
        for expected, wanted in (('"a; b"', "pass"), ('"a;b"', "fail")):
            status, _ = evaluate_expectations("result_text_json_eq:/value=" + expected,
                "", {}, "succeeded", text, {"text": text}, {})
            self.assertEqual(status, wanted)

    def test_both_case_loaders_preserve_payload_and_confirmation(self):
        expectation = 'step_contract_json:{"command":"false; printf \'%s\' $?"}'
        with tempfile.TemporaryDirectory() as directory:
            case = Path(directory) / "cases.txt"
            case.write_text("suite|case|requires_tool_call=true|request|expect=" +
                            expectation + ";confirm:yes\n")
            manual = run_embedded("run_manual_test.sh", "load_case_rows", case, ROOT)
            self.assertEqual(manual.returncode, 0, manual.stderr)
            self.assertEqual(manual.stdout.rstrip("\n").split("\x1f")[-2:], [expectation, "yes"])
            continuous = run_embedded("run_client_like_continuous_suite.sh", "load_case_rows",
                                     case, "0", "1", "", "", "0", "0", "0", "", ROOT)
            self.assertEqual(continuous.returncode, 0, continuous.stderr)
            self.assertIn(expectation, continuous.stdout)
            self.assertIn("yes", continuous.stdout)

    def test_case_loaders_do_not_emit_partial_rows_before_invalid_case(self):
        with tempfile.TemporaryDirectory() as directory:
            case = Path(directory) / "cases.txt"
            case.write_text('suite|valid||request|expect=contains:yes\n'
                            'suite|invalid||request|expect=step_contract_json:{"bad":";\n')
            for script, args in (
                ("run_manual_test.sh", (case, ROOT)),
                ("run_client_like_continuous_suite.sh",
                 (case, "0", "1", "", "", "0", "0", "0", "", ROOT)),
            ):
                result = run_embedded(script, "load_case_rows", *args)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, "")

    def test_continuous_result_checker_preserves_semicolon_json_value(self):
        with tempfile.TemporaryDirectory() as directory:
            result = Path(directory) / "result.json"
            result.write_text(json.dumps({"data": {"value": {"command": "a; b"}}}))
            for value, wanted in (("a; b", 0), ("a;b", 1)):
                checked = run_embedded("run_client_like_continuous_suite.sh", "result_text_contains",
                    result, "json_eq:/data/value=" + json.dumps({"command": value}, sort_keys=True,
                    separators=(",", ":")), ROOT)
                self.assertEqual(checked.returncode, wanted, checked.stderr)


if __name__ == "__main__":
    unittest.main()
