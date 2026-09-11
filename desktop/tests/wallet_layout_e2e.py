"""Account selector placement and real X11 clipboard checks in the native WebView."""
from http.client import RemoteDisconnected
import subprocess
import time


def run(execute, rpc, sid, account, second, screenshot, page):
    selector = f'[data-{page}-account-selector]'

    def choose(account_id):
        execute("const s=document.querySelector(arguments[0]+' select');s.value=arguments[1];s.dispatchEvent(new Event('change',{bubbles:true}));return true;", [selector, account_id])
        for _ in range(80):
            if execute("return document.querySelector(arguments[0]+' select')?.value===arguments[1]", [selector, account_id]):
                time.sleep(.15)
                return
            time.sleep(.1)
        raise AssertionError('account switch did not finish')

    structures = {}

    def position_and_copy(account_id):
        choose(account_id)
        observation = execute("""const s=document.querySelector(arguments[0]);
          const select=s.querySelector('select');
          const container=arguments[1]==='assets' ? s.closest('section[aria-label="资产总览"],section[aria-labelledby="asset-overview-title"]') : s.closest('#bancor-trade-panel .bancor-trade-account');
          const balance=container?.querySelector('.bancor-trade-balance-header');
          const r=s.getBoundingClientRect();
          return {count:document.querySelectorAll('select[aria-label="桌面资产账户"]').length,
            inside:!!container,aboveBalance:!balance||r.bottom<=balance.getBoundingClientRect().top+1,
            overflow:document.documentElement.scrollWidth>innerWidth+1,
            key:s.querySelector('code')?.textContent,selected:select.value,option:select.selectedOptions[0].textContent,
            structure:arguments[1]==='assets' ? [...document.querySelectorAll('[data-assets-page] > section[aria-labelledby]')].map(e=>[e.getAttribute('aria-labelledby'),e.className]) : [...document.querySelectorAll('#bancor-market-workspace,.bancor-market-chart-panel,#bancor-trade-panel,[role=group][aria-label=交易模式]')].map(e=>[e.id,e.className]),
            copy:!!s.querySelector('button[aria-label="复制完整公钥"]')};""", [selector, page])
        assert observation['count'] == 1 and observation['inside'] and observation['copy'], observation
        if account_id:
            expected = account if account_id == account['id'] else second
            assert observation['key'] == expected['public_key'], 'displayed key must follow the selected local account'
            assert observation['option'].startswith('桌面本地账号 · ' + expected['name']), observation
        assert not observation['overflow'], observation
        assert len(observation['structure']) >= 3, observation
        if 'shared' in structures:
            assert observation['structure'] == structures['shared'], 'local and hardware account page layout must be identical'
        else:
            structures['shared'] = observation['structure']
        if page == 'bancor':
            assert observation['aboveBalance'], observation
        execute("document.querySelector(arguments[0]).scrollIntoView({block:'center'});return true;", [selector])
        element = rpc('POST', f'/session/{sid}/element', {'using': 'css selector', 'value': selector + ' button[aria-label="复制完整公钥"]'})
        try:
            rpc('POST', f"/session/{sid}/element/{element['element-6066-11e4-a52e-4f735466cecf']}/click", {})
        except RemoteDisconnected:
            # WebKit may drop the automation response during clipboard focus changes.
            # Never repeat the click: verify completion and the actual clipboard below.
            pass
        for _ in range(30):
            if execute("return !!document.querySelector(arguments[0]+' button[aria-label=\"已复制完整公钥\"]')", [selector]):
                break
            time.sleep(.1)
        else:
            raise AssertionError('copy did not report success')
        copied = subprocess.check_output(['xsel', '--clipboard', '--output'], text=True, timeout=5)
        assert copied == observation['key'] and len(copied) >= 40, 'clipboard must contain full selected public key'

    for width, mode in [(1280, 'light'), (900, 'dark')]:
        rpc('POST', f'/session/{sid}/window/rect', {'width': width, 'height': 900})
        execute("document.documentElement.dataset.theme=arguments[0];return true;", [mode])
        time.sleep(.4)
        position_and_copy(account['id'])
        screenshot(f'account-position-{page}-local-{mode}')
        # Switching to another local key must also change the copied public key.
        position_and_copy(second['id'])
        position_and_copy('')
        screenshot(f'account-position-{page}-hardware-{mode}')
    choose(account['id'])
    rpc('POST', f'/session/{sid}/window/rect', {'width': 1280, 'height': 900})
    execute("window.scrollTo(0,0);return true;")
