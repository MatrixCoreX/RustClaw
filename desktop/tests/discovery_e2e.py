"""Opt-in read-only LAN acceptance, using public CA obtained through verified SSH."""
import hashlib
import os
import ssl
import time
from pathlib import Path


def run(execute, native, screenshot, checks, wait_text):
    host = os.environ.get("DESKTOP_TEST_LAN_HOST")
    if not host:
        return
    pem = Path(os.environ["DESKTOP_TEST_LAN_CA"]).read_text()
    fingerprint = hashlib.sha256(ssl.PEM_cert_to_DER_cert(pem)).hexdigest()
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='添加设备').click();return true;")
    wait_text(host)
    screenshot("04-lan-discovery")
    execute("[...document.querySelectorAll('.desktop-discovered')].find(b=>b.textContent.includes(arguments[0])).click();return true;", [host])
    assert execute("return document.querySelector('input[placeholder*=\"https://\"]').value") == f"https://{host}:443"
    assert execute("return [...document.querySelectorAll('input[type=checkbox]')].some(i=>i.checked)")
    assert not execute("return [...document.querySelectorAll('input[type=checkbox]')].find(i=>i.closest('label').textContent.includes('核对')).checked")
    assert not native("profiles")
    checks.append("real Pi mDNS discovery fills address and CA hint without creating trust")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='扫描局域网').click();return true;")
    time.sleep(.15)
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='停止查找').click();return true;")
    wait_text("已停止查找")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='扫描局域网').click();return true;")
    for _ in range(250):
        if execute("return document.querySelector('.desktop-discovery').getAttribute('aria-busy')==='false'"):
            break
        time.sleep(.1)
    else:
        raise AssertionError("LAN scan did not finish within its bound")
    assert host in execute("return document.querySelector('.desktop-discovery').innerText")
    screenshot("05-lan-scan")
    checks.append("real LAN scan completes; cancellation returns promptly and rescan works")
    profile = native("add_profile", {"alias": "LAN HTTPS validation", "connection": {
        "kind": "https", "origin": f"https://{host}", "ca_pem": pem, "ca_sha256": fingerprint}})
    info = native("connect_device", {"profileId": profile["id"], "sshSecret": ""})
    # connect_device itself validates TLS and performs the native-only session GET.
    assert info["identity"] is None
    native("disconnect_device")
    native("forget_profile", {"profileId": profile["id"]})
    assert not native("profiles")
    checks.append("installed native client validates actual Pi HTTPS chain and hostname, anonymous session GET only")
    execute("[...document.querySelectorAll('button')].find(b=>b.textContent==='返回').click();return true;")
