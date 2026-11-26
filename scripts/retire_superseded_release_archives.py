#!/usr/bin/env python3
"""Retire verified redundant deployment archives, retaining component rollbacks."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


def load(path):
    return json.loads(path.read_text())


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def save(path, data):
    with tempfile.NamedTemporaryFile(mode='w', dir=path.parent, delete=False) as tmp:
        json.dump(data, tmp, indent=2)
        tmp.flush()
        os.fsync(tmp.fileno())
    os.replace(tmp.name, path)


def remote_artifact(repo, proof):
    artifact = json.loads(subprocess.check_output(
        ['gh', 'api', f"repos/{repo}/actions/artifacts/{int(proof['artifact_id'])}"]))
    run = artifact['workflow_run']
    require(not artifact['expired'] and run['head_sha'] == proof['source']
            and run['id'] == proof['workflow_run'] and artifact.get('digest'),
            'signed restoration artifact unavailable or changed')
    return {k:artifact[k] for k in ('id','name','digest','expires_at')}


def plan(prior, successor):
    state = load(successor/'state.json')
    assets = load(prior/'retained-release-assets.json')
    base, active = state['base'], state['source']
    require(base != active and assets['source'] == base, 'superseded source mismatch')
    directory = prior/'latest-packages'/base[:12]
    result = []
    for platform, deployed, rollback in [('ubuntu','local-deployed','component-rollback'),
                                         ('pi','pi-deployed','pi-component-rollback')]:
        installed = load(successor/(deployed+'.json'))
        recovery = load(successor/(rollback+'.json'))
        require(installed['passed'] and installed['source'] == active, 'successor deployment missing')
        require(recovery['source'] == base and digest(Path(recovery['archive'])) == recovery['archive_sha256'],
                'component rollback unavailable')
        remotes = {}
        for source, folder in ((base,prior),(active,successor)):
            proof = load(folder/f'verified-{platform}.json')
            require(proof['source'] == source and proof['platform'] == platform
                    and proof['signature_verified'] and proof['github_provenance_verified']
                    and proof['manifest_and_sbom_verified'], 'signed package verification missing')
            remotes[source] = remote_artifact(state['repo'], proof)
            if source == base:
                originals = {r['name']:r for r in proof['files']}
        archives = [r for r in assets['files'] if r['platform']==platform and r['durable_path'].endswith('.tar.gz')]
        require(len(archives)==1,'expected one complete archive per platform')
        record = archives[0]
        path = Path(record['durable_path'])
        require(path.is_absolute() and path.resolve().parent == directory/platform and not path.is_symlink(),
                'archive outside recorded package directory')
        original = originals.get(path.name, {})
        require((record['bytes'],record['sha256']) == (original.get('bytes'),original.get('sha256')),
                'inventory and signed package disagree')
        result.append({'path':str(path),'sha256':record['sha256'],'bytes':record['bytes'],
                       'restoration':remotes[base], 'successor_artifact':remotes[active]['id'],
                       'previously_retired':record.get('local_archive_retired') is True})
    return {'source':active,'superseded':base,'archives':result}


def inspect(row):
    path = Path(row['path'])
    identity = path.lstat()
    require(path.is_file() and not path.is_symlink() and identity.st_size==row['bytes']
            and digest(path)==row['sha256'],'local package mismatch')
    command = ['fuser',str(path)] if os.geteuid()==0 else ['sudo','-n','fuser',str(path)]
    holders = subprocess.run(command,capture_output=True)
    require(holders.returncode==1 and not holders.stdout.strip() and not holders.stderr.strip(),
            'archive in use or open-file check unavailable')
    return identity


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--prior',type=Path,required=True)
    parser.add_argument('--successor',type=Path,required=True)
    parser.add_argument('--apply',action='store_true',help='apply the verified retirement plan')
    args=parser.parse_args()
    prior, successor = args.prior.resolve(), args.successor.resolve()
    if args.apply:
        with (successor/'.archive-retirement.lock').open('a') as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            retire(prior, successor, apply=True)
    else:
        retire(prior, successor, apply=False)


def retire(prior, successor, apply):
    prior = prior.resolve(strict=True)
    successor = successor.resolve(strict=True)
    audit = successor/'superseded-full-archive-retirement.json'
    report = plan(prior, successor)
    previous = load(audit) if audit.exists() else {}
    pending = []
    for row in report['archives']:
        if not Path(row['path']).exists():
            recorded = (previous.get('source')==report['source'] and previous.get('superseded')==report['superseded']
                        and any(r['path']==row['path'] and r['sha256']==row['sha256'] for r in previous['archives']))
            require(row['previously_retired'] or recorded,'unexplained missing archive')
        else:
            pending.append((row,inspect(row)))
    report.update(available_before=shutil.disk_usage(prior).free, recoverable_bytes=sum(r['bytes'] for r,_ in pending))
    if apply and pending:
        report['retired']=False
        save(audit,report)
        for row, identity in pending:
            path=Path(row['path'])
            current=path.lstat()
            require((current.st_dev,current.st_ino,current.st_ctime_ns)==(identity.st_dev,identity.st_ino,identity.st_ctime_ns),
                    'archive changed after preflight')
            path.unlink()
    if apply:
        inventory_path=prior/'retained-release-assets.json'
        inventory=load(inventory_path)
        require(inventory['source']==report['superseded'],'changed retention inventory')
        for artifact in report['archives']:
            entry=next(r for r in inventory['files'] if r['durable_path']==artifact['path'])
            entry.update(local_archive_retired=True,restoration_artifact_id=artifact['restoration']['id'])
        inventory.update(full_archives_retired=True,superseded_by=report['source'])
        save(inventory_path,inventory)
        report.update(retired=True,available_after=shutil.disk_usage(prior).free)
        if pending or not previous.get('retired'):
            save(audit,report)
    print(json.dumps(report,indent=2))


if __name__=='__main__':
    main()
