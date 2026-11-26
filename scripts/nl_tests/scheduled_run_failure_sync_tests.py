"""Check execution errors independently of success or failure of notification."""
import json
from pathlib import Path
import sqlite3
import unittest

SQL = (Path(__file__).resolve().parents[2] / 'crates/clawd/src/scheduled_run_failure_sync.sql').read_text()

class ScheduledFailureTest(unittest.TestCase):
    def test_both_orders_and_restart(self):
        db = sqlite3.connect(':memory:')
        self.addCleanup(db.close)
        db.executescript("""
          PRAGMA recursive_triggers=ON;
          CREATE TABLE tasks(task_id TEXT PRIMARY KEY,status TEXT,result_json TEXT);
          CREATE TABLE scheduled_job_runs(task_id TEXT UNIQUE,task_status TEXT,result_json TEXT);
          INSERT INTO tasks VALUES ('first','failed',NULL),('other','succeeded','{}');
          INSERT INTO scheduled_job_runs VALUES ('first','failed','{}'),('other','succeeded','{}');
        """)
        db.executescript(SQL)
        for signal, attribution, expected in [
            ('repeat_action_limit','contract_gap','repeat_action_limit'),
            ('respond','provider_error','provider_error'),
            (None,'provider_error','provider_error'),
        ]:
            evidence = json.dumps({'task_journal':{'summary':{'final_stop_signal':signal,'final_failure_attribution':attribution}}})
            db.execute("UPDATE tasks SET result_json=? WHERE task_id='first'",(evidence,))
            db.execute("""UPDATE scheduled_job_runs SET result_json='{"notification":{"error_code":"delivery_failed"}}' WHERE task_id='first'""")
            result = json.loads(db.execute("SELECT result_json FROM scheduled_job_runs WHERE task_id='first'").fetchone()[0])
            self.assertEqual(result,{'error_code':expected,'notification':{'error_code':'delivery_failed'}})
            writes = db.total_changes
            db.executescript(SQL)
            self.assertEqual(writes, db.total_changes)
        self.assertEqual(db.execute("SELECT result_json FROM scheduled_job_runs WHERE task_id='other'").fetchone()[0],'{}')

if __name__ == '__main__':
    unittest.main()
