"""Offline audit rendering and least-privilege workflow contracts."""
from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("audit_report", ROOT / "scripts/report_rust_dependency_audit.py")
audit = importlib.util.module_from_spec(spec)
spec.loader.exec_module(audit)


def report():
    return {"vulnerabilities": {"found": False, "count": 0, "list": []}, "warnings": {}}


def entry(name="fixture", advisory="RUSTSEC-2026-0001"):
    return {"package": {"name": name, "version": "1.2.3"}, "advisory": {"id": advisory}}


class AuditReportTest(unittest.TestCase):
    def run_report(self, payload, exit_code):
        with tempfile.TemporaryDirectory(prefix="audit-report-fixture-") as temporary:
            root = Path(temporary)
            source, summary = root / "audit.json", root / "summary.md"
            source.write_text(payload if isinstance(payload, str) else json.dumps(payload))
            with redirect_stdout(io.StringIO()) as output:
                result = audit.main([str(source), "--audit-exit-code", str(exit_code), "--summary", str(summary)])
            return result, summary.read_text() if summary.exists() else "", output.getvalue()

    def test_official_success_and_all_warning_classes_remain_visible(self):
        payload = report()
        payload["warnings"] = {"unsound": [entry()], "unmaintained": [entry("old")],
                               "yanked": [{"package": {"name": "withdrawn", "version": "2"}, "advisory": None}]}
        result, summary, output = self.run_report(payload, 0)
        self.assertEqual(result, 0)
        self.assertIn("Vulnerabilities: 0", summary)
        self.assertIn("Warnings: 3", summary)
        for kind in payload["warnings"]:
            self.assertIn(kind, summary)
        self.assertIn("RUSTSEC-2026-0001", summary)
        self.assertIn("::warning", output)

    def test_audit_tool_failures_keep_the_original_exit_code(self):
        for code in (1, 7, 42):
            with self.subTest(code=code):
                result, summary, _ = self.run_report(report(), code)
                self.assertEqual(result, code)
                self.assertIn(f"exit code: {code}", summary)

    def test_vulnerabilities_are_blocking_even_if_tool_exit_code_is_inconsistent(self):
        payload = report()
        payload["vulnerabilities"] = {"found": True, "count": 1, "list": [entry()]}
        self.assertEqual(self.run_report(payload, 0)[0], 1)

    def test_malformed_missing_or_inconsistent_reports_fail_closed(self):
        for payload in ("not-json", {}, {"error": "database-fetch-failed"},
                        {"vulnerabilities": {"found": False, "count": 1, "list": []}, "warnings": {}},
                        {"vulnerabilities": report()["vulnerabilities"], "warnings": {"unsound": "invalid"}}):
            with self.subTest(payload=payload):
                result, summary, output = self.run_report(payload, 0)
                self.assertEqual(result, 2)
                self.assertEqual(summary, "")
                self.assertIn("::error", output)

    def test_advisory_fields_cannot_inject_html_or_workflow_commands(self):
        payload = report()
        payload["warnings"] = {"notice": [entry("<script>|\n::error::fixture")]}
        _, summary, output = self.run_report(payload, 0)
        self.assertNotIn("<script>", summary)
        self.assertIn("&lt;script&gt;\\|", summary)
        self.assertFalse(any(line.startswith("::error") for line in output.splitlines()))

    def test_workflow_uses_official_pinned_cli_without_write_permissions_or_issue_tokens(self):
        text = (ROOT / ".github/workflows/security-audit.yml").read_text()
        self.assertIn("cargo install cargo-audit --version 0.22.2 --locked", text)
        self.assertIn('cargo audit --json --file "$GITHUB_WORKSPACE/Cargo.lock"', text)
        self.assertIn("working-directory: ${{ runner.temp }}", text)
        self.assertIn('audit_exit=$?', text)
        self.assertIn('--audit-exit-code "$audit_exit"', text)
        self.assertIn("if: ${{ always() }}", text)
        self.assertNotIn("issues: write", text)
        self.assertNotIn("checks: write", text)
        self.assertNotIn("secrets.GITHUB_TOKEN", text)
        self.assertNotIn("rustsec/audit-check@", text)
        self.assertNotIn("--ignore", text)
        parent = (ROOT / ".github/workflows/macos-polyglot-skill-ci.yml").read_text()
        for name in (".github/workflows/security-audit.yml", "scripts/report_rust_dependency_audit.py",
                     "scripts/tests/test_rust_dependency_audit.py"):
            self.assertEqual(parent.count(f'"{name}"'), 2)
        child = (ROOT / ".github/workflows/media-session-portability-ci.yml").read_text()
        self.assertIn("-p test_rust_dependency_audit.py", child)


if __name__ == "__main__":
    unittest.main()
