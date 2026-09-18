#!/usr/bin/env python3
"""Acceptance reports must preserve failed evidence and revalidate new oracles."""
import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from live_suite_report import build_report, excerpt, replay
from shard_live_suite import accepted_prior_results, select_case_lines


class LiveSuiteReportTests(unittest.TestCase):
    def test_latest_current_failure_supersedes_prior_success_and_stale_success(self):
        attempts = [
            {"case_name": "a", "task_id": "old-pass", "current_prompt": True, "accepted": True,
             "successful_skills": ["fixture"], "successful_capabilities": ["fixture.read"]},
            {"case_name": "a", "task_id": "new-fail", "current_prompt": True, "accepted": False},
            {"case_name": "a", "task_id": "stale-pass", "current_prompt": False, "accepted": False},
        ]
        with patch("live_suite_report.case_index", return_value={"a": {}}), \
             patch("live_suite_report.read_attempts", return_value=attempts):
            result = build_report(Path("unused"), Path("unused"))
        self.assertEqual(result["attempt_count"], 3)
        self.assertEqual(result["accepted_distinct"], 0)
        self.assertEqual(result["failed_current"][0]["task_id"], "new-fail")
        self.assertEqual(result["successful_skills"], [])
        self.assertEqual(result["remaining"], ["a"])

    def test_resume_rechecks_latest_failure_regardless_of_directory_order(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            line = 'suite|sample||Inspect fixture|expect=result_text_json_eq:/verified=true'
            runs = []
            for number, verified in enumerate((True, False, True), 1):
                run = root / f"run{number}"
                case = run / "case_001_sample"
                case.mkdir(parents=True)
                result = {"ok": True, "data": {"status": "succeeded", "result_json": {
                    "text": json.dumps({"verified": verified})}}}
                (case / "final.json").write_text(json.dumps(result))
                row = {"source_line": 1, "case_name": "sample", "task_id": f"task{number}",
                       "prompt": "Inspect fixture", "status": "succeeded", "mode": "ask",
                       "started_at": number, "ended_at": number + 1}
                (run / "summary.jsonl").write_text(json.dumps(row) + "\n")
                runs.append(run)
            self.assertEqual(accepted_prior_results([line], runs[:2]), {})
            self.assertEqual(accepted_prior_results([line], list(reversed(runs[:2]))), {})
            self.assertEqual(accepted_prior_results([line], list(reversed(runs)))["sample"]["task_id"], "task3")

    def test_replay_checks_parsed_task_identity_and_ignores_partial_active_tail(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            model_log = root / "model.jsonl"
            rows = [
                {"task_id": "different", "prompt": "mentions wanted", "response": "not ours"},
                {"task_id": "wanted", "response": '{"status":"ok"}'},
                {"task_id": "child", "parent_task_id": "wanted", "response": '{"done":true}'},
            ]
            model_log.write_text("".join(json.dumps(row) + "\n" for row in rows) + '{"task_id":"wanted",')
            (root / "run.log").write_text(f"log_path={model_log}\n")
            result = replay({"run_dir": str(root), "task_id": "wanted", "result_text": "done"}, 200)
            self.assertEqual(result["recorded_llm_returns"], 2)
            self.assertEqual([row["task_id"] for row in result["llm_returns"]], ["wanted", "child"])
            self.assertEqual([row["number"] for row in result["llm_returns"]], [1, 2])

    def test_resume_partition_keeps_current_master_and_excludes_other_workers(self):
        with tempfile.TemporaryDirectory() as directory:
            prior = Path(directory) / "worker.txt"
            prior.write_text("# assigned\nsuite|a|tags|Old prompt\n")
            current = ["suite|a|new tags|Current prompt", "suite|b|tags|Another worker"]
            self.assertEqual(select_case_lines(current, [prior]), current[:1])
            self.assertEqual(select_case_lines(current, []), current)
            prior.write_text("suite|unknown|tags|Missing\n")
            with self.assertRaises(ValueError):
                select_case_lines(current, [prior])

    def test_replay_counts_prefixed_descendants_and_preserves_own_indexes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "model_io.log"
            rows = [
                {"task_id": "parent", "logical_call_index": 1, "clean_response": "first"},
                {"task_id": "parent:child:node:nonce", "logical_call_index": 1,
                 "model": "child-model", "clean_response": "second"},
                {"task_id": "parent-other:child:node:nonce", "logical_call_index": 1,
                 "clean_response": "unrelated parent"},
                {"task_id": "parent", "logical_call_index": 2, "clean_response": "third"},
            ]
            log.write_text("".join(json.dumps(row) + "\n" for row in rows))
            (root / "run.log").write_text(f"log_path={log}\n")
            result = replay({"run_dir": str(root), "task_id": "parent", "result_text": "done"}, 200)
            self.assertEqual(result["recorded_llm_returns"], 3)
            self.assertEqual(result["llm_call_counts"], {"task": 2, "descendants": 1, "recorded_total": 3})
            self.assertEqual([r["number"] for r in result["llm_returns"]], [1, 2, 3])
            self.assertEqual([r["logical_call_index"] for r in result["llm_returns"]], [1, 1, 2])

    def test_replay_reads_daily_archives_and_keeps_source_offsets(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            log = root / "model_io.log"
            archived = root / "model_io.log.2026-09-14"
            archived.write_text(json.dumps({"task_id": "wanted", "response": "first"}) + "\n")
            log.write_text(json.dumps({"task_id": "child", "parent_task_id": "wanted",
                                       "response": "second"}) + "\n")
            (root / "model_io.log.lock").write_text("not JSON")
            (root / "model_io.log.2026-99-99").write_text("not a date")
            (root / "run.log").write_text(f"log_path={log}\n")
            attempt = {"run_dir": str(root), "task_id": "wanted", "result_text": "done"}
            result = replay(attempt, 200)
            self.assertEqual(result["recorded_llm_returns"], 2)
            self.assertEqual([r["response_text"] for r in result["llm_returns"]], ["first", "second"])
            self.assertEqual([r["model_log"] for r in result["llm_returns"]], [str(archived), str(log)])
            self.assertEqual([r["row_offset"] for r in result["llm_returns"]], [0, 0])
            self.assertEqual([r["number"] for r in result["llm_returns"]], [1, 2])
            log.unlink()
            self.assertEqual(replay(attempt, 200)["recorded_llm_returns"], 1)

    def test_rechecks_prior_success_and_rejects_changed_prompts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            run = root / "run"
            case = run / "case_001_sample"
            case.mkdir(parents=True)
            result = {"ok": True, "data": {"status": "succeeded", "result_json": {
                "text": '{"verified":false}', "task_journal": {"trace": {"step_results": [
                    {"requested_action_type": "call_capability", "requested_capability": "fixture.read",
                     "resolved_capability": "fixture.read", "executed_skill": "fixture", "status": "ok"}
                ]}}}}}
            (case / "final.json").write_text(json.dumps(result))
            row = {"source_line": 1, "case_name": "sample", "task_id": "test-id", "prompt": "Inspect fixture",
                   "tags": "", "text": '{"verified":false}', "status": "succeeded", "mode": "ask",
                   "started_at": 1, "ended_at": 2, "wall_seconds": 1, "assertion": "pass"}
            (run / "summary.jsonl").write_text(json.dumps(row) + "\n")
            line = 'suite|sample|requires_tool_call=true|Inspect fixture|expect=result_text_json_eq:/verified=true'
            suite = root / "suite.txt"
            suite.write_text(line + "\n")
            report = build_report(root, suite)
            self.assertEqual(report["tested_distinct"], 1)
            self.assertEqual(report["accepted_distinct"], 0)
            self.assertEqual(report["attempts"][0]["recorded_assertion"], "pass")
            self.assertEqual(report["attempts"][0]["assertion"], "fail")
            self.assertEqual(report["attempts"][0]["current_expectation"], "result_text_json_eq:/verified=true")
            self.assertEqual(accepted_prior_results([line], [run]), {})
            result["data"]["result_json"]["text"] = '{"verified":true}'
            (case / "final.json").write_text(json.dumps(result))
            accepted_report = build_report(root, suite)
            self.assertEqual(accepted_report["accepted_distinct"], 1)
            self.assertEqual(accepted_report["attempts"][0]["acceptance_path"], "successful_execution")
            accepted = accepted_prior_results([line], [run])
            self.assertIn("sample", accepted)
            self.assertFalse(Path(accepted["sample"]["final_json"]).is_absolute())
            suite.write_text(line.replace("Inspect fixture", "Inspect another fixture") + "\n")
            self.assertEqual(build_report(root, suite)["accepted_distinct"], 0)

    def test_excerpt_retains_head_tail_and_length(self):
        self.assertEqual(excerpt("a" * 100 + "z" * 100, 20), {"chars": 200, "head": "a" * 10, "tail": "z" * 10})
        self.assertEqual(excerpt({"status": "ok"}, 20), {"status": "ok"})


if __name__ == "__main__":
    unittest.main()
