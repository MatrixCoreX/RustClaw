#!/usr/bin/env python3
"""Install a daily private control-plane checkpoint on a systemd Linux host."""
import argparse
import json
import os
from pathlib import Path
import pwd
import subprocess
import sys


def quoted(value):
    return '"'+str(value).replace('\\','\\\\').replace('"','\\"').replace('%','%%')+'"'


def units(root,destination,user,python):
    service='''[Unit]
Description=Verified runtime control-plane checkpoint
[Service]
Type=oneshot
User={user}
UMask=0077
Nice=15
IOSchedulingClass=idle
TimeoutStartSec=45min
ExecStart={python} {script} create --root {root} --destination {destination} --retain 1 --reserve-bytes 1610612736
ExecStopPost={python} {script} resume --root {root}
'''.format(user=user,python=quoted(python),script=quoted(root/'scripts/runtime_checkpoint.py'),root=quoted(root),destination=quoted(destination))
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


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--root',required=True,type=Path);p.add_argument('--destination',required=True,type=Path);p.add_argument('--install',action='store_true');a=p.parse_args()
    if sys.platform!='linux' or not Path('/run/systemd/system').exists():
        print(json.dumps({'status':'unsupported','error_code':'checkpoint_timer_requires_systemd','alternative':'Use runtime_checkpoint.py from an operator-managed scheduler with a supported service controller.'}));return 1
    root=a.root.resolve();destination=a.destination.resolve();user=pwd.getpwuid(os.getuid()).pw_name
    service,timer=units(root,destination,user,sys.executable)
    if a.install:
        for name,content in [('agent-runtime-checkpoint.service',service),('agent-runtime-checkpoint.timer',timer)]:
            subprocess.run(['sudo','-n','tee','/etc/systemd/system/'+name],input=content,text=True,stdout=subprocess.DEVNULL,check=True)
        subprocess.run(['sudo','-n','systemctl','daemon-reload'],check=True)
        subprocess.run(['sudo','-n','systemctl','enable','--now','agent-runtime-checkpoint.timer'],check=True)
    print(json.dumps({'status':'installed' if a.install else 'preview','service':service,'timer':timer}));return 0
if __name__=='__main__':sys.exit(main())
