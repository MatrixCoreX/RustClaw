"""Upgrade an actual legacy desktop vault using an isolated OS keyring and UI.
Arguments: legacy executable, new executable. No real user data is opened.
"""
import base64
import hashlib
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import urllib.request
import uuid

ROOT = Path(__file__).resolve().parents[1]
OUT = Path(os.environ.get('DESKTOP_TEST_OUTPUT_DIR', ROOT / 'test-results')) / ('upgrade-' + uuid.uuid4().hex[:8])
OUT.mkdir(parents=True)
(OUT / 'runtime').mkdir(mode=0o700)
os.environ.update(XDG_RUNTIME_DIR=str(OUT / 'runtime'), XDG_DATA_HOME=str(OUT / 'data'), XDG_CONFIG_HOME=str(OUT / 'config'), XDG_CACHE_HOME=str(OUT / 'cache'), GDK_BACKEND='x11', LIBGL_ALWAYS_SOFTWARE='1', WEBKIT_DISABLE_DMABUF_RENDERER='1', TAURI_WEBVIEW_AUTOMATION='true')
os.environ.pop('GNOME_KEYRING_CONTROL', None)
assert os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'disposable D-Bus session required'
xvfb = subprocess.Popen(['Xvfb', '-displayfd', '1', '-screen', '0', '1280x900x24'], stdout=subprocess.PIPE, stderr=(OUT / 'xvfb.log').open('w'))
os.environ['DISPLAY'] = ':' + xvfb.stdout.readline().decode().strip()
subprocess.run(['dbus-update-activation-environment', 'DISPLAY', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME', 'XDG_RUNTIME_DIR'], check=True)
subprocess.run(['gnome-keyring-daemon', '--unlock', '--components=secrets'], input=b'fixture-keyring-password', check=True, stdout=subprocess.DEVNULL)
from wallet_portal_fixture import FilePortal
portal = FilePortal(OUT / 'legacy.backup.json')
driver = subprocess.Popen([str(ROOT / '.build/tools/bin/tauri-driver'), '--port', '4477', '--native-port', '4478', '--native-driver', '/usr/bin/WebKitWebDriver'], stdout=(OUT / 'driver.log').open('w'), stderr=subprocess.STDOUT, start_new_session=True)
sid = None
PASSWORD = 'test-only-vault-password'
legacy, current = [str(Path(p).resolve()) for p in sys.argv[1:3]]
checks = []

def rpc(method, path, data=None):
    request = urllib.request.Request('http://127.0.0.1:4477' + path, data=None if data is None else json.dumps(data).encode(), method=method, headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=60) as response:
        value = json.load(response)['value']
    assert not (isinstance(value, dict) and isinstance(value.get('error'), str) and 'message' in value), value
    return value

def execute(script, args=None, asynchronous=False):
    return rpc('POST', f'/session/{sid}/execute/{"async" if asynchronous else "sync"}', {'script': script, 'args': args or []})

def native(command, args=None, error=None):
    result = execute('const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(e=>done({ok:false,error:String(e)}));', [command, args or {}], True)
    if error:
        assert not result['ok'] and error in result['error'], result
        return
    assert result['ok'], result
    return result.get('value')

def wait(script):
    for _ in range(150):
        if execute(script):
            return
        time.sleep(.1)
    raise AssertionError(script)

def launch(binary):
    global sid
    sid = rpc('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': binary}}}})['sessionId']
    wait("return !!window.__TAURI_INTERNALS__ && document.body.innerText.includes('管理你的设备')")
    main = rpc('GET', f'/session/{sid}/window')
    native('wallet_open')
    for _ in range(100):
        handles = rpc('GET', f'/session/{sid}/window/handles')
        if len(handles) > 1:
            break
        time.sleep(.1)
    wallet = next(h for h in handles if h != main)
    rpc('POST', f'/session/{sid}/window', {'handle': wallet})
    wait('return !!document.querySelector(".wallet-manager")')

def screenshot(name):
    (OUT / (name + '.png')).write_bytes(base64.b64decode(rpc('GET', f'/session/{sid}/screenshot')))

