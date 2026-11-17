"""Exercise the CI-built offline utility against real SQLite/WAL files."""
import json
import importlib.util
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest


class EventStorageMaintenance(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.db = self.root / "original.db"
        self.binary = os.environ["EVENT_MAINTENANCE_BIN"]
        with sqlite3.connect(self.db) as db:
            db.executescript("""
                PRAGMA journal_mode=WAL;
                CREATE TABLE tasks(task_id TEXT PRIMARY KEY,status TEXT,payload_json TEXT,result_json TEXT);
                CREATE TABLE task_event_stream(task_id TEXT,seq INTEGER,event_hash TEXT,event_json TEXT,created_at_ms INTEGER,PRIMARY KEY(task_id,seq));
                CREATE TABLE task_event_archive(task_id TEXT,seq INTEGER,event_hash TEXT,event_json TEXT,created_at_ms INTEGER,PRIMARY KEY(task_id,seq));
                CREATE TABLE task_event_artifacts(task_id TEXT,artifact_id TEXT,payload_json TEXT,payload_bytes INTEGER,PRIMARY KEY(task_id,artifact_id));
                CREATE TABLE task_event_snapshots(task_id TEXT,snapshot_seq INTEGER,snapshot_json TEXT,PRIMARY KEY(task_id,snapshot_seq));
                INSERT INTO tasks VALUES('finished','succeeded','{"text":"preserved"}','{"ok":true}');
                INSERT INTO tasks VALUES('active','running','{"text":"preserved"}',NULL);
            """)
            for table in ("memories", "long_term_memories", "user_preferences", "auth_keys", "credential_bindings", "principals"):
                db.execute(f"CREATE TABLE {table}(id INTEGER PRIMARY KEY,value TEXT)")
                db.execute(f"INSERT INTO {table} VALUES(1,?)", ("preserve 中🙂\0",))
            for seq in range(1, 257):
                raw = json.dumps({"task_id": "finished", "seq": seq, "event_kind": "tool_finished", "payload": {"skill": "example", "requested_action_ref": "example.read", "data": "evidence 中🙂" * 4000}}, ensure_ascii=False)
                db.execute("INSERT INTO task_event_stream VALUES(?,?,?,?,1)", ("finished", seq, str(seq), raw))
                db.execute("INSERT INTO task_event_archive VALUES(?,?,?,?,1)", ("finished", seq, str(seq), raw))
            raw = json.dumps({"data": "original evidence" * 10000})
            db.execute("INSERT INTO task_event_artifacts VALUES('finished','artifact',?,?)", (raw, len(raw)))

    def run_tool(self, action, path=None, *extra):
        result = subprocess.run([self.binary, action, "--database", str(path or self.db), *map(str, extra)], text=True, capture_output=True, timeout=180, check=True)
        return json.loads(result.stdout.splitlines()[-1])

    def test_resume_roundtrip_prune_and_physical_compaction(self):
        probe_path = Path(__file__).resolve().parents[1] / "large_db_memory_probe.py"
        spec = importlib.util.spec_from_file_location("probe", probe_path)
        probe = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(probe)
        with sqlite3.connect(self.db) as db:
            expected_events = probe.event_page(db, "finished")
        before = self.run_tool("verify")
        self.run_tool("prepare")
        process = subprocess.Popen([self.binary, "pack", "--database", str(self.db), "--batch-size", "1"], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            self.assertTrue(json.loads(process.stdout.readline())["ok"])
            process.kill()
        finally:
            process.communicate(timeout=30)
        self.assertEqual(self.run_tool("pack")["pending_rows"], 0)
        self.assertEqual(self.run_tool("verify"), before)
        with sqlite3.connect(self.db) as db:
            self.assertEqual(probe.event_page(db, "finished"), expected_events)
        self.assertEqual(self.run_tool("pack")["total_rows"], 0)
        pruned = self.run_tool("prune", None, "--before-ms", 100)
        self.assertEqual(pruned["total_rows"], 256)
        self.assertEqual(self.run_tool("verify"), before)
        compact = self.root / "compact.db"
        self.run_tool("vacuum", None, "--output", compact)
        self.assertEqual(self.run_tool("verify", compact), before)
        self.assertLess(compact.stat().st_size, self.db.stat().st_size // 3)
        with sqlite3.connect(compact) as db:
            self.assertEqual(db.execute("PRAGMA integrity_check").fetchone()[0], "ok")

    def test_vacuum_refuses_existing_file_and_corruption_is_not_hidden(self):
        existing = self.root / "keep"
        existing.write_bytes(b"preserved")
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_tool("vacuum", None, "--output", existing)
        self.assertEqual(existing.read_bytes(), b"preserved")
        self.run_tool("pack")
        with sqlite3.connect(self.db) as db:
            db.execute("UPDATE task_event_archive SET storage_blob=zeroblob(length(storage_blob)) WHERE seq=1")
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_tool("verify")
        with self.assertRaises(subprocess.CalledProcessError):
            self.run_tool("prune", None, "--before-ms", 100)
        with sqlite3.connect(self.db) as db:
            self.assertEqual(db.execute("SELECT count(*) FROM task_event_stream").fetchone()[0], 256)


if __name__ == "__main__":
    unittest.main()
