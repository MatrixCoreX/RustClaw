import hashlib
import json
import os
from pathlib import Path
import sqlite3
import sys
import tempfile
import unittest
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import deduplicate_task_deliveries as maintenance


class DeliveryStorageTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name).resolve()
        (self.root / 'configs').mkdir()
        (self.root / 'data').mkdir()
        (self.root / 'configs/config.toml').write_text('[database]\nsqlite_path="data/runtime.db"\n')
        executable = self.root / 'target/release/clawd'
        executable.parent.mkdir(parents=True)
        executable.write_bytes(b'fixture executable')
        self.db = sqlite3.connect(self.root / 'data/runtime.db')
        self.addCleanup(self.db.close)
        self.db.execute('CREATE TABLE tasks(task_id TEXT PRIMARY KEY, principal_id TEXT, status TEXT, result_json TEXT)')
        self.payload = b'media bytes' * 10000
        self.audit = []

    def artifact(self, *, principal='owner', status='succeeded', expected=None):
        task = str(uuid.uuid4())
        path = self.root / '.agent-runtime/artifacts/delivery' / task / 'artifact-one' / 'movie.bin'
        path.parent.mkdir(parents=True)
        path.write_bytes(self.payload)
        sha = hashlib.sha256(self.payload).hexdigest()
        manifest = {'schema_version': 2, 'id': 'artifact-one', 'filename': path.name,
                    'artifact_ref': 'artifact:task/' + task + '/artifact-one',
                    'size_bytes': len(self.payload), 'sha256': expected or sha}
        self.db.execute('INSERT INTO tasks VALUES(?,?,?,?)', (task, principal, status, json.dumps({'artifacts': [manifest]})))
        self.db.commit()
        return path

    def idle(self, paths):
        self.audit.append(paths)

    def run_operation(self, apply=False):
        return maintenance.deduplicate(self.root, apply=apply, minimum_bytes=65536, idle=self.idle)

    def test_preview_retains_all_independent_files_and_avoids_process_mutation(self):
        a, b = self.artifact(), self.artifact()
        before = [maintenance.signature(p) for p in [a, b]]
        result = self.run_operation()
        self.assertEqual(result['deduplicated_bytes'], len(self.payload))
        self.assertEqual([maintenance.signature(p) for p in [a, b]], before)
        self.assertEqual(self.audit, [])

    def test_apply_keeps_paths_bytes_manifests_and_later_publication_is_isolated(self):
        a, b, c = self.artifact(), self.artifact(), self.artifact()
        rows = self.db.execute('SELECT * FROM tasks ORDER BY task_id').fetchall()
        result = self.run_operation(True)
        self.assertEqual(result['preserved_paths'], 2)
        self.assertEqual(result['deduplicated_bytes'], len(self.payload) * 2)
        self.assertEqual(len({p.stat().st_ino for p in [a, b, c]}), 1)
        self.assertEqual(self.db.execute('SELECT * FROM tasks ORDER BY task_id').fetchall(), rows)
        for path in [a, b, c]:self.assertEqual(path.read_bytes(), self.payload)
        self.assertEqual(self.audit[0][0], self.root / 'target/release/clawd')
        # The host publishes changed snapshots with private-copy + atomic replace.
        updated = b.with_name('.next-version')
        updated.write_bytes(b'a new artifact')
        os.replace(updated, b)
        self.assertEqual(a.read_bytes(), self.payload)
        self.assertEqual(c.read_bytes(), self.payload)
        self.assertEqual(b.read_bytes(), b'a new artifact')

    def test_other_principals_running_jobs_and_unregistered_files_are_excluded(self):
        paths = [self.artifact(principal='a'), self.artifact(principal='b'), self.artifact(status='running')]
        source = self.root / '.agent-runtime/artifacts/skill-invocations/source.bin'
        source.parent.mkdir(parents=True)
        source.write_bytes(self.payload)
        result = self.run_operation(True)
        self.assertEqual(result['preserved_paths'], 0)
        self.assertEqual(len({p.stat().st_ino for p in paths}), 3)
        self.assertEqual(source.read_bytes(), self.payload)

    def test_tampered_manifest_and_symlink_cannot_enter_a_duplicate_group(self):
        good = self.artifact()
        bad = self.artifact(expected='0' * 64)
        symlink = self.artifact()
        symlink.unlink()
        symlink.symlink_to(good)
        result = self.run_operation(True)
        self.assertEqual(result['preserved_paths'], 0)
        self.assertEqual(result['skipped'][0]['reason_code'], 'manifest_digest_mismatch')
        self.assertNotEqual(good.stat().st_ino, bad.stat().st_ino)
        self.assertTrue(symlink.is_symlink())

    def test_busy_or_incomplete_reference_audit_aborts_before_changing_data(self):
        a, b = self.artifact(), self.artifact()
        before = [maintenance.signature(p) for p in [a, b]]
        def busy(_):raise RuntimeError('cleanup_cache_in_use')
        with self.assertRaisesRegex(RuntimeError, 'cleanup_cache_in_use'):
            maintenance.deduplicate(self.root, apply=True, minimum_bytes=65536, idle=busy)
        self.assertEqual([maintenance.signature(p) for p in [a, b]], before)

    def test_second_run_is_idempotent(self):
        self.artifact();self.artifact();self.artifact()
        self.run_operation(True)
        self.assertEqual(self.run_operation(True)['preserved_paths'], 0)

    def test_shared_source_inode_cannot_become_the_retained_delivery_copy(self):
        a,b = self.artifact(),self.artifact()
        source = self.root/'source.bin'
        os.link(a,source)
        result = self.run_operation(True)
        self.assertEqual(result['preserved_paths'],0)
        self.assertEqual(result['skipped'][0]['reason_code'],'existing_external_hardlink')
        self.assertNotEqual(a.stat().st_ino,b.stat().st_ino)
        self.assertEqual(source.read_bytes(),self.payload)


if __name__ == '__main__':unittest.main()
