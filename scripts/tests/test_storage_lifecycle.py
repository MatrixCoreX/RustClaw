import hashlib
import os
from pathlib import Path
import tempfile
import sys
import unittest
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import storage_lifecycle as lifecycle


class StorageLifecycleTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.base=Path(self.temp.name).resolve()
        self.root=self.base/'root';self.source=self.root/'.release-backups/version/files.tar.gz';self.source.parent.mkdir(parents=True);self.source.write_bytes(b'original cold archive'*1000)
        self.destination=self.base/'external'
        self.audit=patch.object(lifecycle,'active_references',return_value=[]);self.audit.start();self.addCleanup(self.audit.stop)
        self.device=patch.object(lifecycle,'filesystem_id',side_effect=lambda p:1 if p==self.source else 2);self.device.start();self.addCleanup(self.device.stop)
    def test_preview_does_not_copy_move_or_create_directories(self):
        before=self.source.read_bytes();result=lifecycle.relocate(self.root,self.source,self.destination,reserve=0)
        self.assertEqual(result['mode'],'preview');self.assertFalse(self.destination.exists());self.assertFalse(self.source.is_symlink());self.assertEqual(self.source.read_bytes(),before)
    def test_apply_preserves_original_path_bytes_and_metadata_and_records_receipt(self):
        before=self.source.read_bytes();result=lifecycle.relocate(self.root,self.source,self.destination,reserve=0,apply=True)
        self.assertEqual(result['status'],'relocated_verified');self.assertTrue(self.source.is_symlink())
        self.assertEqual(self.source.read_bytes(),before);self.assertEqual(Path(result['destination']).read_bytes(),before)
        self.assertEqual(result['sha256'],hashlib.sha256(before).hexdigest());self.assertTrue((self.root/'data/storage-relocations.json').is_file())
    def test_active_files_low_space_and_hardlinks_are_never_relocated(self):
        with patch.object(lifecycle,'active_references',return_value=[123]):
            with self.assertRaisesRegex(RuntimeError,'archive_active'):lifecycle.relocate(self.root,self.source,self.destination,reserve=0,apply=True)
        with self.assertRaisesRegex(RuntimeError,'low_space'):lifecycle.relocate(self.root,self.source,self.destination,reserve=10**30,apply=True)
        self.assertFalse(self.source.is_symlink());self.assertFalse(self.destination.exists())
        os.link(self.source,self.source.with_name('another-copy'))
        with self.assertRaisesRegex(RuntimeError,'hardlinked'):lifecycle.relocate(self.root,self.source,self.destination,reserve=0,apply=True)
    def test_user_data_and_same_filesystem_are_rejected(self):
        user=self.root/'user-video.mp4';user.write_bytes(b'user work')
        with self.assertRaisesRegex(RuntimeError,'scope_invalid'):lifecycle.relocate(self.root,user,self.destination,reserve=0,apply=True)
        with patch.object(lifecycle,'filesystem_id',return_value=1):
            with self.assertRaisesRegex(RuntimeError,'other_filesystem'):lifecycle.relocate(self.root,self.source,self.destination,reserve=0,apply=True)
        self.assertEqual(user.read_bytes(),b'user work');self.assertFalse(self.destination.exists())
    def test_copy_failure_keeps_source_and_removes_only_own_temporary_files(self):
        self.destination.mkdir(mode=0o700);note=self.destination/'operator-note';note.write_text('keep')
        with patch.object(lifecycle.shutil,'copystat',side_effect=OSError('fixture write failed')):
            with self.assertRaises(OSError):lifecycle.relocate(self.root,self.source,self.destination,reserve=0,apply=True)
        self.assertFalse(self.source.is_symlink());self.assertEqual(note.read_text(),'keep');self.assertFalse(list(self.destination.glob('.relocating-*')))
    def test_retention_policy_does_not_expire_user_artifacts_models_or_evidence(self):
        self.assertEqual(lifecycle.POLICY['task_files'],'explicit_user_delete_only')
        self.assertEqual(lifecycle.POLICY['models'],'explicit_user_delete_only')
        self.assertEqual(lifecycle.POLICY['validation_evidence'],'preserve')
        self.assertTrue(lifecycle.POLICY['local_checkpoints']['retire_after_verified_replacement'])


if __name__=='__main__':unittest.main()
