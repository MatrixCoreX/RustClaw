"""Exercise the installed release APK on a disposable Android emulator."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

parser=argparse.ArgumentParser()
parser.add_argument('--serial',required=True)
parser.add_argument('--output',type=Path,required=True)
parser.add_argument('--apk-sha256',required=True)
args=parser.parse_args()
assert args.serial.startswith('emulator-'), 'requires_disposable_emulator'
args.output.mkdir(parents=True,exist_ok=True)
adb=Path(os.environ['ANDROID_HOME'])/'platform-tools/adb'
def device(*command):
    return subprocess.check_output([str(adb),'-s',args.serial,*command],text=True,timeout=180)
path=device('shell','pm','path','org.agent_runtime.mobile').strip().removeprefix('package:')
assert path.startswith('/data/app/') and path.endswith('/base.apk') and '\n' not in path
assert device('shell','sha256sum',path).split()[0]==args.apk_sha256
for name,count in [('NativeSecurityTest',2),('WalletRoundTripTest',1)]:
    result=device('shell','am','instrument','-w','-r','-e','class','org.agent_runtime.mobile.'+name,
                  'org.agent_runtime.mobile.test/androidx.test.runner.AndroidJUnitRunner')
    (args.output/(name+'.log')).write_text(result)
    assert f'OK ({count} test'+('s)' if count>1 else ')') in result,result
    print(name+': PASS',flush=True)
signatures=args.output/'signatures.json'
device('pull','/sdcard/Android/data/org.agent_runtime.mobile/files/wallet-signatures.json',str(signatures))
subprocess.run([sys.executable,str(Path(__file__).with_name('verify_signatures.py')),str(signatures),
                '--output',str(args.output/'signature-verification.json')],check=True)
report=dict(ok=True,apk_sha256=args.apk_sha256,serial=args.serial,
            android_api=int(device('shell','getprop','ro.build.version.sdk').strip()),
            abi=device('shell','getprop','ro.product.cpu.abi').strip(),tests=3,verified_signatures=2)
(args.output/'acceptance.json').write_text(json.dumps(report,indent=2)+'\n')
print(json.dumps(report))
