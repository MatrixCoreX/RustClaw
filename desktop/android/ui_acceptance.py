"""Release Android WebView acceptance against loopback fixtures, never real funds."""
import argparse
import base64
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import time
import uuid

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / 'tests'))
from ui_fixture import AndroidFixture as Fixture
from standalone_fixture import StandaloneApi
from wallet_fixture import OwnerApi
from wallet_shared_ui_e2e import transfer_form, bancor_form
import ui_documents
import ui_console
import ui_keyboard


class Driver:
    def __init__(self, serial, output):
        assert serial.startswith('emulator-'), 'requires_disposable_emulator'
        self.serial, self.output = serial, output
        self.adb = Path(os.environ['ANDROID_HOME']) / 'platform-tools/adb'
        output.mkdir(parents=True, exist_ok=True)
        self.observations = []
        self.document_errors, self.document_threads = [], []
        self.export_name = 'fixture-' + uuid.uuid4().hex + '.txt'

    def device(self, *args):
        return subprocess.check_output([str(self.adb), '-s', self.serial, *args], text=True, timeout=45)

    def call(self, command, **params):
        if command == 'prepareDocument':
            ui_documents.prepare(self, params['mode'], ROOT/'tests/fixtures/wallet-linux-v2.json')
            return True
        if command == 'savedDocument':
            return ui_documents.saved(self)
        with socket.create_connection(('127.0.0.1', 8766), timeout=25) as connection:
            connection.sendall((json.dumps(dict(command=command, **params)) + '\n').encode())
            result = json.loads(connection.makefile('rb').readline())
        assert result['ok'], result
        return result.get('value')

    def execute(self, script, args=None):
        code = "(()=>{try{return JSON.stringify({ok:true,value:(function(){" + script + "}).apply(null," + json.dumps(args or []) + ")})}catch(e){return JSON.stringify({ok:false,error:String(e)})}})()"
        result = json.loads(json.loads(self.call('eval', script=code)))
        assert result['ok'], result
        return result.get('value')

    def wait(self, script, timeout=50):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if self.execute(script):
                return
            time.sleep(.2)
        raise AssertionError(script + ': ' + str(self.execute('return document.body.innerText'))[:2000])

    def native(self, command, args=None, error=None):
        self.execute("window.__fixtureResult=null;window.__TAURI_INTERNALS__.invoke(arguments[0],arguments[1]).then(value=>window.__fixtureResult={ok:true,value},e=>window.__fixtureResult={ok:false,error:String(e)});", [command, args or {}])
        self.wait('return window.__fixtureResult!==null', 150)
        result = self.execute('return window.__fixtureResult')
        if error:
            assert not result['ok'] and error in result['error'], result
            return result['error']
        assert result['ok'], result
        return result.get('value')

    def switch(self, window):
        self.call('switch', window=window)
        time.sleep(.4)

    def screenshot(self, name):
        (self.output / (name + '.png')).write_bytes(base64.b64decode(self.call('screenshot')))

    def tap(self, selector):
        self.execute('document.querySelector(arguments[0]).scrollIntoView({block:"center"})', [selector])
        time.sleep(.2)
        point = self.execute('const e=document.querySelector(arguments[0]),r=e.getBoundingClientRect();return {x:(r.x+r.width/2)*devicePixelRatio,y:(r.y+r.height/2)*devicePixelRatio}', [selector])
        self.call('tap', **point)

    def rpc(self, method, path, data=None):
        # Existing form tests use only element lookup and input entry here.
        if path.endswith('/element'):
            return {'element-6066-11e4-a52e-4f735466cecf': base64.urlsafe_b64encode(data['value'].encode()).decode()}
        selector = base64.urlsafe_b64decode(path.split('/element/')[1].split('/')[0]).decode()
        if path.endswith('/value'):
            self.execute('const e=document.querySelector(arguments[0]);Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,"value").set.call(e,arguments[1]);e.dispatchEvent(new Event("input",{bubbles:true}));e.dispatchEvent(new Event("change",{bubbles:true}));', [selector, data['text']])
        else:
            self.tap(selector)

    def layout(self, page):
        for width, height, theme, language in [(320,640,'dark','zh'), (393,852,'light','en'),
                (720,960,'dark','zh'), (800,1280,'light','en'), (915,412,'dark','en'), (1280,800,'light','zh')]:
            self.device('shell','wm','size', f'{width}x{height}')
            self.device('shell','wm','density','160')
            time.sleep(.8)
            self.execute('document.documentElement.dataset.theme=arguments[0];if((document.documentElement.lang.startsWith("zh")?"zh":"en")!==arguments[1])document.querySelector("[data-desktop-language-toggle]").click();window.scrollTo(0,0)', [theme,language])
            time.sleep(.3)
            result = self.execute('return {width:innerWidth,height:innerHeight,scroll:document.documentElement.scrollWidth,dpr:devicePixelRatio,lang:document.documentElement.lang,theme:document.documentElement.dataset.theme,overflow:[...document.querySelectorAll("body *")].filter(e=>e.getBoundingClientRect().right>innerWidth+1).slice(0,12).map(e=>({tag:e.tagName,cl:e.className,width:e.getBoundingClientRect().width}))}')
            self.observations.append(dict(page=page, physical=[width,height], **result))
            self.screenshot(f'{page}-{width}-{height}-{theme}')
            assert result['scroll'] <= result['width'] + 1, result
            if page in {'assets', 'bancor'}:
                assert self.execute('return [...document.querySelectorAll(".desktop-asset-header nav button")].every(e=>e.getBoundingClientRect().height<=64)'), 'asset navigation labels must stay readable on small screens'
        self.device('shell','wm','size','393x852')
        self.execute('document.documentElement.dataset.theme="dark";if(document.documentElement.lang==="en")document.querySelector("[data-desktop-language-toggle]").click();window.scrollTo(0,0)')
        time.sleep(.4)


