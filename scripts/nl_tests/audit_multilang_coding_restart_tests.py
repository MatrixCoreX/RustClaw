import hashlib
import json
import platform
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from audit_multilang_coding_restart import CASE_NAMES, audit, project_commands, initial_implementations, run_validator


class CodingAcceptanceTests(unittest.TestCase):
    @unittest.skipUnless(platform.system() == "Linux" and shutil.which("bwrap"), "isolated Linux validator unavailable")
    def test_noisy_generated_validation_cannot_allocate_unbounded_captured_output(self):
        with tempfile.TemporaryDirectory() as temp:
            command = project_commands(Path(temp))[0]
            command = command[:command.index("--") + 1] + ["/usr/bin/python3", "-c", "print('x'*200000)"]
            with self.assertRaisesRegex(ValueError, "output_budget_exceeded"):
                run_validator(command)

    @unittest.skipUnless(platform.system() == "Linux" and shutil.which("bwrap"), "isolated Linux validator unavailable")
    def test_real_read_only_sandbox_can_verify_python_and_node_without_host_secrets(self):
        with tempfile.TemporaryDirectory() as temp:
            project = Path(temp)
            (project / "pricing.py").write_text("def price_total(values):\n    return sum(value for value in values if value >= 0)\n")
            (project / "pricing.mjs").write_text("export const price_total = values => values.filter(value => value >= 0).reduce((total,value) => total+value,0);\n")
            (project / "test_pricing.py").write_text("import unittest\nimport pricing\nclass PricingTests(unittest.TestCase):\n    def test_total(self):\n        self.assertEqual(pricing.price_total([2,-3,4]), 6)\n")
            (project / "test_pricing.mjs").write_text("import {price_total} from './pricing.mjs'; import assert from 'node:assert/strict'; assert.equal(price_total([2,-3,4]),6);\n")
            for command in project_commands(project):
                result = subprocess.run(command, text=True, capture_output=True, timeout=60)
                self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse((project / "__pycache__").exists())

    def fixture(self, root):
        project = root / "workspace/tmp/multilang_project_20261029"
        project.mkdir(parents=True)
        for name in ("pricing.py", "pricing.mjs", "test_pricing.py", "test_pricing.mjs"):
            (project / name).write_text("fixture")
        (project / "red_phase.json").write_text(json.dumps({"command":"python3 test_pricing.py", "returncode":1, "stdout":"", "stderr":"assertion failed"}))
        for index, name in enumerate(CASE_NAMES):
            run = root / f"results/run{index}"
            case = run / f"case_1_{name}"
            case.mkdir(parents=True)
            (run / "summary.jsonl").write_text(json.dumps({"case_name":name,"status":"succeeded","assertion":"pass","task_id":str(index)}))
            capabilities = [{"capability":capability,"status":"ok"} for capability in
                            ("code.find_definitions","code.find_references","code.list_tests")]
            if index == 1:
                capabilities.append({"capability":"task_plan.read","status":"ok"})
            if index == 0:
                capabilities = []
                for filename, content in (("pricing.py", "def price_total(values): return sum(values)\n"),
                                          ("pricing.mjs", "export const price_total = values => values.reduce((a,b)=>a+b,0);\n")):
                    raw = content.encode()
                    capabilities.append({"capability":"filesystem.write_file","status":"ok",
                        "provenance":{"continuation_args":{"content":content},"continuation_args_projection":{"truncated":False}},
                        "data":{"output":{"after":[{"path":"tmp/multilang_project_20261029/"+filename,
                            "size_bytes":len(raw),"sha256":"sha256:"+hashlib.sha256(raw).hexdigest()}]}}})
            (case / "final.json").write_text(json.dumps({"data":{"result_json":{"task_journal":{"trace":{"capability_results":capabilities}}}}}))
        (root / "workspace/logs").mkdir()
        (root / "workspace/logs/model_io.log").write_text("\n".join(json.dumps({"task_id":str(index)}) for index in range(4)))
        (root / "restart-evidence.json").write_text(json.dumps({"after_case":"index_project","old_pid":100,"new_pid":200}))

    def test_complete_journals_require_independent_sandbox_verification(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.fixture(root)
            with patch("audit_multilang_coding_restart.project_commands", return_value=[["fixture-sandbox"]]), \
                 patch("audit_multilang_coding_restart.subprocess.run", side_effect=[
                     subprocess.CompletedProcess([],1,"","red"), subprocess.CompletedProcess([],0,"ok","")]) as runner:
                self.assertEqual(audit(root)["status"], "passed")
                self.assertEqual(runner.call_count, 2)

    def test_red_validation_rejects_self_report_without_actual_failing_tests(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.fixture(root)
            with patch("audit_multilang_coding_restart.project_commands", return_value=[["fixture-sandbox"]]), \
                 patch("audit_multilang_coding_restart.subprocess.run", return_value=subprocess.CompletedProcess([],0,"","")), \
                 self.assertRaisesRegex(ValueError, "do_not_reject_initial"):
                audit(root)

    def test_initial_implementation_requires_host_output_digest_and_complete_arguments(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.fixture(root)
            original = json.loads((root / "results/run0/case_1_create_project/final.json").read_text())["data"]["result_json"]["task_journal"]["trace"]
            self.assertEqual(set(initial_implementations(original)), {"pricing.py", "pricing.mjs"})
            for defect in ("hash", "size", "truncated", "path", "missing"):
                value = json.loads(json.dumps(original))
                item = value["capability_results"][0]
                if defect == "hash": item["data"]["output"]["after"][0]["sha256"] = "sha256:" + "0"*64
                elif defect == "size": item["data"]["output"]["after"][0]["size_bytes"] = 0
                elif defect == "truncated": item["provenance"]["continuation_args_projection"]["truncated"] = True
                elif defect == "path": item["data"]["output"]["after"][0]["path"] = "other/pricing.py"
                else: value["capability_results"] = []
                with self.subTest(defect=defect), self.assertRaises(ValueError):
                    initial_implementations(value)

    def test_false_red_phase_or_missing_execution_or_restart_never_passes(self):
        for defect in ("red", "index", "index_write", "restart", "independent"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as temp:
                root = Path(temp)
                self.fixture(root)
                if defect == "red":
                    (root / "workspace/tmp/multilang_project_20261029/red_phase.json").write_text(json.dumps({"returncode":0}))
                elif defect in ("index", "index_write"):
                    path = root / "results/run1/case_1_index_project/final.json"
                    value = json.loads(path.read_text())
                    caps = value["data"]["result_json"]["task_journal"]["trace"]["capability_results"]
                    if defect == "index": caps.clear()
                    else: caps.append({"capability":"filesystem.write_file","status":"ok"})
                    path.write_text(json.dumps(value))
                elif defect == "restart":
                    (root / "restart-evidence.json").write_text(json.dumps({"after_case":"index_project","old_pid":100,"new_pid":100}))
                with patch("audit_multilang_coding_restart.project_commands", return_value=[["fixture-sandbox"]]), \
                     patch("audit_multilang_coding_restart.subprocess.run", return_value=subprocess.CompletedProcess([],1,"","failed")), \
                     self.assertRaises((AssertionError, ValueError)):
                    audit(root)

    def test_absent_sandbox_never_runs_generated_code_on_host(self):
        with patch("audit_multilang_coding_restart.platform.system", return_value="Darwin"), \
             self.assertRaisesRegex(ValueError, "sandbox_unavailable"):
            project_commands(Path("/workspace"))
        with patch("audit_multilang_coding_restart.platform.system", return_value="Linux"), \
             patch("audit_multilang_coding_restart.shutil.which", return_value="/usr/bin/bwrap"):
            for command in project_commands(Path("/fixture")):
                self.assertEqual(command[0], "bwrap")
                self.assertIn("--unshare-all", command)
                self.assertIn("--clearenv", command)
                self.assertNotIn("--bind", command)


if __name__ == "__main__":
    unittest.main()
