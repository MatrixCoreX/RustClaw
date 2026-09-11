"""Shared browser forms must dispatch local account operations only to native signing."""
from http.client import RemoteDisconnected
import time


def wait(execute, script):
    for _ in range(100):
        if execute(script):
            return
        time.sleep(.1)
    raise AssertionError(script + ': ' + execute('return document.body.innerText')[:1800])


def click(execute, selector):
    try:
        execute('document.querySelector(arguments[0]).click();return true;', [selector])
    except RemoteDisconnected:
        pass  # Observe completion; never dispatch a second write.


def text_button(execute, text):
    try:
        execute("[...document.querySelectorAll('button')].find(b=>b.textContent.trim()===arguments[0]).click();return true;", [text])
    except RemoteDisconnected:
        pass


def keys(rpc, sid, selector, value):
    element = rpc('POST', f'/session/{sid}/element', {'using': 'css selector', 'value': selector})
    rpc('POST', f"/session/{sid}/element/{element['element-6066-11e4-a52e-4f735466cecf']}/value", {'text': value})


def pending(native, switch, wallet, kind):
    time.sleep(1)
    switch(wallet)
    for _ in range(30):
        result = native('wallet_pending')
        if result:
            assert result['payload']['terms']['kind'] == kind, result
            return result
        time.sleep(.1)
    raise AssertionError('shared form did not reach native confirmation')


def transfer_form(execute, rpc, sid, native, switch, main, wallet, recipient, fixture, screenshot):
    wait(execute, "return !!document.querySelector('[data-assets-list-ready]')")
    before = len([r for r in fixture.requests if r[0]=='POST' and r[1]=='/v1/nni/assets/transfer'])
    click(execute, '[data-asset-transfer=AIC]')
    wait(execute, "return !!document.querySelector('[role=dialog]')")
    assert execute("return !document.querySelector('[role=dialog] input[type=password]')")
    keys(rpc, sid, '[role=dialog] input[placeholder="0.00000000"]', '1')
    keys(rpc, sid, '[role=dialog] input[aria-label="收款账户公钥"]', recipient['public_key'])
    text_button(execute, '查看并确认')
    wait(execute, "return !!document.querySelector('[data-asset-transfer-review]')")
    assert execute("return document.querySelector('[data-asset-transfer-review]').textContent.includes('桌面密钥库签名')")
    screenshot('shared-local-transfer-review')
    text_button(execute, '确认转账')
    request = pending(native, switch, wallet, 'transfer')
    assert request['payload']['terms']['recipient'] == recipient['public_key']
    assert request['payload']['terms']['amount_units'] == '100000000'
    time.sleep(2.1)
    assert native('wallet_confirm', {'operationId': request['payload']['operation_id'], 'password': 'fixture-vault-password'})['status']=='succeeded'
    switch(main)
    wait(execute, "return !document.querySelector('[role=dialog]')")
    assert execute("return !document.querySelector('[data-asset-transfer-completed]')"), 'preparation must not display completed transfer'
    assert len([r for r in fixture.requests if r[0]=='POST' and r[1]=='/v1/nni/assets/transfer']) == before


def bancor_form(execute, rpc, sid, native, switch, main, wallet, fixture, screenshot):
    wait(execute, "return !!document.querySelector('button[aria-label=\"最大化 K 线与交易区域\"]')")
    wait(execute, "return !document.querySelector('#bancor-trade-panel button[title=\"签名刷新余额\"]').disabled")
    text_button(execute, 'SWAP')
    wait(execute, "return !!document.querySelector('[data-bancor-trade-layout=swap]')")
    screenshot('shared-local-bancor-swap')
    text_button(execute, '标准')
    click(execute, 'button[aria-label="最大化 K 线与交易区域"]')
    wait(execute, "return document.querySelector('#bancor-market-workspace').dataset.chartMaximized==='true'")
    assert execute("const b=document.querySelector('[data-bancor-account-selector] button[aria-label]');const r=b.getBoundingClientRect();return b.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))"), 'full-screen copy control must be reachable'
    screenshot('shared-local-bancor-maximized')
    click(execute, 'button[aria-label="恢复市场布局"]')
    before = len([r for r in fixture.requests if r[0]=='POST' and r[1] in ('/v1/nni/bancor/quote','/v1/nni/bancor/trade')])
    click(execute, '[data-bancor-trade-side=buy]')
    keys(rpc, sid, '#bancor-standard-input-amount', '1')
    wait(execute, "return !document.querySelector('[data-bancor-trade-submit=buy]').disabled")
    click(execute, '[data-bancor-trade-submit=buy]')
    request = pending(native, switch, wallet, 'bancor_trade')
    assert request['payload']['terms']['side']=='buy' and request['payload']['terms']['input_units']=='100000000'
    time.sleep(2.1)
    assert native('wallet_confirm', {'operationId': request['payload']['operation_id'], 'password': 'fixture-vault-password'})['status']=='succeeded'
    switch(main)
    assert len([r for r in fixture.requests if r[0]=='POST' and r[1] in ('/v1/nni/bancor/quote','/v1/nni/bancor/trade')]) == before
