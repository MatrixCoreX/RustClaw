"""Home-to-assets acceptance with no device session or authentication cookies."""
import subprocess
import time
from http.client import RemoteDisconnected
from standalone_fixture import StandaloneApi
from wallet_shared_ui_e2e import transfer_form, bancor_form, keys, pending


def run(native, execute, rpc, sid, switch, main, wallet, account, second, fixture, screenshot, output, passed):
    def wait(script):
        for _ in range(150):
            try:
                if execute(script): return
            except RemoteDisconnected: pass
            time.sleep(.1)
        raise AssertionError(script + ': ' + execute('return document.body.innerText')[:1600])

    def click(selector):
        element = rpc('POST', f'/session/{sid}/element', {'using':'css selector','value':selector})
        try:
            rpc('POST',f"/session/{sid}/element/{element['element-6066-11e4-a52e-4f735466cecf']}/click",{})
        except RemoteDisconnected: pass

    switch(wallet)
    native('wallet_nodes', error='not allowed')
    native('wallet_prefer_node', error='not allowed')
    switch(main)
    native('disconnect_device')
    fixture.standalone_api = api = StandaloneApi(fixture.local_origin, second['public_key'])
    node = native('wallet_add_node', {'origin':fixture.local_origin})
    initial = native('wallet_connect_node', {'nodeId':node['id']})
    assert native('current_session') is None
    native('wallet_add_node', {'origin':'http://192.168.31.243'}, 'wallet_node_invalid')
    invalid_tls = native('wallet_add_node', {'origin':fixture.origin})
    native('wallet_connect_node', {'nodeId':invalid_tls['id']}, 'wallet_node_connection_failed')
    assert native('wallet_nodes')['selected'] == node['id']
    native('wallet_select', {'accountId':account['id']})
    assert native('wallet_read', {'sessionId':initial['id'],'accountId':account['id'],'service':'assets','page':None})['account'] == account['public_key']
    native('wallet_market_read', {'sessionId':initial['id'],'path':'/v1/auth/me'}, 'wallet_intent_invalid')
    try: execute('location.reload();return true;')
    except RemoteDisconnected: pass
    wait("return !!document.querySelector('[data-home-account]')")
    assert execute("return document.querySelectorAll('[data-home-account]').length") == 1
    assert execute("return document.querySelectorAll('[data-home-open]').length") == 1
    options = execute("return [...document.querySelector('#home-asset-account').options].map(o=>({id:o.value,label:o.textContent}))")
    assert len(options)==2 and all(any(o['id']==a['id'] and a['name'] in o['label'] for o in options) for a in [account,second])
    assert all('共 2 个' in option['label'] for option in options)
    theme_before = execute('return document.documentElement.dataset.theme')
    click('[data-desktop-language-toggle]')
    wait("return document.querySelector('#home-accounts-title').textContent.includes('Local asset accounts')")
    assert execute("return document.querySelector('#home-asset-account').selectedOptions[0].textContent.includes('2 total')")
    assert execute('return document.documentElement.dataset.theme') == theme_before
    assert execute("return document.querySelector('[data-home-account] code').textContent") == account['public_key']
    screenshot('home-english')
    switch(wallet)
    wait("return document.querySelector('.wallet-manager h1').textContent==='Local asset accounts'")
    assert execute("return document.querySelector('[data-desktop-language-toggle]').textContent") == 'English'
    screenshot('wallet-english')
    switch(main)
    try: execute('location.reload();return true;')
    except RemoteDisconnected: pass
    wait("return document.querySelector('#home-accounts-title')?.textContent.includes('Local asset accounts')")
    click('[data-desktop-language-toggle]')
    wait("return document.querySelector('#home-accounts-title').textContent.includes('本地资产账户')")
    selector = '[data-home-account]'
    for selected in [second,account]:
        execute('const s=document.querySelector("#home-asset-account");s.value=arguments[0];s.dispatchEvent(new Event("change",{bubbles:true}));return true;',[selected['id']])
        wait('return document.querySelector("[data-home-account]").dataset.homeAccount===' + repr(selected['id']))
        wait('return document.querySelector("[data-home-account] code")?.textContent===' + repr(selected['public_key']))
        assert execute('return document.querySelector("[data-home-account] .desktop-badge").textContent') == ('备份已验证' if selected['id']==account['id'] else '尚未备份')
        click(selector + ' button[aria-label="复制完整公钥"]')
        assert subprocess.check_output(['xsel','--clipboard','--output'],text=True,timeout=5) == selected['public_key']
    screenshot('standalone-home-' + execute('return document.documentElement.dataset.theme'))
    click('[data-desktop-theme-toggle]')
    screenshot('standalone-home-' + execute('return document.documentElement.dataset.theme'))
    click(selector + ' [data-home-open=assets]')
    wait("return !!document.querySelector('[data-assets-list-ready]')")
    assert execute("return document.querySelector('.desktop-asset-header > div').textContent.includes('已优选')")
    click('[data-desktop-language-toggle]')
    wait("return document.querySelector('[data-assets-page] header').textContent.includes('Asset overview')")
    assert execute("return !!document.querySelector('select[aria-label=\"Desktop asset account\"]')")
    screenshot('assets-english')
    click('[data-desktop-language-toggle]')
    wait("return !!document.querySelector('select[aria-label=\"桌面资产账户\"]')")
    click('[data-prefer-node]')
    wait("return document.querySelector('[data-prefer-node]').disabled===false")
    assert execute("return document.querySelector('.desktop-asset-header > div').textContent.includes('已优选')")
    assert not execute("return [...document.querySelectorAll('[data-assets-page] button')].some(b=>/获取奖励|获得奖励/.test(b.textContent))")
    assert native('current_session') is None
    # Preference reuses the current session when the same node still wins.
    assert native('wallet_read', {'sessionId':initial['id'],'accountId':account['id'],'service':'assets','page':None})['node_url'] == fixture.local_origin
    select = 'select[aria-label="桌面资产账户"]'
    assert execute('return document.querySelector(arguments[0]).value', [select]) == account['id']
    assert not execute('return [...document.querySelector(arguments[0]).options].some(o=>o.value==="")',[select])
    wait("return !!document.querySelector('[data-asset-transfer-history]')")
    for page in range(2, 12):
        wait("return !document.querySelector('[data-asset-history-pagination] button[aria-label=\"下一页\"]').disabled")
        click('[data-asset-history-pagination] button[aria-label="下一页"]')
        wait(f"return document.querySelector('[data-asset-history-pagination] span')?.textContent.trim()==='{page} / 11'")
    wait("return !!document.querySelector('[data-asset-transfer-history] [title=\"fixture-asset_transfer-100\"]')")
    assert any('/explorer/transactions?' in path and 'page=2' in path for _,path in api.requests)
    for attr,value in [('source','trade'),('direction','outgoing')]:
        field=f'[data-asset-history-{attr}-filter]'
        assert not execute('return document.querySelector(arguments[0]).disabled',[field])
        execute('const s=document.querySelector(arguments[0]);s.value=arguments[1];s.dispatchEvent(new Event("change",{bubbles:true}));return true;',[field,value])
    wait("return !!document.querySelector('[data-asset-transfer-history] [title=\"fixture-bancor_buy-0\"]')")
    assert any('transaction_class=market_trade' in path and 'direction=outgoing' in path for _,path in api.requests)
    screenshot('standalone-03-assets-filtered')
    execute('const s=document.querySelector(arguments[0]);s.value=arguments[1];s.dispatchEvent(new Event("change",{bubbles:true}));return true;',[select,second['id']])
    wait(f"return document.querySelector('{select}').value==={second['id']!r}")
    wait("return document.querySelector('[data-assets-account-selector]').textContent.includes('请在管理本地账号中完成加密备份')")
    execute('const s=document.querySelector(arguments[0]);s.value=arguments[1];s.dispatchEvent(new Event("change",{bubbles:true}));return true;',[select,account['id']])
    wait("return document.querySelector('[data-asset-transfer=AIC]')?.disabled===false")
    transfer_form(execute,rpc,sid,native,switch,main,wallet,second,fixture,screenshot)
    passed('one home account dropdown with total count and one asset entry; names, selected full public key, backup state and real clipboard follow account selection; two themes and no device login')
    passed('standalone Assets balances, local account switching, full source/direction filters, 101-record remote pagination and native transfer signature')
    switch(main)
    execute("[...document.querySelectorAll('[data-assets-page] header button')].find(b=>b.textContent.trim()==='Bancor').click();return true;")
    wait("return document.querySelector('[data-standalone-page]')?.dataset.standalonePage==='bancor'")
    wait("return !!document.querySelector('button[aria-label=\"最大化 K 线与交易区域\"]')")
    assert not execute("return [...document.querySelectorAll('button')].some(b=>/获取奖励|获得奖励/.test(b.textContent))")
    assert not execute("return !!document.querySelector('[data-bancor-open-apr]')")
    click('[data-desktop-language-toggle]')
    wait("return document.querySelector('#bancor-trade-panel').textContent.includes('Trading account')")
    assert execute("return document.body.textContent.includes('BANCOR reserve-curve market')")
    screenshot('bancor-english')
    click('[data-desktop-language-toggle]')
    wait("return document.querySelector('#bancor-trade-panel').textContent.includes('交易账户')")
    passed('shared Chinese/English language persists across reload and synchronizes home, Assets, Bancor and native wallet; account selection, public key and theme stay unchanged')
    contrast_checks = {}
    for theme in ['light','dark']:
        if execute('return document.documentElement.dataset.theme') != theme:
            click('.desktop-asset-header > button:last-child')
        contrast = execute('''
            const rgb=s=>(s.match(/[\\d.]+/g)||[]).map(Number);
            const background=e=>{if(!e)return [255,255,255];const c=rgb(getComputedStyle(e).backgroundColor),a=c[3]??1;if(a===1)return c.slice(0,3);const behind=background(e.parentElement);return c.slice(0,3).map((v,i)=>v*a+behind[i]*(1-a));};
            const luminance=c=>c.map(v=>{v/=255;return v<=.04045?v/12.92:((v+.055)/1.055)**2.4}).reduce((s,v,i)=>s+v*[.2126,.7152,.0722][i],0);
            const ratio=(fg,bg)=>{const a=luminance(rgb(fg).slice(0,3)),b=luminance(bg);return (Math.max(a,b)+.05)/(Math.min(a,b)+.05)};
            const panel=document.querySelector('#bancor-trade-panel');
            const input=panel.querySelector('select');
            return {heading:ratio(getComputedStyle(panel.querySelector('h2')).color,background(panel)),
                selector:ratio(getComputedStyle(input).color,background(input)),appearance:getComputedStyle(input).appearance};
        ''')
        assert contrast['heading'] >= 4.5 and contrast['selector'] >= 4.5 and contrast['appearance']=='none', contrast
        contrast_checks[theme] = contrast
        execute('window.scrollTo(0,0);return true;')
        screenshot('standalone-bancor-' + theme)
    bancor_form(execute,rpc,sid,native,switch,main,wallet,fixture,screenshot)
    click('[data-bancor-trade-side=sell]')
    amount = rpc('POST', f'/session/{sid}/element', {'using':'css selector','value':'#bancor-standard-input-amount'})
    rpc('POST',f"/session/{sid}/element/{amount['element-6066-11e4-a52e-4f735466cecf']}/clear",{})
    keys(rpc,sid,'#bancor-standard-input-amount','1')
    wait("return !document.querySelector('[data-bancor-trade-submit=sell]').disabled")
    click('[data-bancor-trade-submit=sell]')
    sell = pending(native,switch,wallet,'bancor_trade')
    assert sell['payload']['terms']['side']=='sell' and sell['payload']['terms']['input_units']=='100000000'
    time.sleep(2.1)
    assert native('wallet_confirm',{'operationId':sell['payload']['operation_id'],'password':'fixture-vault-password'})['status']=='succeeded'
    switch(main)
    screenshot('standalone-04-bancor')
    assert any('price_kind=pool_marginal_usd_per_aic' in path for _,path in api.requests)
    assert len(api.verified) == 3 and {p['terms'].get('side') for p in api.verified} == {None,'buy','sell'}
    assert native('current_session') is None
    click('.desktop-asset-header > button:first-child')
    wait("return !!document.querySelector('[data-home-account]')")
    theme = execute('return document.documentElement.dataset.theme')
    assert execute('return document.querySelector("[data-desktop-theme-toggle]").getAttribute("aria-label")') == ('切换到浅色外观' if theme=='dark' else '切换到深色外观')
    assert execute('return document.querySelector("[data-desktop-theme-toggle]").textContent') == ''
    click('[data-desktop-theme-toggle]')
    assert execute('return document.documentElement.dataset.theme') != theme
    assert native('current_session') is None and not native('wallet_status')['unlocked']
    assert execute("return document.querySelectorAll('[data-home-open]').length") == 1
    click(selector + ' [data-home-open=assets]')
    wait("return document.querySelector('[data-standalone-page]')?.dataset.standalonePage==='assets'")
    wait("return document.querySelector('.desktop-asset-header nav button:nth-child(2)')?.disabled===false")
    click('.desktop-asset-header nav button:nth-child(2)')
    wait("return document.querySelector('[data-standalone-page]')?.dataset.standalonePage==='bancor'")
    wait("return !!document.querySelector('button[aria-label=\"最大化 K 线与交易区域\"]')")
    click('.desktop-asset-header > button:first-child')
    wait("return !!document.querySelector('[data-home-account]')")
    passed('standalone Bancor live market, pool-price candles, SWAP and chart controls, native buy/sell signatures; renamed Bancor entry reaches trading page; rewards and APR buttons absent')
    from standalone_node_acceptance import run as node_acceptance
    node_acceptance(native,execute,switch,main,wallet,account,second,fixture,node,output,passed)
    import json
    (output/'standalone-acceptance.json').write_text(json.dumps({'signatures':len(api.verified),'public_reads':len(api.public_reads),'device_login_used':False,'contrast':contrast_checks,'requests':api.requests},ensure_ascii=False,indent=2))
