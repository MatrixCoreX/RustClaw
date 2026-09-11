"""Ubuntu native vault/owner-flow acceptance. Run with dbus-run-session -- /usr/bin/python3.
Uses an isolated OS keyring and loopback TLS fixture; file chooser paths are simulated.
"""
import base64
import ctypes
from http.client import RemoteDisconnected
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
OUT = Path(os.environ.get('DESKTOP_TEST_OUTPUT_DIR', str(ROOT / 'test-results'))) / ('wallet-native-' + uuid.uuid4().hex[:8])
OUT.mkdir(parents=True)
os.environ.update(XDG_DATA_HOME=str(OUT / 'data'), XDG_CONFIG_HOME=str(OUT / 'config'),
    XDG_CACHE_HOME=str(OUT / 'cache'), GDK_BACKEND='x11', LIBGL_ALWAYS_SOFTWARE='1',
    WEBKIT_DISABLE_DMABUF_RENDERER='1', TAURI_WEBVIEW_AUTOMATION='true')
assert os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'disposable D-Bus session required'
xvfb = subprocess.Popen(['Xvfb', '-displayfd', '1', '-screen', '0', '1280x900x24'], stdout=subprocess.PIPE, stderr=(OUT / 'xvfb.log').open('w'))
os.environ['DISPLAY'] = ':' + xvfb.stdout.readline().decode().strip()
subprocess.run(['dbus-update-activation-environment', 'DISPLAY', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'], check=True)
subprocess.run(['gnome-keyring-daemon', '--unlock', '--components=secrets'], input=b'fixture-keyring-password', check=True, stdout=subprocess.DEVNULL)
from fixture_server import Fixture
from wallet_fixture import OwnerApi
from wallet_portal_fixture import FilePortal
fixture = Fixture(OUT / 'tls')
fixture.owner_api = api = OwnerApi()
portal = FilePortal(OUT / 'account.backup.json')
driver = subprocess.Popen([str(ROOT / '.build/tools/bin/tauri-driver'), '--port', '4457', '--native-port', '4458', '--native-driver', '/usr/bin/WebKitWebDriver'], stdout=(OUT / 'driver.log').open('w'), stderr=subprocess.STDOUT, start_new_session=True)
sid = None
checks = []

def rpc(method, path, data=None):
    request = urllib.request.Request('http://127.0.0.1:4457' + path, data=None if data is None else json.dumps(data).encode(), method=method, headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=60) as response:
        value = json.load(response)['value']
    if isinstance(value, dict) and isinstance(value.get('error'), str) and 'message' in value:
        raise AssertionError(value)
    return value

def execute(script, args=None, asynchronous=False):
    return rpc('POST', f'/session/{sid}/execute/{"async" if asynchronous else "sync"}', {'script': script, 'args': args or []})

def native(command, args=None, error=None):
    result = execute('const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(e=>done({ok:false,error:String(e)}));', [command, args or {}], True)
    if error:
        assert not result['ok'] and error in result['error'], result
        return result['error']
    assert result['ok'], result
    return result.get('value')

def switch(handle):
    rpc('POST', f'/session/{sid}/window', {'handle': handle})

def screenshot(name):
    (OUT / (name + '.png')).write_bytes(base64.b64decode(rpc('GET', f'/session/{sid}/screenshot')))

def wait_text(text):
    for _ in range(100):
        if text in execute('return document.body.innerText'):
            return
        time.sleep(.1)
    raise AssertionError(execute('return document.body.innerText'))

def passed(text):
    checks.append(text)
    print(text, flush=True)

def prepare_once(command):
    # WebKit can interrupt the driver reply when focus changes to the native
    # confirmation window. Observe the pending result; never repeat the mutation.
    try:
        execute("window.__TAURI_INTERNALS__.invoke('wallet_prepare',arguments[0]).then(id=>window.preparedOperation=id).catch(e=>window.prepareError=String(e));return true;", [command])
    except RemoteDisconnected:
        pass
    time.sleep(1)
    switch(wallet)
    for _ in range(30):
        pending = native('wallet_pending')
        if pending:
            return pending
        time.sleep(.1)
    raise AssertionError('No pending confirmation; mutation was not retried')

try:
    for _ in range(60):
        try:
            rpc('GET', '/status'); break
        except Exception:
            time.sleep(.2)
    binary = str(Path(sys.argv[1]).resolve()) if len(sys.argv) > 1 else str(ROOT / 'target/debug/agent-desktop')
    sid = rpc('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': binary}}}})['sessionId']
    wait_text('管理你的设备')
    main = rpc('GET', f'/session/{sid}/window')
    native('wallet_initialize', {'password': 'fixture-vault-password'}, 'not allowed')
    profile = native('add_profile', {'alias': '本地资产协议测试', 'connection': {'kind': 'https', 'origin': fixture.origin, 'ca_pem': fixture.pem, 'ca_sha256': fixture.fingerprint}})
    session = native('connect_device', {'profileId': profile['id'], 'sshSecret': ''})['id']
    native('login', {'sessionId': session, 'input': {'mode': 'password', 'username': 'tester', 'secret': 'fixture-password'}, 'remember': False})
    execute('location.reload();return true;')
    wait_text('本地资产协议测试')
    execute("window.__TAURI_INTERNALS__.invoke('wallet_open');return true;")
    time.sleep(1)
    wallet = next(h for h in rpc('GET', f'/session/{sid}/window/handles') if h != main)
    switch(wallet); wait_text('设置密钥库密码')
    native('wallet_initialize', {'password': 'fixture-vault-password'})
    account = native('wallet_create', {'name': '桌面测试账户 A'})
    second = native('wallet_create', {'name': '桌面测试账户 B'})
    assert account['public_key'] != second['public_key']
    assert set(account) == {'id', 'name', 'public_key', 'backed_up'}
    assert native('wallet_backup', {'accountId': account['id'], 'password': 'fixture-backup-password'})
    assert portal.calls == ['SaveFile'] and portal.path.exists()
    native('wallet_restore', {'password': 'fixture-backup-password', 'name': 'duplicate'}, 'wallet_account_duplicate')
    native('profiles', error='not allowed')
    time.sleep(1); screenshot('01-wallet-light')
    execute("document.querySelector('header button').click();return true;")
    time.sleep(.2); screenshot('02-wallet-dark')
    native('wallet_lock')
    passed('native OS keyring, unique key generation, encrypted backup and duplicate restore; main/wallet IPC isolation')
    switch(main)
    native('wallet_select', {'accountId': account['id']})
    base = {'sessionId': session, 'accountId': account['id'], 'service': 'assets'}
    data = native('wallet_read', {**base, 'page': None})
    assert data['account'] == account['public_key'] and data['aic_balance_units'] == '12345000000'
    assert native('wallet_read', {**base, 'page': 2})['page'] == 2
    assert api.public_reads and not api.verified and not api.challenges
    native('wallet_select', {'accountId': second['id']})
    native('wallet_read', {**base, 'page': None}, 'wallet_selection_changed')
    assert native('wallet_read', {**base, 'accountId': second['id'], 'page': None})['account'] == second['public_key']
    native('wallet_select', {'accountId': account['id']})
    transfer = dict(kind='transfer', asset='AIC', amount_units='100000000', recipient=second['public_key'], memo='测试 memo', max_fee_bps=0)
    api.tamper = ('recipient', account['public_key'])
    native('wallet_prepare', {**base, 'intent': transfer}, 'wallet_challenge_mismatch')
    api.tamper = None
    passed('locked and unbacked-up accounts read without signatures; pagination, selected-account isolation and recipient tampering rejection')
    for service, intent in [('assets', transfer), ('bancor', dict(kind='bancor_trade', side='buy', input_units='100000000', slippage_bps=300, max_fee_bps=100)), ('bancor', dict(kind='bancor_trade', side='sell', input_units='100000000', slippage_bps=300, max_fee_bps=100))]:
        command = {**base, 'service': service, 'intent': intent}
        pending = prepare_once(command)
        assert pending and pending['payload']['terms']['kind'] == intent['kind']
        wait_text('确认并签名提交'); screenshot('03-confirm-' + service + '-' + intent.get('side', 'transfer'))
        count = len(api.outcomes)
        if service == 'assets':
            native('wallet_confirm', {'operationId': pending['payload']['operation_id'], 'password': 'wrong-password'}, 'wallet_')
            assert len(api.outcomes) == count and native('wallet_pending') is not None
            assert not native('wallet_status')['unlocked']
            time.sleep(2.1)
            from wallet_shared_ui_e2e import keys
            keys(rpc, sid, 'input[aria-label="交易确认密码"]', 'fixture-vault-password')
            element = rpc('POST', f'/session/{sid}/element', {'using': 'css selector', 'value': '[data-wallet-confirm]'})
            rpc('POST', f"/session/{sid}/element/{element['element-6066-11e4-a52e-4f735466cecf']}/click", {})
            wait_text('操作已确认完成')
            assert api.outcomes[pending['payload']['operation_id']]['status'] == 'succeeded'
        else:
            time.sleep(2.1)
            assert native('wallet_confirm', {'operationId': pending['payload']['operation_id'], 'password': 'fixture-vault-password'})['status'] == 'succeeded'
        assert len(api.outcomes) == count + 1
        assert not native('wallet_status')['unlocked']
        native('wallet_confirm', {'operationId': pending['payload']['operation_id'], 'password': 'fixture-vault-password'}, 'wallet_confirmation_missing')
        switch(main)
    passed('native confirmation and real compact K1 signatures for transfer, Bancor buy and sell; double-submit blocked')
    api.drop_response = True
    pending = prepare_once({**base, 'intent': transfer})
    operation = pending['payload']['operation_id']
    time.sleep(2.1)
    native('wallet_confirm', {'operationId': operation, 'password': 'fixture-vault-password'}, 'wallet_outcome_unknown')
    switch(main)
    records = native('wallet_operations', {'sessionId': session, 'accountId': account['id']})
    assert next(r for r in records if r['operation_id'] == operation)['status'] == 'pending'
    native('wallet_prepare', {**base, 'intent': transfer}, 'wallet_unresolved_operation')
    assert native('wallet_check_operation', {**base, 'operationId': operation})['status'] == 'succeeded'
    passed('lost response keeps durable pending record; no automatic write retry; password-free status query resolves it')
    api.unsupported = True
    native('wallet_capabilities', {'sessionId': session, 'service': 'assets'}, 'wallet_backend_unsupported')
    api.unsupported = False
    # Assets may be reached through a contextual route in the shared console.
    try:
        execute("const key=Object.keys(localStorage).find(k=>k.endsWith('agent-runtime.monitor.currentPage'));localStorage.setItem(key,'assets');location.reload();return true;")
    except RemoteDisconnected:
        pass  # Observe the requested navigation instead of issuing it again.
    time.sleep(2)
    execute("const s=document.querySelector('select[aria-label=\"桌面资产账户\"]');s.value=arguments[0];s.dispatchEvent(new Event('change',{bubbles:true}));return true;", [account['id']])
    wait_text('123.45')
    from wallet_layout_e2e import run as layout_acceptance
    layout_acceptance(execute, rpc, sid, account, second, screenshot, 'assets')
    from wallet_shared_ui_e2e import transfer_form, bancor_form
    transfer_form(execute, rpc, sid, native, switch, main, wallet, second, fixture, screenshot)
    screenshot('04-local-assets')
    execute("[...document.querySelectorAll('[data-assets-page] header button')].find(b=>b.textContent.trim()==='交易').click();return true;")
    wait_text('BANCOR储备曲线市场')
    assert execute("return document.querySelector('select[aria-label=\"桌面资产账户\"]').value") == account['id']
    layout_acceptance(execute, rpc, sid, account, second, screenshot, 'bancor')
    bancor_form(execute, rpc, sid, native, switch, main, wallet, fixture, screenshot)
    screenshot('05-local-bancor')
    passed('account selectors stay in web overview/trading slots; hardware/local switching and full public-key clipboard copy in both themes and widths')
    passed('identical shared page structure; native signatures from shared transfer and Bancor forms with no legacy writes; SWAP and maximized chart controls')
    display_lib = ctypes.CDLL('libX11.so.6')
    display_lib.XOpenDisplay.restype = ctypes.c_void_p
    display_lib.XOpenDisplay.argtypes = [ctypes.c_char_p]
    display_lib.XDefaultRootWindow.argtypes = [ctypes.c_void_p]
    display_lib.XDefaultRootWindow.restype = ctypes.c_ulong
    display_lib.XCreateSimpleWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_int, ctypes.c_uint, ctypes.c_uint, ctypes.c_uint, ctypes.c_ulong, ctypes.c_ulong]
    display_lib.XCreateSimpleWindow.restype = ctypes.c_ulong
    display_lib.XMapWindow.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    display_lib.XSetInputFocus.argtypes = [ctypes.c_void_p, ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    display_lib.XSync.argtypes = [ctypes.c_void_p, ctypes.c_int]
    display_lib.XCloseDisplay.argtypes = [ctypes.c_void_p]
    display = display_lib.XOpenDisplay(None)
    assert display
    outside = display_lib.XCreateSimpleWindow(display, display_lib.XDefaultRootWindow(display), 0, 0, 200, 100, 0, 0, 0)
    display_lib.XMapWindow(display, outside)
    display_lib.XSetInputFocus(display, outside, 1, 0)
    display_lib.XSync(display, 0)
    time.sleep(1)
    assert not native('wallet_status')['unlocked']
    display_lib.XCloseDisplay(display)
    count = len(api.verified)
    assert native('wallet_read', {**base, 'page': None})['account'] == account['public_key']
    assert len(api.verified) == count
    assert all(p['terms']['kind'] in ('transfer', 'bancor_trade') for p in api.verified)
    passed('actual local-account Assets/Bancor UI, unsupported backend, focus lock preserves password-free reads; only financial writes signed')
    from wallet_deployed_e2e import run as deployed_acceptance
    if deployed_acceptance(native, account, second, OUT):
        passed('native desktop to deployed webd/gateway/Edge/Core: locked public reads and unfunded write rejection; no real funds touched')
    (OUT / 'acceptance.json').write_text(json.dumps({'checks': checks, 'signature_verifications': len(api.verified), 'public_reads': len(api.public_reads), 'simulated_file_picker': True}, ensure_ascii=False, indent=2))
    print('PASS ' + str(OUT), flush=True)
finally:
    if sid:
        try:
            rpc('DELETE', f'/session/{sid}')
        except Exception:
            pass
    os.killpg(driver.pid, signal.SIGTERM)
    xvfb.terminate()
    fixture.close()
