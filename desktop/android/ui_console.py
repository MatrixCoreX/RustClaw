"""Verify the shared device console inside the release Android WebView."""
import time


def check(driver):
    driver.tap('.device-row .desktop-actions button:last-child')
    driver.wait('return !!document.querySelector(".desktop-login-form input[autocomplete=username]") && !document.querySelector(".desktop-login-form button.primary").disabled')
    for selector, value in [('input[autocomplete=username]', 'tester'), ('input[type=password]', 'fixture-password')]:
        element=driver.rpc('POST','/session/mobile/element',{'value':'.desktop-login-form '+selector})
        driver.rpc('POST','/session/mobile/element/'+element['element-6066-11e4-a52e-4f735466cecf']+'/value',{'text':value})
    driver.tap('.desktop-login-form button.primary')
    driver.wait('return !!document.querySelector(".desktop-console .theme-header")')
    assert driver.native('current_session')['identity']['role']=='admin'
    for width,height in [(320,640),(393,852),(800,1280),(915,412)]:
        driver.device('shell','wm','size',f'{width}x{height}')
        time.sleep(.8)
        driver.execute('window.scrollTo(0,0)')
        assert driver.execute('return document.documentElement.scrollWidth<=innerWidth+1'), 'shared console overflow'
        assert driver.execute('return document.querySelector(".theme-header").getBoundingClientRect().top>=document.querySelector(".desktop-device-bar").getBoundingClientRect().bottom-1'), 'device bar covers navigation'
        driver.screenshot(f'device-console-{width}-{height}')
    driver.tap('.theme-topbar-nav-btn')
    driver.wait('return document.querySelector(".theme-topbar-nav-btn").getAttribute("aria-expanded")==="true"')
    driver.screenshot('device-navigation')
    driver.tap('.theme-topbar-nav-btn')
    driver.tap('.desktop-device-bar button')
    driver.wait('return !!document.querySelector(".desktop-home")')
    driver.device('shell','wm','size','393x852')

