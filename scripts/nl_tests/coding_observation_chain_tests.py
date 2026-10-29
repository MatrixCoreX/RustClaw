"""Resumed acceptance supplements a waiting attempt; it never erases a failure."""
import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import audit_multilang_coding_restart_tests as fixtures
from audit_multilang_coding_restart import CASE_NAMES, audit
from manual_case_observation import bind_case


def digest_files(root):
    return {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
            for path in root.rglob("*") if path.is_file()}


def fixture(root):
    fixtures.CodingAcceptanceTests().fixture(root)
    for index, name in enumerate(CASE_NAMES):
        run = root / f"results/run{index}"
        case = run / f"case_1_{name}"
        task_id = f"00000000-0000-4000-8000-{index:012d}"
        row = json.loads((run / "summary.jsonl").read_text())
        row.update(task_id=task_id, source_line=index + 1, tags="requires_tool_call=false",
                   prompt=f"fixture {name}", mode="ask", expect_substr="json_eq:/data/status=succeeded",
                   started_at=index * 20 + 1, ended_at=index * 20 + 5)
        (run / "summary.jsonl").write_text(json.dumps(row) + "\n")
        final = json.loads((case / "final.json").read_text())
        final["ok"] = True
        final["data"].update(task_id=task_id, status="succeeded")
        (case / "final.json").write_text(json.dumps(final))
        (case / "submit.json").write_text(json.dumps({"ok": True, "data": {"task_id": task_id}}))
    source = root / "results/run2"
    case = source / "case_1_amend_after_restart"
    row = json.loads((source / "summary.jsonl").read_text())
    observed = {**row, "started_at": 50, "ended_at": 55}
    successful_final = json.loads((case / "final.json").read_text())
    row.update(status="provider_unavailable", assertion="fail")
    (source / "summary.jsonl").write_text(json.dumps(row) + "\n")
    waiting = copy.deepcopy(successful_final)
    waiting["data"].update(status="running", lifecycle={"state": "waiting",
        "provider_blocker_active": True, "provider_status": {"external_provider_blocked": True}})
    (case / "final.json").write_text(json.dumps(waiting))
    run = root / "observations/run0"
    supplemental = run / "case_1_amend_after_restart"
    supplemental.mkdir(parents=True)
    (run / "summary.jsonl").write_text(json.dumps(observed) + "\n")
    (supplemental / "final.json").write_text(json.dumps(successful_final))
    bind_case(case, supplemental / "observation.json", row["source_line"], row["case_name"],
              row["tags"], row["prompt"], row["expect_substr"])
    return run, case, supplemental


def mocked_audit(root, observations):
    with patch("audit_multilang_coding_restart.project_commands", return_value=[["fixture-sandbox"]]), \
         patch("audit_multilang_coding_restart.subprocess.run", side_effect=[
             subprocess.CompletedProcess([], 1, "", "red"),
             subprocess.CompletedProcess([], 0, "ok", "")]):
        return audit(root, observation_runs=observations)


class ObservationChainTests(unittest.TestCase):
    def test_same_task_success_is_supplemental_and_original_wait_is_preserved(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run, source, _ = fixture(root)
            original = digest_files(root / "results")
            result = mocked_audit(root, [run])
            self.assertEqual(result["status"], "passed")
            self.assertEqual(result["cases"], 4)
            self.assertEqual(result["attempt_rows"], 5)
            self.assertEqual(result["observations"][0]["original_status"], "provider_unavailable")
            self.assertEqual(result["observations"][0]["task_id"],
                             json.loads((source.parent / "summary.jsonl").read_text())["task_id"])
            self.assertEqual(digest_files(root / "results"), original)
            self.assertFalse(result["independent_red_suite"]["historical_probe_chronology_verified"])

    def test_default_audit_does_not_silently_select_an_observation(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture(root)
            with self.assertRaisesRegex(ValueError, "case_not_accepted"):
                audit(root)

    def test_wrong_task_contract_or_digest_never_replaces_the_original(self):
        for defect in ("task_id", "prompt", "source_digest", "final_failed", "not_accepted",
                       "earlier_than_wait", "later_than_next_turn", "source_outside"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                run, source, case = fixture(root)
                summary = run / "summary.jsonl"
                row = json.loads(summary.read_text())
                if defect in ("task_id", "prompt", "not_accepted", "earlier_than_wait", "later_than_next_turn"):
                    if defect == "task_id": row["task_id"] = "00000000-0000-4000-8000-000000000099"
                    elif defect == "prompt": row["prompt"] = "a different task"
                    elif defect == "not_accepted": row["status"] = "provider_unavailable"
                    elif defect == "earlier_than_wait": row["started_at"] = 40
                    else: row["ended_at"] = 65
                    summary.write_text(json.dumps(row) + "\n")
                elif defect == "source_digest":
                    (source / "final.json").write_text((source / "final.json").read_text() + " ")
                elif defect == "final_failed":
                    file = case / "final.json"
                    final = json.loads(file.read_text())
                    final["data"]["status"] = "failed"
                    file.write_text(json.dumps(final))
                else:
                    file = case / "observation.json"
                    proof = json.loads(file.read_text())
                    proof["source_case"] = str(root / "outside")
                    file.write_text(json.dumps(proof))
                with self.assertRaises(ValueError):
                    mocked_audit(root, [run])

    def test_terminal_failures_missing_turns_and_duplicate_observations_never_pass(self):
        for defect in ("failed", "succeeded", "missing_turn", "duplicate"):
            with self.subTest(defect=defect), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                run, source, supplemental = fixture(root)
                if defect in ("failed", "succeeded"):
                    file = source.parent / "summary.jsonl"
                    row = json.loads(file.read_text())
                    row["status"] = defect
                    file.write_text(json.dumps(row) + "\n")
                    final = json.loads((source / "final.json").read_text())
                    final["data"]["status"] = defect
                    (source / "final.json").write_text(json.dumps(final))
                    proof = supplemental / "observation.json"
                    proof.unlink()
                    bind_case(source, proof, row["source_line"], row["case_name"], row["tags"],
                              row["prompt"], row["expect_substr"])
                elif defect == "missing_turn":
                    (root / "results/run3/summary.jsonl").unlink()
                with self.assertRaises(ValueError):
                    mocked_audit(root, [run, run] if defect == "duplicate" else [run])


if __name__ == "__main__":
    unittest.main()
