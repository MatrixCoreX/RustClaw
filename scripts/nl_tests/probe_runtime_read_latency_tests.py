import contextlib
import io
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from probe_runtime_read_latency import probe, timing_summary


class ReadProbeTests(unittest.TestCase):
    def test_remote_or_credential_bearing_targets_are_rejected_before_reading_keys(self):
        for url in ("https://example.com", "http://192.0.2.1", "http://user@localhost",
                    "http://localhost/elsewhere", "http://localhost?target=remote"):
            with self.subTest(url=url), self.assertRaisesRegex(ValueError, "loopback"):
                probe(Path("missing"), 1, url)

    def test_percentiles_do_not_claim_more_samples_than_measured(self):
        self.assertEqual(timing_summary([]), {"samples": 0})
        self.assertEqual(timing_summary([5]), {"samples": 1, "p50_ms": 5, "p95_ms": 5, "max_ms": 5})
        self.assertEqual(timing_summary(list(range(20)))["p95_ms"], 18)

    def test_probe_is_read_only_and_never_emits_credentials_or_task_bodies(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "configs").mkdir()
            (root / "configs/config.toml").write_text('[database]\nsqlite_path="tasks.sqlite"\n')
            with sqlite3.connect(root / "tasks.sqlite") as db:
                db.executescript("""CREATE TABLE auth_keys(user_key TEXT, role TEXT, enabled INTEGER);
                    CREATE TABLE tasks(task_id TEXT,status TEXT,result_json TEXT,lease_expires_at INTEGER,updated_at TEXT,created_at TEXT);
                    INSERT INTO auth_keys VALUES('private-test-key','admin',1);
                    INSERT INTO tasks VALUES('private-task-id','running','private-task-body',0,'1','1');""")
            before = (root / "tasks.sqlite").read_bytes()
            requests = []

            class Opener:
                def open(self, request, timeout):
                    requests.append(request)
                    return contextlib.closing(io.BytesIO(b'{"ok":true}'))

            with patch("urllib.request.build_opener", return_value=Opener()):
                report = probe(root, 2)
            self.assertEqual((root / "tasks.sqlite").read_bytes(), before)
            self.assertEqual(report["health"]["failures"], 0)
            self.assertEqual(report["recovery_query_offset_0"]["rows"], 1)
            self.assertEqual(report["recovery_query_offset_1024"]["rows"], 0)
            encoded = json.dumps(report)
            for private in ("private-test-key", "private-task-id", "private-task-body"):
                self.assertNotIn(private, encoded)
            self.assertEqual({request.full_url for request in requests}, {"http://127.0.0.1:8787/v1/health"})


if __name__ == "__main__":
    unittest.main()
