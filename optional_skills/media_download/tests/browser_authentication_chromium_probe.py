"""Opt-in Chromium probe using synthetic cookies; no real account is accessed."""
from __future__ import annotations

import argparse
import contextlib
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
from unittest import mock

sys.path.insert(0, str(Path(__file__).parents[1] / "src" / "tool"))
from browser_authentication import AuthenticationStore
import browser_authentication
import wechat_channels_access as access


@contextlib.contextmanager
def browser(chrome: str, profile: Path):
    port, arguments = access._devtools_endpoint()
    process = subprocess.Popen([
        chrome, "--headless=new", "--no-sandbox", "--disable-dev-shm-usage",
        "--no-first-run", "--no-default-browser-check", "--disable-background-networking",
        "--remote-allow-origins=*", *arguments, f"--user-data-dir={profile}", "about:blank",
    ], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    client = None
    try:
        client = access.DevToolsConnection(access._wait_for_page(port, process, 20, None))
        def call(method, params):
            result = access._wait_result(client, client.send(method, params), 10, None)
            if "error" in result:
                raise AssertionError("CDP probe command failed: " + method)
            return result.get("result", {})
        call("Network.enable", {})
        yield call
    finally:
        if client is not None:
            try:
                client.send("Browser.close")
                process.wait(timeout=5)
            except (OSError, RuntimeError, subprocess.TimeoutExpired):
                pass
            client.close()
        access._stop_browser(process)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--chrome", required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="browser-authentication-probe-") as temporary:
        root = Path(temporary)
        vault = root / "authentication"; vault.mkdir(mode=0o700)
        store = AuthenticationStore(vault, "a" * 64, "https://yuanbao.tencent.com", "wechat_channels")
        expiry = int(time.time()) + 3600
        cookie = {"name": "synthetic_probe", "value": "synthetic_probe_value", "domain": "yuanbao.tencent.com",
                  "path": "/", "expires": expiry, "secure": True, "httpOnly": True, "sameSite": "Lax"}
        with store.lease():
            for round_number in range(3):
                profile = root / f"task-{round_number}"
                with browser(args.chrome, profile) as call:
                    assert call("Network.getCookies", {"urls": [store.origin + "/"]})["cookies"] == []
                    if round_number == 0:
                        call("Network.setCookies", {"cookies": [cookie]})
                    else:
                        store.restore(call)
                    cookies = call("Network.getCookies", {"urls": [store.origin + "/"]})["cookies"]
                    assert len(cookies) == 1 and cookies[0]["value"] == cookie["value"]
                    assert cookies[0]["expires"] == expiry
                    store.capture(call)
                    assert store.metadata()["earliest_cookie_expiry"] == expiry
                shutil.rmtree(profile)
            with browser(args.chrome, root / "expired-task") as call, mock.patch.object(browser_authentication.time, "time", return_value=expiry + 1):
                assert store.metadata()["status"] == "expired"
                store.restore(call)
                assert call("Network.getCookies", {"urls": [store.origin + "/"]})["cookies"] == []
        assert {p.name for p in vault.iterdir()} == {"wechat_channels.json", "wechat_channels.json.lock"}
        print(json.dumps({"ok": True, "browser_rounds": 3, "task_profiles_deleted": True,
                          "expiration_excluded": True, "real_account_tested": False}))


if __name__ == "__main__":
    main()
