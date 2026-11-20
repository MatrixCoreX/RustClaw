#!/usr/bin/env python3
"""Install a daily private control-plane checkpoint on a systemd Linux host."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys


def quoted(value):
    if any(c in str(value) for c in ('\n','\r','\0')):raise RuntimeError('checkpoint_unit_value_invalid')
    return '"'+str(value).replace('\\','\\\\').replace('"','\\"').replace('%','%%')+'"'


def units(root,destination,user,python,remote_host=None,remote_directory=None,*,vault_key_file=None,scopes=('packages','models'),retain_versions=3,retry_limit=6):
    if not 1<=retain_versions<=8 or not 1<=retry_limit<=12:raise RuntimeError('backup_limits_invalid')
    from runtime_vault_payloads import SCOPES
    if not set(scopes).issubset(SCOPES):raise RuntimeError('backup_scopes_invalid')
    command=[python,root/'scripts/runtime_backup_schedule.py','--root',root,'--destination',destination,'--reserve-bytes','2147483648','--read-fence-seconds','1800','--retry-limit',str(retry_limit),'--retain-versions',str(retain_versions),'--scopes',*scopes]
    if remote_host:
        from runtime_checkpoint_remote import validate_target
        validate_target(remote_host,remote_directory)
        command.extend(['--remote-host',remote_host,'--remote-directory',remote_directory])
    elif remote_directory or vault_key_file:raise RuntimeError('backup_remote_target_missing')
    if vault_key_file:command.extend(['--vault-key-file',vault_key_file])
    service='''[Unit]
Description=Verified runtime control-plane checkpoint
After=agent-runtime.service
[Service]
Type=oneshot
User={user}
UMask=0077
Nice=15
IOSchedulingClass=idle
TimeoutStartSec=90min
ExecStart={command}
ExecStopPost={python} {script} resume --root {root}
'''.format(user=user,python=quoted(python),script=quoted(root/'scripts/runtime_checkpoint.py'),root=quoted(root),command=' '.join(quoted(value) for value in command))
    timer='''[Unit]
Description=Daily runtime control-plane checkpoint
[Timer]
OnCalendar=*-*-* 04:30:00
RandomizedDelaySec=15min
Persistent=true
[Install]
WantedBy=timers.target
'''
    return service,timer


def retry_units(service):
    lines=service.splitlines()
    retry_service='\n'.join(line+' --retry-only' if line.startswith('ExecStart=') else line for line in lines)+'\n'
    retry_timer='''[Unit]
Description=Retry deferred runtime backups and sample storage growth
[Timer]
OnBootSec=15min
OnUnitActiveSec=15min
RandomizedDelaySec=2min
[Install]
WantedBy=timers.target
'''
    return retry_service,retry_timer


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',required=True,type=Path);p.add_argument('--destination',required=True,type=Path);p.add_argument('--install',action='store_true');p.add_argument('--remote-host');p.add_argument('--remote-directory');p.add_argument('--vault-key-file',type=Path);p.add_argument('--scopes',nargs='*',default=['packages','models']);p.add_argument('--retain-versions',type=int,default=3);p.add_argument('--retry-limit',type=int,default=6);a=p.parse_args()
    if sys.platform!='linux' or not Path('/run/systemd/system').exists():
        print(json.dumps({'status':'unsupported','error_code':'checkpoint_timer_requires_systemd','alternative':'Use runtime_checkpoint.py from an operator-managed scheduler with a supported service controller.'}));return 1
    import pwd
    root=a.root.resolve();destination=a.destination.resolve();user=pwd.getpwuid(os.getuid()).pw_name
    service,timer=units(root,destination,user,sys.executable,a.remote_host,a.remote_directory,vault_key_file=a.vault_key_file,scopes=a.scopes,retain_versions=a.retain_versions,retry_limit=a.retry_limit)
    retry_service,retry_timer=retry_units(service)
    if a.install:
        if not (root/'scripts/runtime_backup_schedule.py').is_file():raise RuntimeError('checkpoint_timer_script_unavailable')
        for name,content in [('agent-runtime-checkpoint.service',service),('agent-runtime-checkpoint.timer',timer),('agent-runtime-checkpoint-retry.service',retry_service),('agent-runtime-checkpoint-retry.timer',retry_timer)]:
            subprocess.run(['sudo','-n','tee','/etc/systemd/system/'+name],input=content,text=True,stdout=subprocess.DEVNULL,check=True)
        subprocess.run(['sudo','-n','systemctl','daemon-reload'],check=True)
        for unit in ('agent-runtime-checkpoint.service','agent-runtime-checkpoint-retry.service'):
            effective=subprocess.check_output(['systemctl','show',unit,'--property=ExecStart','--value'],text=True)
            if effective.count('path=')!=1 or 'runtime_backup_schedule.py' not in effective:raise RuntimeError('checkpoint_timer_override_requires_review')
        subprocess.run(['sudo','-n','systemctl','enable','--now','agent-runtime-checkpoint.timer','agent-runtime-checkpoint-retry.timer'],check=True)
    print(json.dumps({'status':'installed' if a.install else 'preview','service':service,'timer':timer,'retry_service':retry_service,'retry_timer':retry_timer}));return 0
if __name__=='__main__':sys.exit(main())
