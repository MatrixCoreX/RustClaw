"""Direct connect UX, TLS rejection, duplicate clicks and SSH credential boundary."""


def run(execute, native, wait, button, fill, screenshot, checks, fixture, trusted):
    untrusted = native('add_profile', {'alias': 'Untrusted certificate', 'connection': {
        'kind': 'https', 'origin': fixture.origin, 'ca_pem': None, 'ca_sha256': None}})
    ssh = native('add_profile', {'alias': 'SSH credentials', 'connection': {
        'kind': 'ssh', 'host': '127.0.0.1', 'port': 1, 'username': 'fixture',
        'host_key_sha256': 'SHA256:' + 'a' * 43, 'webd_port': 8788}})
    execute('location.reload();return true;')
    wait('return document.querySelectorAll("article").length===4')

    def click_connect(profile):
        # Two events in the same JavaScript turn also test the synchronous guard,
        # before React has committed the disabled button to the DOM.
        execute('''const b=[...document.querySelectorAll('article')].find(a=>a.querySelector('h2').textContent===arguments[0]).querySelector('button:last-child');
          b.dispatchEvent(new MouseEvent('click',{bubbles:true}));
          b.dispatchEvent(new MouseEvent('click',{bubbles:true}));return true;''', [profile['alias']])

    before = list(fixture.requests)
    click_connect(untrusted)
    wait('return !!document.querySelector(".desktop-error") && [...document.querySelectorAll("article button")].every(b=>!b.disabled)')
    assert '安全连接未建立' in execute('return document.querySelector(".desktop-error").textContent')
    assert native('current_session') is None
    assert fixture.requests == before, 'untrusted TLS must not reach HTTP or login handlers'
    assert not execute('return !!document.querySelector("form")')
    bounds = execute('const e=document.querySelector(".desktop-error"),r=e.getBoundingClientRect();return {top:r.top,bottom:r.bottom,height:innerHeight,alias:e.closest("article")?.querySelector("h2").textContent};')
    assert 0 <= bounds['top'] < bounds['bottom'] <= bounds['height'], bounds
    assert bounds['alias'] == untrusted['alias'], bounds
    screenshot('01-certificate-failure-stays-in-list')
    checks.append('one click verifies TLS; untrusted certificate fails before login with no HTTP fallback and leaves a retryable device list')

    # Hold the actual fixture response while inspecting the pending UI. No IPC
    # replacement or timer assumption: native certificate validation still runs.
    import threading
    fixture.bootstrap_gate = threading.Event()
    try:
        click_connect(trusted)
        wait('return document.body.textContent.includes("正在连接…")')
        assert not execute('return !!document.querySelector("form")')
        state = execute('return [...document.querySelectorAll("article button,.desktop-list-head button")].map(b=>({text:b.textContent,disabled:b.disabled}))')
        assert all(b['disabled'] for b in state), state
        screenshot('00-connecting-in-device-list')
    finally:
        fixture.bootstrap_gate.set()
    wait('return !!document.querySelector(".desktop-login-form .primary") && !document.querySelector(".desktop-login-form .primary").disabled')
    assert native('current_session')['profile']['id'] == trusted['id']
    requests = fixture.requests[len(before):]
    assert requests == [('GET', '/webd/session', 0)], requests
    assert not execute('return !!document.querySelector(".desktop-error") || document.body.textContent.includes("建立安全连接")')
    screenshot('02-direct-login-after-verification')
    checks.append('duplicate clicks make one connection; pending actions disabled; trusted HTTPS enters login directly, clears prior error and never logs in automatically')

    button('断开')
    wait('return !!document.querySelector(".desktop-device-list")')
    click_connect(ssh)
    wait('return document.body.textContent.includes("SSH 密码")')
    assert native('current_session') is None
    fill('input[type=password]', 'disposable-ssh-input')
    screenshot('03-ssh-credential-form')
    button('返回')
    click_connect(ssh)
    assert execute('return document.querySelector("input[type=password]").value') == ''
    button('返回')
    native('forget_profile', {'profileId': untrusted['id']})
    native('forget_profile', {'profileId': ssh['id']})
    checks.append('SSH still requests credentials before connecting; returning to list clears the SSH secret')
