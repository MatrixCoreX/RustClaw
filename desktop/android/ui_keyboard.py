"""Check that Android's real input method leaves an account field visible."""
import time


def check(driver):
    driver.device('shell','settings','put','secure','show_ime_with_hard_keyboard','1')
    initial=driver.execute('return innerHeight')
    driver.tap('input[aria-label="账户名称"]')
    deadline=time.monotonic()+15
    while not driver.call('imeVisible') and time.monotonic()<deadline: time.sleep(.1)
    assert driver.call('imeVisible'), 'system keyboard did not open'
    driver.wait('return innerHeight < '+str(initial), 15)
    driver.wait('const e=document.activeElement,r=e.getBoundingClientRect();return e.tagName==="INPUT" && r.top>=0 && r.bottom<=innerHeight', 10)
    driver.screenshot('wallet-system-keyboard')
    opened=driver.execute('return innerHeight')
    driver.device('shell','input','keyevent','4')
    deadline=time.monotonic()+10
    while driver.call('imeVisible') and time.monotonic()<deadline: time.sleep(.1)
    assert not driver.call('imeVisible'), 'system keyboard did not close'
    driver.wait('return innerHeight > '+str(opened+100), 10)
    # System navigation bars may change height after an orientation sequence;
    # verify actual IME dismissal instead of requiring identical bar pixels.
    print({'keyboard_initial':initial,'keyboard_open':opened,'keyboard_closed':driver.execute('return innerHeight')},flush=True)
