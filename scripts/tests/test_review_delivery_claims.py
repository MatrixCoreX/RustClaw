import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "review_delivery_claims.py"


class DeliveryReview(unittest.TestCase):
    def test_confirmation_is_snapshot_bound_and_never_changes_dispatch(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / 'test.db'
            db = sqlite3.connect(path)
            db.executescript("""
            CREATE TABLE channel_delivery_dispatch_claims(idempotency_key TEXT,delivery_id TEXT,channel TEXT,
                state TEXT,attempt_count INTEGER,updated_at_ts INTEGER,lease_expires_at_ts INTEGER);
            CREATE TABLE channel_delivery_receipts(idempotency_key TEXT,delivery_id TEXT,channel TEXT,status TEXT,receipt_digest TEXT);
            INSERT INTO channel_delivery_dispatch_claims VALUES('k','d','wechat','query_required',3,20,15);
            """)
            command = [sys.executable,str(SCRIPT),str(path)]
            [claim] = json.loads(subprocess.check_output(command))
            args = ['--confirm-received','d','--snapshot-digest',claim['snapshot_digest'],'--evidence-ref','recipient-confirmation']
            for _ in range(2):
                [item] = json.loads(subprocess.check_output(command+args))
                self.assertEqual(item['review_status'],'operator_confirmed')
            self.assertEqual(db.execute('select count(*) from channel_delivery_operator_reviews').fetchone()[0],1)
            self.assertEqual(db.execute('select state,attempt_count from channel_delivery_dispatch_claims').fetchone(),('query_required',3))
            db.execute("insert into channel_delivery_receipts values('k','d','wechat','delivered','receipt')"); db.commit()
            self.assertEqual(json.loads(subprocess.check_output(command)),[])
            self.assertNotEqual(subprocess.run(command+args,capture_output=True).returncode,0)
            db.close()

if __name__ == "__main__":
    unittest.main()
