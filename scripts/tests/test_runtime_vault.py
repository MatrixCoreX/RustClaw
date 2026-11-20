import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import runtime_checkpoint as checkpoint
import runtime_vault_receiver as receiver
from runtime_checkpoint_vault import build_snapshot, RemoteTransport
from runtime_vault_cipher import VaultCipher, load_key, CHUNK_BYTES
from runtime_vault_payloads import enumerate_payloads
from runtime_vault_restore import LocalReader, RemoteReader, restore
from runtime_vault_verify import verify


class LocalTransport:
    def __init__(self,root):self.root=root;self.report={}
    def inventory(self):return receiver.inventory(self.root)
    def load_manifest(self,name):return (self.root/'snapshots'/(name+'.enc')).read_bytes()
    @contextlib.contextmanager
    def writer(self):
        with tempfile.TemporaryFile() as stream:
            yield stream
            stream.seek(0);self.report=receiver.receive(self.root,stream)


class ProcessTransport(RemoteTransport):
    def __init__(self,root):self.root=root
    def command(self,operation,*args):return [sys.executable,str(Path(receiver.__file__)),operation,str(self.root),*args]


class CipherTests(unittest.TestCase):
    def test_authenticated_objects_bind_identity_key_and_size(self):
        cipher=VaultCipher(b'x'*32)
        identity,blob=cipher.encrypt_object(b'private fixture payload'*1000)
        self.assertEqual(cipher.decrypt_object(identity,blob),b'private fixture payload'*1000)
        other_id,other_blob=cipher.encrypt_object(b'private fixture payload'*1000)
        self.assertEqual(identity,other_id);self.assertNotEqual(blob,other_blob)
        for key,name,value in [(cipher,'0'*64,blob),(VaultCipher(b'y'*32),identity,blob),(cipher,identity,blob[:-1]),(cipher,identity,blob[:25]+bytes([blob[25]^1])+blob[26:])]:
            with self.assertRaises(RuntimeError):key.decrypt_object(name,value)
        with self.assertRaisesRegex(RuntimeError,'too_large'):cipher.encrypt_object(b'x'*(CHUNK_BYTES+1))

    def test_recovery_keys_are_private_and_existing_keys_are_never_replaced(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'key';first=load_key(path,True)
            self.assertEqual(first,load_key(path,True));self.assertEqual(path.stat().st_mode&0o777,0o600)
            path.chmod(0o644)
            with self.assertRaisesRegex(RuntimeError,'not_private'):load_key(path)
            path.chmod(0o600);link=path.with_name('link');link.symlink_to(path)
            with self.assertRaisesRegex(RuntimeError,'not_private'):load_key(link)


class VaultTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.base=Path(self.temp.name).resolve();self.root=self.base/'source';self.root.mkdir()
        self.remote=self.base/'vault';self.remote.mkdir(mode=0o700)
        self.destination=self.base/'checkpoints';self.destination.mkdir(mode=0o700)
        (self.root/'configs').mkdir();(self.root/'data').mkdir()
        (self.root/'configs/config.toml').write_text('[database]\nsqlite_path="data/runtime.db"\n')
        (self.root/'configs/private.toml').write_text('credential="SENSITIVE_FIXTURE_ONLY"\n')
        with contextlib.closing(sqlite3.connect(self.root/'data/runtime.db')) as db,db:
            db.execute('CREATE TABLE tasks(status TEXT,body TEXT)');db.execute("INSERT INTO tasks VALUES('succeeded','original result')")
        subprocess.run(['git','init','-q',str(self.root)],check=True)
        subprocess.run(['git','-C',str(self.root),'-c','user.name=fixture','-c','user.email=fixture@example.invalid','commit','--allow-empty','-qm','fixture'],check=True)
        self.model=self.root/'data/models/model.bin';self.model.parent.mkdir();self.model.write_bytes(b'model-fixture'*1000)
        artifact=self.root/'.agent-runtime/artifacts/completed.txt';artifact.parent.mkdir(parents=True);artifact.write_text('completed attachment')
        package=self.root/'data/skill-packages/fixture/versions/1.0.0-test';package.mkdir(parents=True)
        (package/'package.bin').write_bytes(b'package fixture'*1000)
        (package/'empty').mkdir();os.link(package/'package.bin',package/'hardlink.bin');(package/'relative.bin').symlink_to('package.bin')
        self.pointer=package.parent.parent/'current.json';self.pointer.write_text(json.dumps({'schema_version':1,'version':'1.0.0','install_dir':'1.0.0-test','receipt_digest':'a'*64}))
        self.transport=LocalTransport(self.remote);self.key=b'k'*32
        self.reserve=patch.object(receiver,'RESERVE',0);self.reserve.start();self.addCleanup(self.reserve.stop)
    def snapshot(self,scopes=('models','packages'),retain=3):
        result=checkpoint.create(self.root,self.destination,mode='offline',unit='',offline=True,reserve=0)
        return build_snapshot(self.root,Path(result['checkpoint']),self.transport,self.key,scopes,retain)

    def test_full_selected_payload_restore_and_no_plaintext_or_paths_in_remote_metadata(self):
        result=self.snapshot(('models','packages','attachments'))
        self.assertTrue(result['encrypted']);self.assertGreater(result['uploaded_bytes'],0)
        index=json.loads((self.remote/'vault-index.json').read_text())
        self.assertNotIn('private.toml',json.dumps(index));self.assertNotIn('model.bin',json.dumps(index))
        for path in self.remote.rglob('*'):
            if path.is_file():self.assertNotIn(b'SENSITIVE_FIXTURE_ONLY',path.read_bytes())
        output=self.base/'restored';report=restore(LocalReader(self.remote),self.key,output,reserve=0)
        self.assertFalse(report['activated']);self.assertEqual(report['status'],'restored_verified')
        self.assertEqual((output/'configs/private.toml').read_bytes(),(self.root/'configs/private.toml').read_bytes())
        self.assertEqual((output/'data/models/model.bin').read_bytes(),self.model.read_bytes())
        self.assertEqual((output/'.agent-runtime/artifacts/completed.txt').read_text(),'completed attachment')
        package=output/'data/skill-packages/fixture/versions/1.0.0-test'
        self.assertTrue((package/'empty').is_dir());self.assertTrue((package/'relative.bin').is_symlink())
        self.assertEqual((package/'package.bin').stat().st_ino,(package/'hardlink.bin').stat().st_ino)
        with self.assertRaisesRegex(RuntimeError,'output_exists'):restore(LocalReader(self.remote),self.key,output,reserve=0)

    def test_restore_refuses_a_filesystem_that_does_not_enforce_private_output(self):
        self.snapshot();output=self.base/'insecure-output';original=Path.mkdir
        def insecure(path,*args,**kwargs):
            result=original(path,*args,**kwargs)
            if path==output:path.chmod(0o755)
            return result
        with patch.object(Path,'mkdir',insecure):
            with self.assertRaisesRegex(RuntimeError,'output_not_private'):restore(LocalReader(self.remote),self.key,output,reserve=0)
        self.assertFalse(output.exists());self.assertTrue(self.model.exists())

    def test_incremental_versions_reuse_objects_and_retire_only_unreferenced_data(self):
        first=self.snapshot();first_objects=set(self.transport.inventory()['objects'])
        self.model.write_bytes(b'changed model fixture'*1000)
        second=self.snapshot();self.assertGreater(second['reused_bytes'],0);self.assertGreater(second['uploaded_bytes'],0)
        third=self.snapshot();self.assertEqual(third['uploaded_bytes'],0)
        index=json.loads((self.remote/'vault-index.json').read_text());self.assertEqual(len(index['snapshots']),3)
        unknown=self.remote/'operator-note';unknown.write_text('retained')
        last=self.snapshot(retain=1)
        self.assertEqual(last['retained_versions'],1);self.assertEqual(unknown.read_text(),'retained')
        current=self.transport.inventory();self.assertEqual(set(p.name for p in (self.remote/'objects').iterdir()),set(current['objects']))
        self.assertTrue(first_objects.intersection(current['objects']))

    def test_remote_phase_replay_does_not_upload_again_or_create_a_duplicate_version(self):
        local=checkpoint.create(self.root,self.destination,mode='offline',unit='',offline=True,reserve=0)
        args=(self.root,Path(local['checkpoint']),self.transport,self.key,('models',),3)
        build_snapshot(*args);second=build_snapshot(*args)
        self.assertTrue(second['idempotent_replay']);self.assertEqual(second['uploaded_bytes'],0)
        self.assertEqual(len(json.loads((self.remote/'vault-index.json').read_text())['snapshots']),1)

    def test_wrong_key_corrupt_object_and_inadequate_space_never_activate_or_remove_old_versions(self):
        self.snapshot();before=(self.remote/'vault-index.json').read_bytes()
        with self.assertRaises(RuntimeError):restore(LocalReader(self.remote),b'w'*32,self.base/'wrong',reserve=0)
        self.assertFalse((self.base/'wrong').exists())
        with self.assertRaisesRegex(RuntimeError,'low_space'):restore(LocalReader(self.remote),self.key,self.base/'space',reserve=10**30)
        row=json.loads(before)['snapshots'][0];identity=next(iter(row['objects']));path=self.remote/'objects'/identity
        old=path.read_bytes();path.write_bytes(old[:-1])
        with self.assertRaises(RuntimeError):restore(LocalReader(self.remote),self.key,self.base/'corrupt',reserve=0)
        self.assertFalse((self.base/'corrupt').exists());self.assertEqual((self.remote/'vault-index.json').read_bytes(),before)

    def test_scope_selection_does_not_silently_include_attachments_and_rejects_bad_pointers(self):
        self.snapshot(('models',));output=self.base/'models-only'
        restore(LocalReader(self.remote),self.key,output,prefixes=['data/models'],reserve=0)
        self.assertFalse((output/'.agent-runtime').exists());self.assertFalse((output/'configs').exists())
        self.pointer.write_text(json.dumps({'schema_version':1,'install_dir':'../../escape'}))
        with self.assertRaisesRegex(RuntimeError,'pointer_invalid'):list(enumerate_payloads(self.root,['packages']))
        with self.assertRaisesRegex(RuntimeError,'scope_invalid'):list(enumerate_payloads(self.root,['unknown']))

    def test_transport_traversal_and_truncation_preserve_previous_index(self):
        self.snapshot();before=(self.remote/'vault-index.json').read_bytes()
        attack=io.BytesIO()
        with tarfile.open(fileobj=attack,mode='w') as archive:
            member=tarfile.TarInfo('../escape');member.size=1;archive.addfile(member,io.BytesIO(b'x'))
        attack.seek(0)
        with self.assertRaisesRegex(RuntimeError,'member_invalid'):receiver.receive(self.remote,attack)
        with self.assertRaises(Exception):receiver.receive(self.remote,io.BytesIO(b'truncated'))
        self.assertEqual((self.remote/'vault-index.json').read_bytes(),before)
        self.assertFalse(list(self.remote.glob('.pending-*')));self.assertFalse((self.base/'escape').exists())

    def test_real_pipe_transport_restores_repeated_large_chunks_without_local_blob_cache(self):
        self.model.write_bytes(b'a'*CHUNK_BYTES+b'a'*CHUNK_BYTES+b'b'*1000)
        self.transport=ProcessTransport(self.remote)
        self.snapshot(('models',))
        reader=RemoteReader(self.transport)
        report=verify(reader,self.key);self.assertEqual(report['status'],'decrypted_hashes_verified');self.assertFalse(report['writes_restored_files'])
        output=self.base/'streamed';restored=restore(reader,self.key,output,reserve=0)
        self.assertEqual(restored['status'],'restored_verified');self.assertEqual((output/'data/models/model.bin').read_bytes(),self.model.read_bytes())

    def test_frozen_package_pointer_never_follows_a_new_live_current_version(self):
        local=checkpoint.create(self.root,self.destination,mode='offline',unit='',offline=True,reserve=0)
        newer=self.pointer.parent/'versions/2.0.0-new';newer.mkdir();(newer/'new.bin').write_text('different package')
        self.pointer.write_text(json.dumps({'schema_version':1,'version':'2.0.0','install_dir':'2.0.0-new','receipt_digest':'b'*64}))
        build_snapshot(self.root,Path(local['checkpoint']),self.transport,self.key,('packages',),3)
        output=self.base/'pinned';restore(LocalReader(self.remote),self.key,output,reserve=0)
        self.assertTrue((output/'data/skill-packages/fixture/versions/1.0.0-test/package.bin').is_file())
        self.assertFalse((output/'data/skill-packages/fixture/versions/2.0.0-new').exists())

    def test_lost_publication_ack_never_deletes_objects_referenced_by_committed_index(self):
        self.snapshot();self.model.write_bytes(b'new content after first snapshot')
        original=receiver.atomic_json
        def lost(path,value):
            original(path,value)
            if path.name=='vault-index.json':raise OSError('fixture lost acknowledgement after atomic publication')
        with patch.object(receiver,'atomic_json',side_effect=lost):
            with self.assertRaises(OSError):self.snapshot()
        receiver.recover(self.remote)
        result=verify(LocalReader(self.remote),self.key)
        self.assertEqual(result['status'],'decrypted_hashes_verified');self.assertFalse((self.remote/'.vault-transaction.json').exists())

    def test_recovery_cleans_only_owned_pending_transfers_and_uncommitted_journal_objects(self):
        self.snapshot();before=(self.remote/'vault-index.json').read_bytes()
        orphan=self.remote/'objects'/('e'*64);orphan.write_bytes(b'fixture orphan')
        receiver.atomic_json(self.remote/'.vault-transaction.json',{'schema_version':1,'name':'snapshot-interrupted','manifest_sha256':'a'*64,'created':{orphan.name:hashlib.sha256(orphan.read_bytes()).hexdigest()},'previous':[]})
        pending=self.remote/('.pending-'+'f'*32);pending.mkdir();receiver.atomic_json(pending/'.vault-owner.json',{'schema_version':1,'kind':'runtime_vault_pending'});(pending/'partial').write_text('partial')
        unrelated=self.remote/('.pending-'+'d'*32);unrelated.mkdir();(unrelated/'note').write_text('operator data')
        receiver.recover(self.remote)
        self.assertFalse(orphan.exists());self.assertFalse(pending.exists());self.assertTrue(unrelated.exists());self.assertEqual((self.remote/'vault-index.json').read_bytes(),before)

    def test_source_mutation_before_publication_retains_previous_remote_snapshot(self):
        self.snapshot();before=(self.remote/'vault-index.json').read_bytes()
        self.model.write_bytes(b'unique model fixture to force a new object')
        original=VaultCipher.encrypt_object
        def changed(cipher,raw):
            result=original(cipher,raw)
            if raw.startswith(b'unique model fixture'):
                self.model.write_bytes(b'changed while reading fixture')
            return result
        with patch.object(VaultCipher,'encrypt_object',changed):
            with self.assertRaisesRegex(RuntimeError,'payload_changed'):self.snapshot()
        self.assertEqual((self.remote/'vault-index.json').read_bytes(),before)


if __name__=='__main__':unittest.main()
