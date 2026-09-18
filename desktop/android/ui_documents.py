"""Operate only Android's system document picker on a disposable emulator."""
import re
import subprocess
import threading
import time
import xml.etree.ElementTree as ET


def prepare(driver, mode, fixture):
    if mode == 'open':
        driver.device('push', str(fixture), '/sdcard/Download/desktop-fixture.backup.json')
        driver.device('shell','touch','/sdcard/Download/desktop-fixture.backup.json')
    assert mode in {'open', 'save'}
    def choose():
        try:
            end = time.monotonic() + 40
            selected = False
            while time.monotonic() < end:
                path = '/sdcard/mobile-acceptance-picker.xml'
                result=driver.device('shell', 'uiautomator', 'dump', path)
                if 'UI hierchary dumped' not in result:
                    time.sleep(.2)
                    continue
                xml = driver.device('shell', 'cat', path)
                nodes = list(ET.fromstring(xml).iter('node'))
                wanted = ({'SAVE', 'Save', '保存', 'REPLACE', 'Replace', '替换'} if mode == 'save'
                          else {'desktop-fixture.backup.json'})
                target = next((n for n in nodes if n.get('text') in wanted
                               and 'documentsui' in n.get('package', '')), None)
                if target is not None:
                    x1,y1,x2,y2 = map(int,re.findall(r'\d+',target.get('bounds')))
                    driver.device('shell','input','tap',str((x1+x2)//2),str((y1+y2)//2))
                    if mode == 'open' or target.get('text') in {'REPLACE','Replace','替换'}:
                        return
                    selected = True
                elif selected:
                    return
                elif mode == 'open':
                    scroll = next((n for n in nodes if n.get('scrollable')=='true'
                                   and 'documentsui' in n.get('package','')), None)
                    if scroll is not None:
                        x1,y1,x2,y2=map(int,re.findall(r'\d+',scroll.get('bounds')))
                        driver.device('shell','input','swipe',str((x1+x2)//2),str(y2-40),
                                      str((x1+x2)//2),str(y1+40),'250')
                time.sleep(.3)
            raise AssertionError('system_document_picker_timeout:' + mode)
        except Exception as error:
            driver.document_errors.append(str(error))
    thread = threading.Thread(target=choose, daemon=True)
    driver.document_threads.append(thread)
    thread.start()


def saved(driver):
    try:
        return subprocess.check_output([str(driver.adb),'-s',driver.serial,'shell','cat','/sdcard/Download/'+driver.export_name],
                                       text=True,stderr=subprocess.DEVNULL)
    except subprocess.CalledProcessError:
        return ''