def run(driver):
    fixture = Fixture(driver.output / 'tls')
    # Freshly generated certificates can be one second ahead of the emulator
    # clock. Wait for validity; never weaken the production TLS checks.
    from cryptography import x509
    certificate=x509.load_pem_x509_certificate((driver.output/'tls/leaf.pem').read_bytes())
    valid_after=int(certificate.not_valid_before_utc.timestamp())+1
    deadline=time.monotonic()+10
    while int(driver.device('shell','date','+%s').strip())<valid_after and time.monotonic()<deadline:
        time.sleep(.25)
    fixture.owner_api = OwnerApi()
    # Standalone nodes use public HTTPS roots or explicit loopback HTTP. Keep
    # this private synthetic ledger on loopback, without changing its TLS policy.
    driver.device('reverse', f'tcp:{fixture.local_server.server_port}', f'tcp:{fixture.local_server.server_port}')
    checks=[]
    try:
        driver.wait('return !!document.querySelector(".desktop-home")')
        driver.native('wallet_initialize', {'password':'fixture-vault-password'}, error='not allowed')
        driver.native('add_profile', {'alias':'Invalid LAN HTTP','connection':{'kind':'https','origin':'http://192.168.1.2','ca_pem':None,'ca_sha256':None}}, error='https')
        unpaired=driver.native('add_profile', {'alias':'Untrusted certificate','connection':{'kind':'https','origin':fixture.origin,'ca_pem':None,'ca_sha256':None}})
        driver.native('connect_device', {'profileId':unpaired['id'],'sshSecret':''}, error='tls_or_connection_failed')
        driver.native('forget_profile', {'profileId':unpaired['id']})
        profile=driver.native('add_profile', {'alias':'Synthetic HTTPS device','connection':{'kind':'https','origin':fixture.origin,'ca_pem':fixture.pem,'ca_sha256':fixture.fingerprint}})
        session=driver.native('connect_device', {'profileId':profile['id'],'sshSecret':''})
        login=driver.native('login', {'sessionId':session['id'],'input':{'mode':'password','username':'tester','secret':'fixture-password'},'remember':True})
        assert login['session']['identity']['role']=='admin'
        driver.native('login_prefill', {'sessionId':session['id']}, error='login_prefill_unavailable')
        driver.native('disconnect_device')
        session=driver.native('connect_device', {'profileId':profile['id'],'sshSecret':''})
        prefill=driver.native('login_prefill', {'sessionId':session['id']})
        assert prefill=={'mode':'password','username':'tester'}
        saved=driver.native('login', {'sessionId':session['id'],'input':None,'remember':False})
        assert saved['session']['identity']['role']=='admin'
        driver.native('disconnect_device')
        checks.append('Native TLS pairing and encrypted saved-login reuse with metadata-only prefill; main-page wallet command denied; LAN HTTP and untrusted certificate rejected')
        driver.native('wallet_open')
        end=time.monotonic()+30
        while 'wallet' not in driver.call('windows') and time.monotonic()<end: time.sleep(.2)
        driver.switch('wallet');driver.wait('return !!document.querySelector(".wallet-manager")')
        status=driver.native('wallet_status')
        driver.native('wallet_unlock' if status['initialized'] else 'wallet_initialize', {'password':'fixture-vault-password'})
        status=driver.native('wallet_status')
        account=next((a for a in status['accounts'] if a['name']=='Phone 一'),None) or driver.native('wallet_create',{'name':'Phone 一'})
        second=next((a for a in status['accounts'] if a['name']=='Phone 二'),None) or driver.native('wallet_create',{'name':'Phone 二'})
        driver.call('prepareDocument',mode='save')
        assert driver.native('wallet_backup',{'accountId':account['id'],'vaultPassword':'fixture-vault-password','password':'Jasper!flume7-Pebble4-Orbit9-velvet'})
        driver.native('wallet_unlock', {'password':'fixture-vault-password'})
        driver.call('prepareDocument',mode='open')
        restored=driver.native('wallet_restore',{'password':'Jasper!flume7-Pebble4-Orbit9-velvet','name':'Desktop backup'})
        assert restored['public_key']==json.loads((ROOT/'tests/fixtures/wallet-linux-v2.json').read_text())['public_key']
        driver.layout('wallet')
        ui_keyboard.check(driver)
        checks.append('Native account generation, SAF activity result and content-provider export with readback, desktop encrypted backup recovery')
        checks.append('Real system keyboard resizes the viewport and leaves the focused account field visible')
        for event in ['3','223']:
            driver.native('wallet_unlock', {'password':'fixture-vault-password'})
            driver.device('shell','input','keyevent',event)
            time.sleep(2.5)
            driver.device('shell','input','keyevent','224')
            driver.device('shell','input','keyevent','82')
            driver.device('shell','am','start','-n','org.agent_runtime.mobile/.MainActivity')
            driver.switch('main')
            driver.wait('return !!window.__TAURI_INTERNALS__ && !!document.querySelector(".desktop-home")')
            driver.native('wallet_open')
            driver.switch('wallet')
            assert not driver.native('wallet_status')['unlocked']
        checks.append('Background and screen-off revoke the unlocked account session')
        driver.switch('main');driver.execute('location.reload()');time.sleep(1)
        driver.wait('return !!window.__TAURI_INTERNALS__ && !!document.querySelector("#home-asset-account")')
        driver.native('wallet_select',{'accountId':account['id']})
        driver.wait('return document.querySelector("#home-asset-account").options.length>=3')
        driver.layout('home')
        driver.device('shell','settings','put','system','font_scale','1.5')
        time.sleep(1)
        assert driver.execute('return document.documentElement.scrollWidth <= innerWidth + 1')
        driver.screenshot('home-large-font')
        driver.device('shell','settings','put','system','font_scale','1.0')
        driver.call('prepareDocument',mode='save')
        driver.execute('const b=new Blob(["Synthetic mobile export"],{type:"text/plain"}),u=URL.createObjectURL(b),a=document.createElement("a");a.href=u;a.download=arguments[0];a.click();URL.revokeObjectURL(u);', [driver.export_name])
        end=time.monotonic()+30
        while driver.call('savedDocument')!='Synthetic mobile export' and time.monotonic()<end: time.sleep(.2)
        assert driver.call('savedDocument')=='Synthetic mobile export'
        checks.append('150% system font without horizontal overflow and real Blob-to-SAF export')
        driver.tap('[data-home-account] button[aria-label="复制完整公钥"]')
        end=time.monotonic()+3
        while driver.call('clipboard')!=account['public_key'] and time.monotonic()<end: time.sleep(.1)
        assert driver.call('clipboard')==account['public_key']
        fixture.standalone_api=StandaloneApi(fixture.local_origin,second['public_key'])
        node=driver.native('wallet_add_node',{'origin':fixture.local_origin})
        driver.native('wallet_connect_node',{'nodeId':node['id']})
        driver.execute('document.querySelector("[data-home-open=assets]").click()')
        driver.wait('return !!document.querySelector("[data-assets-list-ready]")')
        driver.layout('assets')
        assert not driver.execute('return [...document.querySelectorAll("button")].some(b=>/获取奖励|获得奖励/.test(b.textContent))')
        transfer_form(driver.execute,driver.rpc,'android',driver.native,driver.switch,'main','wallet',second,fixture,driver.screenshot)
        driver.execute('[...document.querySelectorAll(".desktop-asset-header nav button")].find(b=>b.textContent.trim()==="Bancor").click()')
        driver.wait('return !!document.querySelector("#bancor-trade-panel")')
        driver.layout('bancor')
        assert not driver.execute('return !!document.querySelector("[data-bancor-open-apr]")')
        bancor_form(driver.execute,driver.rpc,'android',driver.native,driver.switch,'main','wallet',fixture,driver.screenshot)
        assert len(fixture.standalone_api.verified)==2
        checks.append('Full Assets and Bancor pages on six viewport shapes in two languages/themes, public-key clipboard, verified synthetic transfer and trade signatures')
        driver.device('shell','input','keyevent','4')
        driver.wait('return !!document.querySelector(".desktop-home")')
        checks.append('System back returns from the asset workspace to device home')
        ui_console.check(driver)
        checks.append('Visible device login opens the shared console; phone/tablet/landscape layouts and navigation remain accessible; disconnect returns home')
        for thread in driver.document_threads: thread.join(timeout=5)
        assert not driver.document_errors, driver.document_errors
        (driver.output/'acceptance.json').write_text(json.dumps(dict(ok=True,checks=checks,screens=driver.observations,verified=fixture.standalone_api.verified),ensure_ascii=False,indent=2))
        print(json.dumps(checks,ensure_ascii=False),flush=True)
    finally:
        (driver.output/'screen-observations.json').write_text(json.dumps(driver.observations,ensure_ascii=False,indent=2))
        fixture.close()


if __name__=='__main__':
    parser=argparse.ArgumentParser();parser.add_argument('--serial',default='emulator-5556');parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args();driver=Driver(args.serial,args.output)
    try:
        run(driver)
    finally:
        try:
            driver.call('quit')
        except (OSError, ValueError, AssertionError):
            pass  # Preserve the original failure when the emulator disconnects.
