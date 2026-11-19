import contextlib
import importlib.util
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import runtime_checkpoint as checkpoint
from runtime_checkpoint_storage import restore_checkpoint,verify_checkpoint

class Checkpoints(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory();self.addCleanup(self.tmp.cleanup);self.base=Path(self.tmp.name)
        self.root=self.base/'root';self.root.mkdir();self.destination=self.base/'backups';self.destination.mkdir(mode=0o700)
        (self.root/'configs').mkdir();(self.root/'data').mkdir();(self.root/'configs/config.toml').write_text('[database]\nsqlite_path="data/tasks.db"\n')
        (self.root/'configs/private.toml').write_text('secret="fixture-only"\n')
        with contextlib.closing(sqlite3.connect(self.root/'data/tasks.db')) as db, db:
            db.execute('CREATE TABLE tasks(status TEXT,body TEXT)');db.execute("INSERT INTO tasks VALUES('succeeded','original evidence')")
        subprocess.run(['git','init','-q',str(self.root)],check=True)
        subprocess.run(['git','-C',str(self.root),'-c','user.name=fixture','-c','user.email=fixture@example.invalid','commit','--allow-empty','-qm','fixture'],check=True)
    def create(self):
        return checkpoint.create(self.root,self.destination,mode='offline',unit='',offline=True,reserve=0)
    def test_lossless_restore_and_retirement_only_after_verified_success(self):
        result=self.create();backup=Path(result['checkpoint']);verify_checkpoint(backup)
        output=self.base/'restored';report=restore_checkpoint(backup,output);self.assertFalse(report['activated'])
        self.assertEqual((output/'configs/private.toml').read_text(),'secret="fixture-only"\n')
        with contextlib.closing(sqlite3.connect(output/'data/tasks.db')) as db, db:self.assertEqual(db.execute('SELECT body FROM tasks').fetchone()[0],'original evidence')
        with self.assertRaisesRegex(RuntimeError,'output_exists'):restore_checkpoint(backup,output)
        with patch.object(checkpoint,'backup_files',side_effect=RuntimeError('checkpoint_simulated_write_failure')):
            with self.assertRaises(RuntimeError):self.create()
        self.assertTrue(backup.exists());self.assertFalse(list(self.destination.glob('.pending-*')))
    def test_corruption_symlinks_and_low_space_are_rejected(self):
        result=self.create();backup=Path(result['checkpoint']);record=json.loads((backup/'manifest.json').read_text())['files'][0]
        (backup/record['archive']).write_bytes(b'corrupt')
        with self.assertRaises(RuntimeError):verify_checkpoint(backup)
        with self.assertRaisesRegex(RuntimeError,'low_space'):
            checkpoint.create(self.root,self.destination,mode='offline',unit='',offline=True,reserve=10**30)
    def test_active_tasks_block_backup_and_default_requires_another_filesystem(self):
        with contextlib.closing(sqlite3.connect(self.root/'data/tasks.db')) as db, db:db.execute("INSERT INTO tasks VALUES('running','in progress')")
        with self.assertRaisesRegex(RuntimeError,'foreground_not_idle'):self.create()
        with self.assertRaisesRegex(RuntimeError,'other_filesystem'):
            checkpoint.create(self.root,self.destination,mode='systemd-system',unit='fixture.service',reserve=0)
    def test_stopped_service_is_restarted_after_failed_backup(self):
        calls=[]
        original=Path.stat
        # Public create uses offline for fixtures; the real controller contract is separately tested via fake destination identity.
        def stat(path,*a,**kw):
            value=original(path,*a,**kw)
            if path==self.destination:
                values=list(value);values[2]+=1;return os.stat_result(values)
            return value
        with patch.object(checkpoint,'running_revision',return_value='fixture-revision'),patch.object(Path,'stat',stat),patch.object(checkpoint,'controller',side_effect=lambda mode,unit,action:(calls.append(action) or True)),patch.object(checkpoint,'freeze',side_effect=RuntimeError('checkpoint_simulated')):
            with self.assertRaises(RuntimeError):checkpoint.create(self.root,self.destination,mode='systemd-system',unit='fixture.service',reserve=0)
        self.assertEqual(calls,['is-active','stop','start'])

    def test_manifest_traversal_and_checkpoint_symlink_are_rejected(self):
        result=self.create();backup=Path(result['checkpoint'])
        link=self.base/'alias';link.symlink_to(backup,target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError,'root_symlink'):verify_checkpoint(link)
        path=backup/'manifest.json';value=json.loads(path.read_text());value['files'][0]['path']='../escaped'
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(RuntimeError,'path_invalid'):restore_checkpoint(backup,self.base/'unsafe-restore')
        self.assertFalse((self.base/'escaped').exists())


if __name__=='__main__':unittest.main()
