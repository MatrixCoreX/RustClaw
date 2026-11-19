import contextlib
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import unittest
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import runtime_checkpoint_remote as remote
from runtime_checkpoint_storage import backup_files,compress_file,write_json,digest

class RemoteCheckpoint(unittest.TestCase):
    def test_target_validation_and_host_key_checks(self):
        remote.validate_target('owner@backup.example','/private/checkpoints')
        for host,path in [('-oProxyCommand=bad','/private/checkpoints'),('host;id','/private/checkpoints'),('host','/private/../outside'),('host','/')]:
            with self.assertRaises(RuntimeError):remote.validate_target(host,path)
        self.assertIn('StrictHostKeyChecking=yes',remote.ssh_options())
        self.assertIn('BatchMode=yes',remote.ssh_options())

    def run_payload(self,code,*args):
        result=subprocess.run([sys.executable,'-B','-c',code,*map(str,args)],capture_output=True,text=True,check=True)
        return json.loads(result.stdout) if result.stdout.strip() else None

    def test_remote_prepare_rejects_a_nonprivate_destination(self):
        with tempfile.TemporaryDirectory() as d:
            root=Path(d).resolve()/'remote'
            result=self.run_payload(remote.PREPARE,root,'.pending-fixture',1)
            self.assertTrue(Path(result['pending']).is_dir())
            self.assertEqual(root.stat().st_mode&0o777,0o700)
            root.chmod(0o755)
            with self.assertRaises(subprocess.CalledProcessError):self.run_payload(remote.PREPARE,root,'.pending-denied',1)
            self.assertFalse((root/'.pending-denied').exists())

    def package(self,base,root,pending_name):
        source=base/'source';(source/'configs').mkdir(parents=True,exist_ok=True);(source/'data').mkdir(exist_ok=True)
        (source/'configs/settings.json').write_text('{"example":"fixture-only"}')
        db_path=source/'data/state.db'
        with contextlib.closing(sqlite3.connect(db_path)) as db,db:
            db.execute('CREATE TABLE IF NOT EXISTS facts(value TEXT)');db.execute("INSERT INTO facts VALUES('original data')")
        self.run_payload(remote.PREPARE,root,pending_name,1000);pending=root/pending_name
        record=compress_file(db_path,pending/'database-0.xz');record.update(path='data/state.db',kind='sqlite',archive='database-0.xz')
        value={'schema_version':1,'kind':'runtime_control_checkpoint','source_revision':'0'*40,'created_at_ts':1,'scope':{'included':['sqlite'],'excluded':[]},'files':[record]}
        backup_files(source,pending,value);write_json(pending/'manifest.json',value)
        module=Path(remote.__file__).with_name('runtime_checkpoint_storage.py');shutil.copyfile(module,pending/module.name)
        return pending,digest(module),digest(pending/'manifest.json')

    def test_publish_restores_databases_then_retires_only_its_previous_checkpoint(self):
        with tempfile.TemporaryDirectory() as d:
            base=Path(d).resolve();root=base/'remote'
            for number in [1,2]:
                pending,module_sha,manifest_sha=self.package(base,root,'.pending-'+str(number))
                report=self.run_payload(remote.PUBLISH,root,pending.name,'checkpoint-'+str(number),module_sha,manifest_sha,'1'*40,'restore')
                self.assertEqual(report['status'],'verified');self.assertEqual(report['verifier_sha256'],module_sha)
                self.assertTrue(report['restore_test']['databases_only']);self.assertFalse((root/('.restore-'+str(number))).exists())
            self.assertFalse((root/'checkpoint-1').exists());self.assertTrue((root/'checkpoint-2/manifest.json').is_file())
            self.assertEqual(len(json.loads((root/'checkpoint-index.json').read_text())['checkpoints']),1)

    def test_restore_test_never_deletes_an_existing_output(self):
        with tempfile.TemporaryDirectory() as d:
            base=Path(d).resolve();root=base/'remote';pending,module_sha,manifest_sha=self.package(base,root,'.pending-existing')
            output=root/'.restore-existing';output.mkdir();(output/'keep').write_text('preserved')
            with self.assertRaises(subprocess.CalledProcessError):self.run_payload(remote.PUBLISH,root,pending.name,'checkpoint-1',module_sha,manifest_sha,'1'*40,'restore')
            self.assertEqual((output/'keep').read_text(),'preserved')
