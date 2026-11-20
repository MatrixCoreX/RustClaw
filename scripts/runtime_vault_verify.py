"""Verify decrypted file hashes as a stream without a second full restore copy."""
import hashlib
import json
from runtime_checkpoint_storage import relative
from runtime_vault_cipher import VaultCipher


def verify(reader,key):
    cipher=VaultCipher(key);index=reader.index()
    if not index['snapshots']:raise RuntimeError('vault_restore_snapshot_missing')
    entry=index['snapshots'][0];name=entry['name']
    encrypted=reader.manifest(name)
    if hashlib.sha256(encrypted).hexdigest()!=entry['manifest_sha256']:raise RuntimeError('vault_restore_manifest_digest')
    value=json.loads(cipher.decrypt_manifest(encrypted))
    if value.get('schema_version')!=1 or value.get('kind')!='encrypted_runtime_vault' or value.get('name')!=name:raise RuntimeError('vault_restore_schema_invalid')
    seen=set();files=[]
    for row in value['files']:
        path=str(relative(row['path']))
        if path in seen:raise RuntimeError('vault_restore_duplicate_path')
        seen.add(path)
        if row['kind'] in ('file','sqlite'):files.append(row)
        elif row['kind'] not in ('directory','hardlink','symlink'):raise RuntimeError('vault_restore_record_kind_invalid')
    identities=[identity for row in files for identity in row['chunks']]
    total=0
    with reader.objects(name,identities) as stream:
        iterator=iter(stream)
        for row in files:
            sha=hashlib.sha256();size=0
            for expected in row['chunks']:
                try:identity,blob=next(iterator)
                except StopIteration as error:raise RuntimeError('vault_verify_missing_chunk') from error
                if identity!=expected or hashlib.sha256(blob).hexdigest()!=value['objects'][identity]:raise RuntimeError('vault_restore_chunk_order_or_digest')
                raw=cipher.decrypt_object(identity,blob);size+=len(raw)
                if size>row['raw_bytes']:raise RuntimeError('vault_restore_size_mismatch')
                sha.update(raw)
            if size!=row['raw_bytes'] or sha.hexdigest()!=row['raw_sha256']:raise RuntimeError('vault_restore_file_digest')
            total+=size
        try:next(iterator)
        except StopIteration:pass
        else:raise RuntimeError('vault_restore_extra_chunks')
    return {'schema_version':1,'status':'decrypted_hashes_verified','activated':False,'source_revision':value['source_revision'],'files':len(files),'sqlite_files':sum(row['kind']=='sqlite' for row in files),'raw_bytes':total,'manifest_sha256':entry['manifest_sha256'],'writes_restored_files':False}
