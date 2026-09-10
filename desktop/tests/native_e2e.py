"""Native WebKitGTK / WebView2 + Tauri IPC against a loopback TLS fixture."""
import base64
import json
import os
import re
from pathlib import Path
import subprocess
import signal
import sys
import time
import urllib.request
from fixture_server import Fixture

ROOT = Path(__file__).resolve().parents[1]
WINDOWS = sys.platform == "win32"
if WINDOWS and os.environ.get("GITHUB_ACTIONS") != "true":
    raise SystemExit("windows_native_e2e_requires_disposable_ci_runner")
OUT = ROOT / "test-results" / ("native-platform/e2e" if WINDOWS else "native")
OUT.mkdir(parents=True, exist_ok=True)
FIXTURE = Fixture(OUT / "tls")
BASE = "http://127.0.0.1:4447"


def rpc(method, path, data=None):
    print(method, path, flush=True)
    request = urllib.request.Request(BASE + path, data=None if data is None else json.dumps(data).encode(), method=method, headers={"Content-Type": "application/json"})
    with urllib.request.urlopen(request, timeout=60) as response:
        result = json.load(response)["value"]
    if isinstance(result, dict) and isinstance(result.get("error"), str) and "message" in result:
        raise AssertionError(result)
    return result


env = dict(os.environ, GDK_BACKEND="x11", LIBGL_ALWAYS_SOFTWARE="1", WEBKIT_DISABLE_DMABUF_RENDERER="1", TAURI_WEBVIEW_AUTOMATION="true", XDG_DATA_HOME=str(OUT / "data"), XDG_CONFIG_HOME=str(OUT / "config"), XDG_CACHE_HOME=str(OUT / "cache"))
if WINDOWS:
    driver_command = [str(ROOT / ".build/tools/bin/tauri-driver.exe"), "--port", "4447", "--native-port", "4448", "--native-driver", str(ROOT / ".build/tools/msedgedriver.exe")]
else:
    driver_command = ["xvfb-run", "-a", str(ROOT / ".build/tools/bin/tauri-driver"), "--port", "4447", "--native-port", "4448", "--native-driver", "/usr/bin/WebKitWebDriver"]
driver = subprocess.Popen(driver_command, env=env, stdout=(OUT / "driver.log").open("w"), stderr=subprocess.STDOUT, start_new_session=not WINDOWS)
session_id = None
checks = []
measurements = {}
def sample_processes():
    if WINDOWS:
        return {"unavailable": "linux_process_sampler"}
    rows = [line.split(None, 4) for line in subprocess.check_output(["ps", "-e", "-o", "pid=,ppid=,rss=,pcpu=,comm="], text=True).splitlines()]
    owned = {driver.pid}
    for _ in range(8):
        owned.update(int(row[0]) for row in rows if int(row[1]) in owned)
    relevant = [row for row in rows if int(row[0]) in owned and ("agent-desktop" in row[4] or "WebKit" in row[4]) and not row[4].startswith("WebKitWebDr")]
    return {"rss_kib":sum(int(row[2]) for row in relevant), "processes":len(relevant)}
