"""Saved-login UX against an isolated OS keyring and a loopback TLS device."""
import base64
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
OUT = Path(os.environ.get('DESKTOP_TEST_OUTPUT_DIR', ROOT / 'test-results')) / ('saved-login-' + uuid.uuid4().hex[:8])
OUT.mkdir(parents=True)
assert os.environ.get('DBUS_SESSION_BUS_ADDRESS'), 'run inside a disposable dbus-run-session'
os.environ.update(XDG_DATA_HOME=str(OUT / 'data'), XDG_CONFIG_HOME=str(OUT / 'config'),
    XDG_CACHE_HOME=str(OUT / 'cache'), GDK_BACKEND='x11', LIBGL_ALWAYS_SOFTWARE='1',
    WEBKIT_DISABLE_DMABUF_RENDERER='1', TAURI_WEBVIEW_AUTOMATION='true')
xvfb = subprocess.Popen(['Xvfb', '-displayfd', '1', '-screen', '0', '1280x900x24'], stdout=subprocess.PIPE, stderr=(OUT / 'xvfb.log').open('w'))
os.environ['DISPLAY'] = ':' + xvfb.stdout.readline().decode().strip()
subprocess.run(['dbus-update-activation-environment', 'DISPLAY', 'XDG_DATA_HOME', 'XDG_CONFIG_HOME'], check=True)
subprocess.run(['gnome-keyring-daemon', '--unlock', '--components=secrets'], input=b'fixture-keyring-password', check=True, stdout=subprocess.DEVNULL)
from fixture_server import Fixture
fixture = Fixture(OUT / 'tls')
driver = subprocess.Popen([str(ROOT / '.build/tools/bin/tauri-driver'), '--port', '4467', '--native-port', '4468', '--native-driver', '/usr/bin/WebKitWebDriver'], stdout=(OUT / 'driver.log').open('w'), stderr=subprocess.STDOUT, start_new_session=True)
sid = None
checks = []


def rpc(method, path, data=None):
    request = urllib.request.Request('http://127.0.0.1:4467' + path, data=None if data is None else json.dumps(data).encode(), method=method, headers={'Content-Type': 'application/json'})
    with urllib.request.urlopen(request, timeout=40) as response:
        value = json.load(response)['value']
    if isinstance(value, dict) and 'error' in value and 'message' in value:
        raise AssertionError(value)
    return value


def execute(script, args=None, asynchronous=False):
    return rpc('POST', f'/session/{sid}/execute/{"async" if asynchronous else "sync"}', {'script': script, 'args': args or []})


def native(command, args=None, error=False):
    result = execute('const done=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(e=>done({ok:false,error:String(e)}));', [command, args or {}], True)
    assert result['ok'] != error, result
    return result.get('error' if error else 'value')


def wait(script):
    for _ in range(100):
        try:
            if execute(script):
                return
        except RemoteDisconnected:
            pass  # Read-only polling may cross a WebView reload.
        time.sleep(.1)
    raise AssertionError('UI did not reach expected state: ' + script)


def button(text):
    try:
        execute('[...document.querySelectorAll("button")].find(b=>b.textContent.trim()===arguments[0]).click();return true;', [text])
    except RemoteDisconnected:
        pass  # Observe the result below; never repeat a submitted click.


def fill(selector, value, clear=True):
    element = rpc('POST', f'/session/{sid}/element', {'using': 'css selector', 'value': selector})['element-6066-11e4-a52e-4f735466cecf']
    rpc('POST', f'/session/{sid}/element/{element}/click', {})
    if clear:
        rpc('POST', f'/session/{sid}/element/{element}/clear', {})
    rpc('POST', f'/session/{sid}/element/{element}/value', {'text': value})


def login_count():
    return sum(method == 'POST' and path in ('/webd/login', '/v1/auth/ui-key/verify') for method, path, _ in fixture.requests)


def connect(profile):
    if native('current_session'):
        button('切换设备 / 断开' if execute('return !!document.querySelector(".desktop-console")') else '断开')
        wait('return !!document.querySelector(".desktop-device-list")')
        assert native('current_session') is None
    execute('location.reload();return true;')
    wait('return !!document.querySelector(".desktop-device-list")')
    execute('[...document.querySelectorAll("article")].find(a=>a.querySelector("h2").textContent===arguments[0]).querySelector("button:last-child").click();return true;', [profile['alias']])
    wait('return !!document.querySelector("form .primary") && !document.querySelector("form .primary").disabled && document.querySelector("form .primary").textContent==="登录"')
    execute('window.scrollTo(0,0);return true;')
    assert not execute('return document.body.textContent.includes("使用已保存的登录")')
    return native('current_session')


def login():
    count = login_count()
    button('登录')
    wait('return !!document.querySelector(".desktop-console")')
    assert login_count() == count + 1


def screenshot(name):
    (OUT / (name + '.png')).write_bytes(base64.b64decode(rpc('GET', f'/session/{sid}/screenshot')))


