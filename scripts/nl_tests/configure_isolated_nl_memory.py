#!/usr/bin/env python3
"""Opt in an isolated NL principal to memory through the normal settings API."""
import argparse
import json
import sqlite3
import urllib.request
from pathlib import Path
from urllib.parse import urlparse


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--isolation-root", type=Path, required=True)
    parser.add_argument("--base-url", required=True)
    args = parser.parse_args()
    root = args.isolation_root.resolve(strict=True)
    if not root.name.startswith("agent-runtime-nl-isolated-") or root.parent.name != "tmp":
        parser.error("only an isolated NL workspace is supported")
    url = urlparse(args.base_url)
    if url.scheme != "http" or url.hostname != "127.0.0.1" or not url.port:
        parser.error("a loopback test URL is required")
    with sqlite3.connect(f"file:{root}/tasks.sqlite?mode=ro", uri=True) as db:
        key = db.execute("SELECT user_key FROM auth_keys WHERE role='admin' AND enabled=1 LIMIT 1").fetchone()[0]
    endpoint = args.base_url.rstrip("/") + "/v1/memory/settings"

    def request(body=None):
        req = urllib.request.Request(endpoint, data=None if body is None else json.dumps(body).encode(),
                                     headers={"X-Agent-Key": key, "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=30) as response:
            value = json.load(response)
        if not value.get("ok"):
            raise RuntimeError("isolated_memory_settings_failed")
        return value["data"]

    before = request()
    after = before
    if not before["generate_memory"] or not before["use_memory"]:
        after = request({"scope": "principal", "expected_revision": before["revision"],
                         "use_mode": "enabled", "generate_mode": "enabled",
                         "external_context_policy": "allow"})
    assert after["generate_memory"] and after["use_memory"]
    print(json.dumps({"isolated_memory_enabled": True, "revision": after["revision"]}))


if __name__ == "__main__":
    main()