def close():
    global sid
    rpc('DELETE', f'/session/{sid}')
    sid = None
    # Only this isolated test's executable and data directory are eligible.
    for proc in Path('/proc').iterdir():
        if not proc.name.isdigit():
            continue
        try:
            if os.readlink(proc / 'exe') not in (legacy, current):
                continue
            env = (proc / 'environ').read_bytes().split(b'\0')
            if ('XDG_DATA_HOME=' + str(OUT / 'data')).encode() in env:
                os.kill(int(proc.name), signal.SIGTERM)
        except (PermissionError, FileNotFoundError, ProcessLookupError):
            pass
    time.sleep(.5)

try:
    for _ in range(60):
        try:
            rpc('GET', '/status')
            break
        except Exception:
            time.sleep(.2)
    launch(legacy)
    native('wallet_initialize', {'password': PASSWORD})
    first = native('wallet_create', {'name': 'legacy first'})
    second = native('wallet_create', {'name': 'legacy second'})
    native('wallet_backup', {'accountId': first['id'], 'vaultPassword': PASSWORD, 'password': 'aaaaaaaaaaaa'})
    accounts = native('wallet_status')['accounts']
    vault_path = next((OUT / 'data').rglob('vault-v1.json'))
    old_bytes = vault_path.read_bytes()
    old_backup = portal.path.read_bytes()
    assert json.loads(old_bytes)['version'] == 1
    assert json.loads(old_backup)['format'] == 'asset-account-backup-v1'
    close()
    launch(current)
    status = native('wallet_status')
    assert status['storage_version'] == 1 and not status['unlocked'] and status['accounts'] == accounts
    assert vault_path.read_bytes() == old_bytes
    wait('return !!document.querySelector("[data-wallet-migration]")')
    screenshot('01-before-upgrade')
    checks.append('new executable opens a real legacy vault without migrating on startup and shows the upgrade explanation')
    native('wallet_unlock', {'password': 'wrong-password'}, 'wallet_unlock_failed')
    assert vault_path.read_bytes() == old_bytes
    time.sleep(2.1)
    native('wallet_unlock', {'password': PASSWORD})
    status = native('wallet_status')
    assert status['storage_version'] == 2 and status['accounts'] == accounts
    assert status['backup_upgrade_accounts'] == [first['id']]
    wait('return document.querySelectorAll("[data-wallet-backup-upgrade]").length === 1')
    screenshot('02-upgraded-accounts')
    execute('document.querySelector("[data-desktop-language-toggle]").click();return true;')
    wait('return document.querySelector("[data-wallet-backup-upgrade]").textContent.includes("Backup update recommended")')
    screenshot('03-upgrade-english')
    execute('document.querySelector("[data-desktop-language-toggle]").click();return true;')
    assert portal.path.read_bytes() == old_backup
    native('wallet_restore', {'password': 'aaaaaaaaaaaa', 'name': 'duplicate'}, 'wallet_account_duplicate')
    checks.append('correct password migrates legacy OS credentials and both accounts; IDs/public keys/backup state survive; the legacy weak-password backup still authenticates and English guidance works')
    portal.path = OUT / 'new.backup.json'
    native('wallet_backup', {'accountId': first['id'], 'vaultPassword': PASSWORD, 'password': 'Jasper!flume7-Pebble4-Orbit9-velvet'})
    assert native('wallet_status')['backup_upgrade_accounts'] == []
    assert json.loads(portal.path.read_bytes())['format'] == 'asset-account-backup-v2'
    stable = vault_path.read_bytes()
    close()
    launch(current)
    assert native('wallet_status')['storage_version'] == 2
    native('wallet_unlock', {'password': PASSWORD})
    assert native('wallet_status')['accounts'] == accounts
    assert vault_path.read_bytes() == stable
    checks.append('verified v2 re-export clears the legacy-backup notice; the upgraded native vault reopens without another migration or account changes')
    (OUT / 'acceptance.json').write_text(json.dumps({'checks': checks, 'passed': len(checks), 'legacy_binary_sha256': hashlib.sha256(Path(legacy).read_bytes()).hexdigest(), 'binary_sha256': hashlib.sha256(Path(current).read_bytes()).hexdigest()}, indent=2) + '\n')
    print('PASS ' + str(OUT), flush=True)
finally:
    if sid:
        try:
            close()
        except Exception:
            pass
    os.killpg(driver.pid, signal.SIGTERM)
    xvfb.terminate()
