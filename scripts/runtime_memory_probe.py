#!/usr/bin/env python3
"""Read-only Linux runtime memory/latency probe; never prints credentials or API bodies."""

import argparse
import json
import pathlib
import socket
import threading
import time
import urllib.error
import urllib.request


def memory_sample(pid):
    values = {}
    for name in ("status", "smaps_rollup"):
        for line in pathlib.Path(f"/proc/{pid}/{name}").read_text().splitlines():
            key, _, raw = line.partition(":")
            if key in ("VmRSS", "VmHWM", "VmSwap", "Rss", "Pss", "SwapPss", "Threads"):
                values[key] = int(raw.split()[0])
    values["resident_and_swap_kib"] = values["Pss"] + values["SwapPss"]
    database_fds = 0
    for fd in pathlib.Path(f"/proc/{pid}/fd").iterdir():
        try:
            database_fds += str(fd.readlink()).endswith(".db")
        except OSError:
            pass
    values["database_fds"] = database_fds
    return values


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--label", required=True)
    parser.add_argument("--cycles", type=int, default=4)
    parser.add_argument("--settle-seconds", type=int, default=90)
    args = parser.parse_args()
    if args.cycles < 1 or args.settle_seconds < 0:
        parser.error("cycles must be positive and settle-seconds nonnegative")
    root = args.root.resolve()
    executable = (root / "target/release/clawd").resolve()
    pids = []
    for proc in pathlib.Path("/proc").glob("[0-9]*"):
        try:
            if (proc / "exe").resolve(strict=True) == executable:
                pids.append(int(proc.name))
        except (OSError, RuntimeError):
            pass
    if len(pids) != 1:
        raise SystemExit(f"expected one core process, found {len(pids)}")
    pid = pids[0]
    sessions = json.loads((root / "data/webd_sessions.json").read_text())["sessions"]
    key = next(row["user_key"] for row in sessions.values() if row.get("role") == "admin")
    deadline = time.monotonic() + 300
    while True:
        try:
            with socket.create_connection(("127.0.0.1", 8787), timeout=1):
                break
        except OSError:
            if time.monotonic() >= deadline:
                raise SystemExit("core API did not become ready within 300 seconds")
            time.sleep(1)
    stop = threading.Event()
    samples = []

    def sample():
        while not stop.wait(0.2):
            samples.append(memory_sample(pid))

    def emit(event, **fields):
        print(json.dumps({"label": args.label, "event": event, "pid": pid, **fields}), flush=True)

    emit("start", memory=memory_sample(pid))
    monitor = threading.Thread(target=sample, daemon=True)
    monitor.start()
    failed = 0
    try:
        for cycle in range(args.cycles):
            for endpoint in ("/v1/health", "/v1/aipps", "/v1/skills/store"):
                start = time.monotonic()
                request = urllib.request.Request(
                    "http://127.0.0.1:8787" + endpoint, headers={"x-agent-key": key}
                )
                with urllib.request.urlopen(request, timeout=180) as response:
                    raw = response.read()
                    data = json.loads(raw)
                    ok = response.status == 200 and data.get("ok", True) is not False
                    failed += not ok
                    emit("request", cycle=cycle + 1, endpoint=endpoint, ok=ok,
                         response_bytes=len(raw), seconds=round(time.monotonic() - start, 3),
                         memory=memory_sample(pid))
                time.sleep(1)
        emit("workload_done", memory=memory_sample(pid))
        time.sleep(args.settle_seconds)
        emit("settled", memory=memory_sample(pid))
    finally:
        stop.set()
        monitor.join()
        if samples:
            emit("peak", memory={key: max(s[key] for s in samples) for key in samples[0]})
    return int(failed > 0)


if __name__ == "__main__":
    raise SystemExit(main())
