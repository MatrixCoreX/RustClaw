"""Visual and hit-testing checks in the packaged Ubuntu WebKitGTK console."""
import json
import time


def run(execute, resize, click, keys, screenshot, out, checks):
    def page(name):
        execute("""const button=document.querySelector('[data-desktop-page="'+arguments[0]+'"]');
          if(button) button.click(); else {
            const key=Object.keys(localStorage).find(k=>k.endsWith('agent-runtime.monitor.currentPage'));
            if(!key)throw Error('route key missing');
            localStorage.setItem(key,arguments[0]);location.reload();
          } return true;""", [name])
        time.sleep(1.5)

    def geometry():
        return execute("""const bar=document.querySelector('.desktop-device-bar');
          const header=document.querySelector('.desktop-console .theme-header');
          const sidebar=document.querySelector('.desktop-console aside');
          const toggle=sidebar.querySelector('button');
          const r=toggle.getBoundingClientRect();
          return {bar:bar.getBoundingClientRect().toJSON(),header:header.getBoundingClientRect().toJSON(),
            sidebar:sidebar.getBoundingClientRect().toJSON(),viewport:innerHeight,
            toggleClickable:toggle.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2))};""")

    def theme(mode):
        execute("window.scrollTo(0,0);return true;")
        time.sleep(.1)
        if execute("return document.documentElement.dataset.theme") != mode:
            click('.theme-header button[aria-label]')
        time.sleep(.2)
        assert execute("return document.documentElement.dataset.theme") == mode

    resize(1280, 860)
    page('dashboard')
    baseline = geometry()
    screenshot('07-navigation-top')
    page('bancor')
    for _ in range(50):
        if execute("return !!document.querySelector('select[aria-label=\"桌面资产账户\"]')"):
            break
        time.sleep(.1)
    else:
        raise AssertionError('Fixture trading account selector did not mount')
    assert execute("return !!document.querySelector('#bancor-trade-panel .bancor-trade-account [data-bancor-account-selector] select')")
    assert execute("return !!document.querySelector('[data-bancor-account-selector] button[aria-label=\"复制完整公钥\"]')")
    styles = {}
    for mode in ['dark', 'light']:
        theme(mode)
        execute("document.querySelector('[data-bancor-account-selector]').scrollIntoView({block:'center'});document.activeElement.blur();return true;")
        time.sleep(.2)
        styles[mode] = execute("""return [...document.querySelectorAll('select')].map(s=>{
          const c=getComputedStyle(s);return {label:s.getAttribute('aria-label'),
            selected:s.selectedOptions[0]?.textContent,color:c.color,fill:c.webkitTextFillColor,
            background:c.backgroundColor,appearance:c.appearance,scheme:c.colorScheme};});""")
        screenshot('08-trading-select-' + mode)
    (out / 'layout-observations.json').write_text(json.dumps({'initial':baseline,'selects':styles}, ensure_ascii=False, indent=2))
    assert baseline['toggleClickable'], baseline
    assert baseline['sidebar']['top'] >= baseline['header']['bottom'] - 1, baseline
    for mode in styles:
        assert all(s['color'] != s['background'] and s['selected'] for s in styles[mode]), styles
    checks.append('actual trading account and other selects remain readable before focus in both themes')

    page('assets')
    assert execute("return !!document.querySelector('section[aria-labelledby=asset-overview-title] [data-assets-account-selector] select')")
    assert execute("return !!document.querySelector('[data-assets-account-selector] button[aria-label=\"复制完整公钥\"]')")
    checks.append('hardware account selectors and copy controls remain inside original web overview/trading slots')

    page('logs')
    for mode in ['dark', 'light']:
        theme(mode)
        execute("document.activeElement.blur();return true;")
        screenshot('12-log-selects-' + mode)
    keys('.desktop-console select:not([disabled])', '\ue011\ue007\ue004')
    assert execute("return document.querySelector('.desktop-console select:not([disabled])').value") == '100'
    keys('.desktop-console select:not([disabled])', '\ue010\ue007\ue004')
    assert execute("return document.querySelector('.desktop-console select:not([disabled])').value") == '1000'
    checks.append('log dropdowns retain native keyboard selection and render both enabled and disabled fields')

    page('dashboard')
    for width, height in [(1280, 860), (1050, 650)]:
        resize(width, height)
        for _ in range(2):
            current = geometry()
            assert current['toggleClickable'], current
            assert current['sidebar']['top'] >= current['header']['bottom'] - 1, current
            assert abs(current['sidebar']['bottom'] - current['viewport']) <= 1, current
            click('.desktop-console aside button')
            time.sleep(.3)
        # Force a long content page; exercise sticky positioning after real scrolling.
        execute("const space=document.createElement('div');space.id='layout-scroll-fixture';space.style.height='1800px';document.querySelector('.desktop-console main').append(space);window.scrollTo(0,500);return true;")
        time.sleep(.2)
        current = geometry()
        assert current['toggleClickable'], current
        assert abs(current['header']['top'] - current['bar']['bottom']) <= 1, current
        screenshot(f'09-navigation-scroll-{width}')
        execute("document.getElementById('layout-scroll-fixture').remove();window.scrollTo(0,0);return true;")
    checks.append('navigation toggle is unobstructed when expanded, collapsed, resized and scrolled')

    execute("document.documentElement.style.fontSize='20px';return true;")
    time.sleep(.3)
    enlarged = geometry()
    assert enlarged['toggleClickable'] and abs(enlarged['sidebar']['top'] - enlarged['header']['bottom']) <= 1, enlarged
    execute("document.documentElement.style.fontSize='';return true;")
    time.sleep(.3)

    resize(860, 600)
    click('.theme-topbar-nav-btn')
    time.sleep(.2)
    assert execute("""const button=[...document.querySelectorAll('.theme-header button')].find(b=>b.textContent.trim()==='首页');
      if(!button)return false;const r=button.getBoundingClientRect();
      return button.contains(document.elementFromPoint(r.x+r.width/2,r.y+r.height/2));""")
    screenshot('10-navigation-compact')
    click('.theme-topbar-nav-btn')
    page('chat')
    for width, height in [(860, 600), (1280, 860)]:
        resize(width, height)
        assert execute("return document.documentElement.scrollHeight <= innerHeight + 1"), geometry()
        current = geometry()
        assert abs(current['header']['top'] - current['bar']['bottom']) <= 1, current
        screenshot(f'11-chat-viewport-{width}')
    checks.append('compact navigation opens unobstructed and chat fits the available window height')
    page('dashboard')
