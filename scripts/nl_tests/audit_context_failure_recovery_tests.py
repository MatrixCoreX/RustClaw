import copy
import json
from pathlib import Path
import tempfile
import unittest

from audit_context_failure_recovery import audit


class AcceptanceTests(unittest.TestCase):
    def fixture(self, root):
        run = root / "results/run"
        run.mkdir(parents=True)
        cases = ["seed_two_reads_en", "stale_cursor_read_zh", "failure_json_en"]
        (run / "summary.jsonl").write_text("\n".join(json.dumps({"case_name":name,"task_id":str(i),"assertion":"pass"}) for i, name in enumerate(cases)))
        error = {"code":"context_cursor_invalid","details":{"structured_error":{"extra":{
            "retryable":True,"side_effect_applied":False,"failure_phase":"pre_dispatch","recovery_action":"replan_arguments"}}}}
        observed = {"source_task_id":"0","instruction_authority":"none","current_execution_evidence":False,
                    "pagination":{"offset":0,"total":2}}
        streams = [
            [{"capability":"filesystem.read_text_range","status":"ok"}] * 2,
            [{"capability":"task_control.recall_context","status":"error","error":error},
             {"capability":"task_control.recall_context","status":"ok","data":{"extra":observed}}],
            [{"capability":"task_control.recall_context","status":"error","error":{"code":"context_field_unavailable"}}],
        ]
        paths = []
        for i, (name, stream) in enumerate(zip(cases, streams)):
            directory = run / f"case_{i}_{name}"
            directory.mkdir()
            path = directory / "final.json"
            path.write_text(json.dumps({"data":{"status":"failed" if i == 2 else "succeeded",
                "result_json":{"text":json.dumps({"status":"error","error_code":"context_field_unavailable","value":None}),
                               "task_journal":{"trace":{"capability_results":stream}}}}}))
            paths.append(path)
        logs = root / "workspace/logs"
        logs.mkdir(parents=True)
        args = {"context_ref":"task-context:v1:0","field_paths":["/capabilities"]}
        (logs / "model_io.log").write_text("\n".join(json.dumps({"task_id":"1","response":json.dumps({"tool_calls":[
            {"name":"call_task_control_recall_context","arguments":item}]})}) for item in [
                {**args,"cursor":"expired.fixture"}, args]))
        return paths

    def test_exact_negative_and_recovery_evidence_passes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            self.fixture(root)
            self.assertEqual(audit(root)["status"], "passed")
            with self.assertRaises(AssertionError):
                audit(root, require_canonical=True)

    def test_missing_proof_wrong_source_and_replayed_mutations_never_pass(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            paths = self.fixture(root)
            original = json.loads(paths[1].read_text())
            for kind in ["retry", "scope", "mutation"]:
                wrong = copy.deepcopy(original)
                stream = wrong["data"]["result_json"]["task_journal"]["trace"]["capability_results"]
                if kind == "retry":
                    stream[0]["error"]["details"]["structured_error"]["extra"]["retryable"] = False
                elif kind == "scope":
                    stream[1]["data"]["extra"]["source_task_id"] = "different-owner-task"
                else:
                    stream[1]["capability"] = "filesystem.write_file"
                paths[1].write_text(json.dumps(wrong))
                with self.assertRaises(AssertionError, msg=kind):
                    audit(root)
            paths[1].write_text(json.dumps(original))
            wrong = json.loads(paths[2].read_text())
            wrong["data"]["status"] = "succeeded"
            paths[2].write_text(json.dumps(wrong))
            with self.assertRaises(AssertionError):
                audit(root)


if __name__ == "__main__":
    unittest.main()