try:
    for _ in range(50):
        try:
            rpc("GET", "/status")
            break
        except Exception:
            time.sleep(0.2)
    binary = str(Path(sys.argv[1]).resolve()) if len(sys.argv) > 1 else str(ROOT / "target/debug/agent-desktop")
    started = time.monotonic()
    session_id = rpc("POST", "/session", {"capabilities": {"alwaysMatch": {"tauri:options": {"application": binary}}}})["sessionId"]
    def execute(script, args=None, asynchronous=False):
        return rpc("POST", f"/session/{session_id}/execute/{'async' if asynchronous else 'sync'}", {"script": script, "args": args or []})
    def native(command, args=None, expect_error=False):
        result = execute("const done=arguments[arguments.length-1]; window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>done({ok:true,value})).catch(error=>done({ok:false,error:String(error)}));", [command, args or {}], True)
        if expect_error:
            assert not result["ok"], result
            return result["error"]
        assert result["ok"], result
        return result.get("value")
    def screenshot(name):
        (OUT / f"{name}.png").write_bytes(base64.b64decode(rpc("GET", f"/session/{session_id}/screenshot")))
    def wait_text(text):
        for _ in range(100):
            if text in execute("return document.body.innerText"):
                return
            time.sleep(.1)
        raise AssertionError(f"Missing visible text: {text}; {execute('return document.body.innerText')[:1000]}")
    wait_text("管理你的设备")
    measurements["startup_seconds"] = round(time.monotonic() - started, 3)
    measurements["home"] = sample_processes()
    initial_theme = execute("return document.documentElement.dataset.theme")
    execute("document.querySelector('.desktop-header button').click();return true;")
    time.sleep(.1)
    assert execute("return document.documentElement.dataset.theme") != initial_theme
    screenshot("00-alternate-theme")
    execute("document.querySelector('.desktop-header button').click();return true;")
    for previous in native("profiles"):
        native("forget_profile", {"profileId": previous["id"]})
    execute("location.reload();return true;")
    wait_text("管理你的设备")
    screenshot("01-device-home")
    checks.append("native WebView startup and device home")
    from discovery_e2e import run as discovery_acceptance
    discovery_acceptance(execute, native, screenshot, checks, wait_text)
    native("add_profile", {"alias": "Rejected HTTP", "connection": {"kind": "https", "origin": "http://192.168.1.2", "ca_pem": None, "ca_sha256": None}}, True)
    checks.append("LAN HTTP rejected before credentials")
    profile = native("add_profile", {"alias": "Ubuntu 协议测试设备", "connection": {"kind": "https", "origin": FIXTURE.origin, "ca_pem": FIXTURE.pem, "ca_sha256": FIXTURE.fingerprint}})
    info = native("connect_device", {"profileId": profile["id"], "sshSecret": ""})
    auth = native("login", {"sessionId": info["id"], "input": {"mode": "password", "username": "tester", "secret": "fixture-password"}, "remember": False})
    assert auth["session"]["identity"]["role"] == "admin"
    assert "user_key" not in auth["session"]["identity"]
    checks.append("HTTPS certificate pairing and cookie login with native credential redaction")
    transfer = native("request_start", {"sessionId": info["id"], "spec": {"path": "/v1/events", "method": "GET", "headers": {}, "has_body": False}})
    head = native("request_headers", {"sessionId": info["id"], "id": transfer})
    assert head["headers"]["content-type"] == "text/event-stream"
    start = time.monotonic()
    first = execute("const done=arguments[arguments.length-1]; window.__TAURI_INTERNALS__.invoke('request_read',{sessionId:arguments[0],id:arguments[1]}).then(b=>done(new TextDecoder().decode(new Uint8Array(b)))).catch(e=>done(String(e)));", [info["id"], transfer], True)
    assert "first" in first and time.monotonic() - start < .7, first
    measurements["sse_first_read_seconds"] = round(time.monotonic() - start, 4)
    native("request_cancel", {"sessionId": info["id"], "id": transfer})
    checks.append("real binary IPC SSE first frame arrives before stream completion")
    for header in ["Host", "Cookie", "X-Agent-Key", "Origin", "X-Forwarded-Proto"]:
        native("request_start", {"sessionId": info["id"], "spec": {"path": "/v1/health", "method": "GET", "headers": {header: "blocked"}, "has_body": False}}, True)
    checks.append("renderer authentication and proxy header injection rejected")
    # Compile the actual production adapter for WebDriver injection, never ship a test IPC.
    transport_source = subprocess.run(["node", "-e", "process.stdout.write(require('../UI/node_modules/esbuild').buildSync({entryPoints:['frontend/transport.ts'],bundle:true,format:'iife',globalName:'DesktopTransport',write:false}).outputFiles[0].text)"], cwd=ROOT, check=True, text=True, capture_output=True).stdout
    execute(transport_source + ";window.desktopTestTransport=DesktopTransport;return true;")
    upload_started = time.monotonic()
    upload = execute("""const done=arguments[arguments.length-1]; const sessionId=arguments[0];
      (async()=>{const call=(cmd,args,opts)=>window.__TAURI_INTERNALS__.invoke(cmd,args,opts);
      const form=new FormData();form.append('file',new Blob([new Uint8Array(190000)]),'附件.txt');
      const fetch=window.desktopTestTransport.createTransport(call,sessionId,arguments[1],async()=>{});
      const response=await fetch('/v1/echo',{method:'POST',body:form});
      return (await response.json()).data.bytes;})().then(done).catch(e=>done(String(e)));""", [info["id"], FIXTURE.origin], True)
    assert isinstance(upload, int) and upload > 190000, upload
    measurements["upload"] = {"bytes":upload, "seconds":round(time.monotonic() - upload_started, 3)}
    checks.append("native WebView FormData uploads through bounded binary IPC")
    execute("location.reload(); return true;")
    wait_text("Ubuntu 协议测试设备")
    time.sleep(3)
    screenshot("02-shared-console")
    measurements["console"] = sample_processes()
    checks.append("shared React console mounted with selected device and role")
    from layout_e2e import run as layout_acceptance
    def resize(width, height):
        rpc("POST", f"/session/{session_id}/window/rect", {"width": width, "height": height})
        time.sleep(.3)
    def click(selector):
        element = rpc("POST", f"/session/{session_id}/element", {"using": "css selector", "value": selector})
        rpc("POST", f"/session/{session_id}/element/{element['element-6066-11e4-a52e-4f735466cecf']}/click", {})
    def keys(selector, text):
        element = rpc("POST", f"/session/{session_id}/element", {"using": "css selector", "value": selector})
        rpc("POST", f"/session/{session_id}/element/{element['element-6066-11e4-a52e-4f735466cecf']}/value", {"text": text})
    layout_acceptance(execute, resize, click, keys, screenshot, OUT, checks)
    expected_pages = re.findall(r'"([a-z_]+)"', re.search(r'const CONSOLE_PAGES: ConsolePage\[\] = \[([^\]]+)\]', (ROOT.parent / "UI/src/App.tsx").read_text())[1])
    pages = execute("return [...document.querySelectorAll('[data-desktop-page]')].map(b=>b.dataset.desktopPage)")
    assert set(pages) <= set(expected_pages) and len(pages) >= 10, pages
    for page in pages:
        execute("document.querySelector('[data-desktop-page=\"'+arguments[0]+'\"]').click();return true;", [page])
        time.sleep(.3)
        assert execute("return document.querySelector('[data-nav-active=true]')?.dataset.desktopPage") == page, page
        assert execute("return document.querySelector('.desktop-console').innerText.length") > 100, page
    # Existing console places some pages under contextual links. Exercise their
    # original persisted-route restoration, in addition to visible navigation.
    for page in set(expected_pages) - set(pages):
        execute("const key=Object.keys(localStorage).find(k=>k.includes(arguments[0])&&k.endsWith('agent-runtime.monitor.currentPage'));if(!key)throw Error('route key missing');localStorage.setItem(key,arguments[1]);location.reload();return true;", [profile["id"], page])
        wait_text("Ubuntu 协议测试设备")
        time.sleep(1)
        assert execute("return document.querySelector('.desktop-console')?.innerText.length??0") > 100, page
        screenshot(f"page-{page}")
    checks.append(f"all {len(expected_pages)} shared pages mount via navigation or existing saved routes")
    media_url = native("media_open", {"sessionId": info["id"], "path": "/v1/tasks/task/artifacts/file/content"})
    execute("const video=document.createElement('video');video.id='desktop-media-test';video.muted=true;video.preload='auto';video.src=arguments[0];document.body.append(video);return true;", [media_url])
    media_state = None
    for _ in range(100):
        media_state = execute("const v=document.getElementById('desktop-media-test');return {ready:v.readyState,error:v.error?.code??null,message:v.error?.message??'',duration:v.duration,mp4:v.canPlayType('video/mp4; codecs=\"avc1.42E01E\"')};")
        if media_state["ready"] >= 2 or media_state["error"]:
            break
        time.sleep(.1)
    if media_state["ready"] < 2:
        print("MEDIA FAILURE", media_state, flush=True)
        execute("const bytes=Uint8Array.from(atob(arguments[0]),c=>c.charCodeAt(0));document.getElementById('desktop-media-test').src=URL.createObjectURL(new Blob([bytes],{type:'video/mp4'}));return true;", [base64.b64encode(FIXTURE.media).decode()])
        time.sleep(2)
        print("BLOB COMPARISON", execute("const v=document.getElementById('desktop-media-test');return {ready:v.readyState,error:v.error?.code??null};"), flush=True)
    assert media_state["ready"] >= 2, (media_state, FIXTURE.requests)
    assert len(FIXTURE.media) > 2 * 1024 * 1024
    seek = execute("const done=arguments[arguments.length-1];const v=document.getElementById('desktop-media-test');v.addEventListener('seeked',()=>done({time:v.currentTime,ready:v.readyState}),{once:true});v.currentTime=9;", asynchronous=True)
    assert seek["time"] >= 9 and seek["ready"] >= 2, seek
    played = execute("const done=arguments[arguments.length-1];const v=document.getElementById('desktop-media-test');v.play().then(()=>setTimeout(()=>done(v.currentTime),400)).catch(e=>done(String(e)));", asynchronous=True)
    assert isinstance(played, (int, float)) and played > 9, played
    measurements["media"] = {**sample_processes(), "fixture_bytes":len(FIXTURE.media)}
    with urllib.request.urlopen(urllib.request.Request(media_url, headers={"Range":"bytes=100-199"}), timeout=5) as part:
        assert part.status == 206 and part.read() == FIXTURE.media[100:200]
    assert "bytes=100-199" in FIXTURE.ranges
    execute("document.getElementById('desktop-media-test').remove();return true;")
    for overrides in [{"Origin":"https://untrusted.invalid"}, {"Host":"untrusted.invalid"}]:
        try:
            urllib.request.urlopen(urllib.request.Request(media_url, headers=overrides), timeout=5)
            raise AssertionError("Media relay accepted an untrusted origin/host")
        except urllib.error.HTTPError as error:
            assert error.code == 403
    native("media_close", {"url": media_url})
    try:
        urllib.request.urlopen(media_url, timeout=5)
        raise AssertionError("Revoked media token was accepted")
    except urllib.error.HTTPError as error:
        assert error.code == 403
    checks.append("authenticated loopback MP4 preview, seeking, origin/host checks and token revocation")
    # WebKit may change its current browsing context while an async script opens a window.
    # Initiate as a real click would, then select the new context explicitly.
    main_handle = rpc("GET", f"/session/{session_id}/window")
    execute("window.__TAURI_INTERNALS__.invoke('aipp_open', arguments[0]).catch(e=>{window.aippOpenError=String(e)});return true;", [{"sessionId": info["id"], "skillName": "protocol_fixture", "locale": "zh"}])
    time.sleep(2)
    handles = rpc("GET", f"/session/{session_id}/window/handles")
    assert len(handles) == 2, handles
    other = next(handle for handle in handles if handle != main_handle)
    rpc("POST", f"/session/{session_id}/window", {"handle": other})
    native("profiles", expect_error=True)
    native("discover_devices", {"scanSubnet": True}, True)
    native("cancel_discovery", expect_error=True)
    native("media_open", {"sessionId":info["id"], "path":"/v1/tasks/task/artifacts/file/content"}, True)
    native("request_start", {"sessionId": info["id"], "spec": {"path": "/v1/health", "method": "GET", "headers": {}, "has_body": False}}, True)
    native("aipp_bridge", {"capability": "not.granted", "args": {}}, True)
    assert execute("return document.querySelector('iframe') !== null")
    frame = rpc("POST", f"/session/{session_id}/element", {"using": "css selector", "value": "iframe"})
    rpc("POST", f"/session/{session_id}/frame", {"id": frame})
    wait_text("Isolated skill fixture")
    bridge = execute("const done=arguments[arguments.length-1];const id='fixture-roundtrip';window.addEventListener('message',function reply(e){if(e.data?.type==='aipp.capability.result'&&e.data?.request_id===id){window.removeEventListener('message',reply);done(e.data);}});window.parent.postMessage({schema_version:1,type:'aipp.capability.invoke',request_id:id,capability:'fixture.read',args:{}},'*');", asynchronous=True)
    assert bridge["ok"] and bridge["data"]["status"] == "succeeded", bridge
    blocked = execute("const done=arguments[arguments.length-1]; if (!window.__TAURI_INTERNALS__) {done(true); return;} window.__TAURI_INTERNALS__.invoke('profiles').then(()=>done(false)).catch(()=>done(true));", asynchronous=True)
    assert blocked
    rpc("POST", f"/session/{session_id}/frame", {"id": None})
    screenshot("03-isolated-aipp")
    checks.append("isolated AiAPP bridge round trip succeeds; main IPC and undeclared capabilities denied")
    rpc("POST", f"/session/{session_id}/window", {"handle": main_handle})
    native("disconnect_device")
    native("request_start", {"sessionId": info["id"], "spec": {"path": "/v1/health", "method": "GET", "headers": {}, "has_body": False}}, True)
    info2 = native("connect_device", {"profileId": profile["id"], "sshSecret": ""})
    assert info2["id"] != info["id"]
    native("login", {"sessionId": info2["id"], "input": {"mode": "key", "username": "", "secret": "fixture-key"}, "remember": False})
    checks.append("key login and connection generation isolation")
    native("disconnect_device")
    native("forget_profile", {"profileId": profile["id"], "deleteSavedLogin": False})
    assert not native("profiles")
    checks.append("forget profile removes local trust without remote task operations")
    # Exercise the visible password form with the actual WEBD 32-character token contract.
    form_profile = native("add_profile", {"alias": "Login form regression", "connection": {"kind": "https", "origin": FIXTURE.origin, "ca_pem": FIXTURE.pem, "ca_sha256": FIXTURE.fingerprint}})
    execute("location.reload();return true;")
    wait_text("Login form regression")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='连接').click();return true;")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='建立安全连接').click();return true;")
    wait_text("加密连接已建立")
    for selector, value in [("input[autocomplete=username]", "tester"), ("input[type=password]", "fixture-password")]:
        element = rpc("POST", f"/session/{session_id}/element", {"using": "css selector", "value": selector})
        element_id = element["element-6066-11e4-a52e-4f735466cecf"]
        rpc("POST", f"/session/{session_id}/element/{element_id}/value", {"text": value})
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='登录设备').click();return true;")
    for _ in range(100):
        if execute("return !!document.querySelector('.desktop-console')"):
            break
        time.sleep(.1)
    else:
        raise AssertionError("Password form did not enter console: " + execute("return document.body.innerText"))
    form_session = native("current_session")
    assert form_session["identity"]["role"] == "admin"
    assert "user_key" not in form_session["identity"]
    screenshot("06-password-login-regression")
    native("disconnect_device")
    native("forget_profile", {"profileId": form_profile["id"]})
    checks.append("visible password login form enters shared console with real 32-character CSRF contract")
    from local_e2e import run as local_acceptance
    local_acceptance(execute, native, screenshot, checks, wait_text, FIXTURE.local_origin)
    (OUT / "report.json").write_text(json.dumps({"ok": True, "binary":binary, "checks": checks, "measurements":measurements, "requests": FIXTURE.requests}, ensure_ascii=False, indent=2))
    print(json.dumps({"ok": True, "checks": checks}, ensure_ascii=False, indent=2))
finally:
    if session_id:
        try:
            rpc("DELETE", f"/session/{session_id}")
        except Exception:
            pass
    if WINDOWS:
        driver.terminate()
    else:
        os.killpg(driver.pid, signal.SIGTERM)
    FIXTURE.close()
