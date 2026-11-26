"""Exercise both receipt/result arrival orders, exact binding and repeated startup."""
import json
from pathlib import Path
import sqlite3
import unittest

SQL = (Path(__file__).resolve().parents[2] / 'crates/clawd/src/scheduled_run_receipt_sync.sql').read_text()


class ScheduleReceiptTests(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        self.addCleanup(self.db.close)
        self.db.executescript("""PRAGMA recursive_triggers=ON;
            CREATE TABLE scheduled_job_runs(task_id TEXT PRIMARY KEY, task_status TEXT, result_json TEXT);
            CREATE TABLE channel_delivery_receipts(idempotency_key TEXT PRIMARY KEY, delivery_id TEXT,channel TEXT,status TEXT,receipt_json TEXT);""")
        self.pending = json.dumps({'unrelated':1,'notification':{'runtime_channel':'wechat',
                                  'delivery_status':'query_required','error_code':'pending'}})

    def receipt(self, key='task:schedule-terminal', status='failed', retry=False, channel='wechat'):
        self.db.execute("""INSERT INTO channel_delivery_receipts VALUES(?,?,?,?,?)
            ON CONFLICT(idempotency_key) DO UPDATE SET status=excluded.status,receipt_json=excluded.receipt_json""",
            (key,'delivery:'+key,channel,status,json.dumps({'retryable':retry, 'error_code':'provider_timeout' if status=='failed' else None})))

    def read(self):
        status, raw = self.db.execute('SELECT task_status,result_json FROM scheduled_job_runs').fetchone()
        self.assertEqual(status,'succeeded')
        self.assertEqual(json.loads(raw)['unrelated'],1)
        return json.loads(raw)['notification']

    def test_backfill_and_late_receipts(self):
        self.db.execute("INSERT INTO scheduled_job_runs VALUES('task','succeeded',?)",(self.pending,))
        self.receipt()
        self.db.executescript(SQL)
        self.assertEqual(self.read()['error_code'],'provider_timeout')
        writes=self.db.total_changes
        self.db.executescript(SQL)
        self.assertEqual(self.db.total_changes,writes)
        self.receipt(status='accepted')
        self.assertEqual(self.read()['delivery_status'],'accepted')
        self.assertFalse(self.read()['delivered'])
        self.assertNotIn('error_code',self.read())
        self.receipt(status='read')
        self.assertTrue(self.read()['delivered'])

    def test_receipt_before_result_and_unrelated_receipts(self):
        self.db.executescript(SQL)
        self.db.execute("INSERT INTO scheduled_job_runs VALUES('task','succeeded','{}')")
        self.receipt('other:schedule-terminal')
        self.receipt('task:terminal',status='accepted')
        self.receipt(status='partial')
        self.db.execute('UPDATE scheduled_job_runs SET result_json=?',(self.pending,))
        self.assertEqual(self.read()['delivery_status'],'query_required')
        self.receipt(status='failed',retry=True)
        self.assertEqual(self.read()['delivery_status'],'query_required')
        self.receipt(status='delivered')
        self.db.execute('UPDATE scheduled_job_runs SET result_json=?',(self.pending,))
        self.assertTrue(self.read()['accepted'])
        self.assertTrue(self.read()['delivered'])


if __name__ == '__main__':
    unittest.main()