try:
    for _ in range(60):
        try:
            rpc('GET', '/status')
            break
        except Exception:
            time.sleep(.1)
    binary = str(Path(sys.argv[1]).resolve())
    sid = rpc('POST', '/session', {'capabilities': {'alwaysMatch': {'tauri:options': {'application': binary}}}})['sessionId']
    wait('return document.body.textContent.includes("管理你的设备")')
    connection = {'kind': 'https', 'origin': fixture.origin, 'ca_pem': fixture.pem, 'ca_sha256': fixture.fingerprint}
    first = native('add_profile', {'alias': 'Saved login device', 'connection': connection})
    second = native('add_profile', {'alias': 'Separate device', 'connection': connection})
    from direct_connect_e2e import run as direct_connect_acceptance
    direct_connect_acceptance(execute, native, wait, button, fill, screenshot, checks, fixture, first)
    info = connect(first)
    assert native('login_prefill', {'sessionId': info['id']}) is None
    fill('input[autocomplete=username]', 'tester')
    fill('input[type=password]', 'fixture-password')
    execute('document.querySelector("input[type=checkbox]").click();return true;')
    login()
    assert next(p for p in native('profiles') if p['id'] == first['id'])['saved_login']
    native('login_prefill', {'sessionId': info['id']}, error=True)
    checks.append('manual login saves credentials in isolated native OS keyring; authenticated console cannot read prefill')

    count = login_count()
    info = connect(first)
    assert native('login_prefill', {'sessionId': info['id']}) == {'mode': 'password', 'username': 'tester'}
    assert execute('return document.querySelector("input[autocomplete=username]").value') == 'tester'
    mask = execute('return document.querySelector("input[type=password]").value')
    assert mask and mask != 'fixture-password'
    assert login_count() == count, 'prefill must not automatically log in'
    screenshot('01-password-prefilled-light')
    bounds = execute('const r=document.querySelector("form .primary").getBoundingClientRect();return {top:r.top,bottom:r.bottom,height:innerHeight};')
    assert bounds['top'] >= 0 and bounds['bottom'] <= bounds['height'], bounds
    execute('document.querySelector(".desktop-header button").click();return true;')
    screenshot('02-password-prefilled-dark')
    assert execute('return !Object.values(localStorage).some(v=>v.includes("fixture-password"))')
    login()
    checks.append('saved username and masked password fill automatically; one Login button, explicit click required, no secret in IPC or browser storage')

    info = connect(first)
    fill('input[autocomplete=username]', 'different-user')
    assert execute('return document.querySelector("input[type=password]").value') == ''
    count = login_count()
    button('登录')
    assert login_count() == count, 'editing username must invalidate the saved password'
    connect(first)
    fill('input[type=password]', 'fixture-wrong', clear=False)
    assert execute('return document.querySelector("input[type=password]").value') == 'fixture-wrong'
    button('登录')
    wait('return !!document.querySelector(".desktop-error") && !document.querySelector("form .primary").disabled')
    assert native('current_session')['identity'] is None
    fill('input[type=password]', 'fixture-password')
    login()
    checks.append('changing username clears retained password; edited password is sent, failed login remains editable, corrected login succeeds')

    connect(first)
    button('用户 Key')
    assert execute('return document.querySelector("input[type=password]").value') == ''
    fill('input[type=password]', 'fixture-key')
    login()
    info = connect(first)
    assert native('login_prefill', {'sessionId': info['id']}) == {'mode': 'key', 'username': ''}
    assert not execute('return !!document.querySelector("input[autocomplete=username]")')
    assert execute('return document.querySelector("input[type=password]").value') not in ('', 'fixture-key')
    screenshot('03-key-prefilled')
    login()
    checks.append('switching authentication mode clears old secret; saved Key mode also fills and uses the same Login button')

    previous = info['id']
    info = connect(second)
    assert native('login_prefill', {'sessionId': info['id']}) is None
    assert execute('return document.querySelector("input[autocomplete=username]").value') == ''
    assert execute('return document.querySelector("input[type=password]").value') == ''
    assert 'stale_connection' in native('login_prefill', {'sessionId': previous}, error=True)
    # A failed remembered login may leave the native profile reference without
    # a keyring entry. It must still show a usable empty manual login form.
    native('login', {'sessionId': info['id'], 'input': {'mode': 'password', 'username': 'tester', 'secret': 'fixture-wrong'}, 'remember': True}, error=True)
    info = connect(second)
    assert native('login_prefill', {'sessionId': info['id']}) is None
    assert execute('return document.querySelector("input[type=password]").value') == ''
    checks.append('missing saved credential entry falls back to an empty editable login form')
    main = rpc('GET', f'/session/{sid}/window')
    native('wallet_open')
    for _ in range(60):
        handles = rpc('GET', f'/session/{sid}/window/handles')
        if len(handles) > 1:
            break
        time.sleep(.1)
    wallet = next(h for h in handles if h != main)
    rpc('POST', f'/session/{sid}/window', {'handle': wallet})
    wait('return !!window.__TAURI_INTERNALS__')
    native('login_prefill', {'sessionId': info['id']}, error=True)
    rpc('POST', f'/session/{sid}/window', {'handle': main})
    native('disconnect_device')
    native('forget_profile', {'profileId': first['id']})
    assert all(p['id'] != first['id'] for p in native('profiles'))
    checks.append('credentials remain scoped to immutable device profile; stale sessions and wallet window denied; forgetting profile still works')
    (OUT / 'report.json').write_text(json.dumps({'ok': True, 'binary': binary, 'checks': checks}, ensure_ascii=False, indent=2))
    print('PASS ' + str(OUT), flush=True)
finally:
    if sid:
        try:
            rpc('DELETE', f'/session/{sid}')
        except Exception:
            pass
    os.killpg(driver.pid, signal.SIGTERM)
    fixture.close()
    xvfb.terminate()
