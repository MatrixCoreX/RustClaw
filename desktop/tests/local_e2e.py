"""Visible loopback-only HTTP setup and login, using the test server."""
import time


def run(execute, native, screenshot, checks, wait_text, origin):
    execute("location.reload();return true;")
    wait_text("管理你的设备")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent.trim()==='添加设备').click();return true;")
    for _ in range(80):
        if execute("return document.querySelector('.desktop-discovery')?.getAttribute('aria-busy')==='false'"):
            break
        time.sleep(.1)
    else:
        raise AssertionError("Local discovery did not finish")
    assert execute("return !!document.querySelector('.desktop-local-status')")
    checks.append("native discovery reports local installation and running-service status")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='本机 HTTP').click();return true;")
    assert execute("return document.querySelector('input[placeholder=\"http://127.0.0.1:8788\"]').value") == 'http://127.0.0.1:8788'
    execute("""const field=document.querySelector('input[placeholder="http://127.0.0.1:8788"]');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(field,arguments[0]);
      field.dispatchEvent(new Event('input',{bubbles:true}));return true;""", [origin])
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='保存设备').click();return true;")
    wait_text("连接本机服务")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='连接本机服务').click();return true;")
    wait_text("本机连接已建立")
    assert '加密连接已建立' not in execute("return document.body.innerText")
    for selector, value in [('input[autocomplete=username]', 'tester'), ('input[type=password]', 'fixture-password')]:
        execute("""const field=document.querySelector(arguments[0]);
          Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set.call(field,arguments[1]);
          field.dispatchEvent(new Event('input',{bubbles:true}));return true;""", [selector, value])
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='登录设备').click();return true;")
    for _ in range(100):
        if execute("return !!document.querySelector('.desktop-console')"):
            break
        time.sleep(.1)
    else:
        raise AssertionError('Local password login did not enter console: ' + execute('return document.body.innerText'))
    session = native('current_session')
    assert session['profile']['connection']['kind'] == 'local'
    assert session['origin'] == origin and session['identity']['role'] == 'admin'
    assert '本机 HTTP' in execute("return document.querySelector('.desktop-device-bar').innerText")
    screenshot('15-local-http-console')
    native('disconnect_device')
    native('forget_profile', {'profileId': session['profile']['id']})
    checks.append('visible local HTTP setup, password login, local connection label and disconnect')
