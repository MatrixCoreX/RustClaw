#!/usr/bin/env python3
"""Measure runtime process-tree memory without printing credentials or request bodies."""

import argparse
import json
import os
import pathlib
import platform
import socket
import subprocess
import threading
import time
import urllib.request


LINUX_MEMORY_FIELDS = (
    "VmRSS",
    "VmHWM",
    "VmSwap",
    "Rss",
    "Pss",
    "SwapPss",
    "Threads",
)

AGGREGATE_MEMORY_FIELDS = (
    "VmRSS",
    "VmSwap",
    "Pss",
    "SwapPss",
    "resident_and_swap_kib",
    "Threads",
    "database_fds",
    "fd_count",
)


def parse_key_values(text):
    values = {}
    for line in text.splitlines():
        fields = line.split()
        if len(fields) == 2:
            try:
                values[fields[0]] = int(fields[1])
            except ValueError:
                continue
    return values


def parse_pressure(text):
    result = {}
    for line in text.splitlines():
        fields = line.split()
        if not fields:
            continue
        scope = fields[0]
        result[scope] = {}
        for field in fields[1:]:
            key, separator, value = field.partition("=")
            if not separator:
                continue
            try:
                result[scope][key] = float(value) if key.startswith("avg") else int(value)
            except ValueError:
                continue
    return result


def process_role(executable_name):
    name = executable_name.lower()
    if name == "clawd":
        return "core"
    if name == "webd":
        return "web_gateway"
    if name == "skill-runner":
        return "skill_runner"
    if name in {"chromium", "chromium-browser", "chrome", "google-chrome"}:
        return "browser"
    if name in {
        "wechatd",
        "telegramd",
        "whatsappd",
        "wa-webd",
        "feishud",
        "larkd",
    }:
        return "channel"
    if name in {"python", "python3", "node", "go"}:
        return "skill_process"
    return "child_process"


def linux_process_table():
    rows = {}
    for proc in pathlib.Path("/proc").glob("[0-9]*"):
        try:
            status = (proc / "status").read_text(errors="replace")
            fields = {}
            for line in status.splitlines():
                key, separator, value = line.partition(":")
                if separator:
                    fields[key] = value.strip()
            pid = int(proc.name)
            ppid = int(fields.get("PPid", "0"))
            executable = proc_executable_path(proc / "exe")
            rows[pid] = {
                "pid": pid,
                "ppid": ppid,
                "executable": executable,
                "name": executable.name,
            }
        except (OSError, RuntimeError, ValueError):
            continue
    return rows


def proc_executable_path(link):
    return pathlib.Path(normalize_proc_executable_target(os.readlink(link)))


def normalize_proc_executable_target(target):
    deleted_suffix = " (deleted)"
    if target.endswith(deleted_suffix):
        target = target[: -len(deleted_suffix)]
    return target


def darwin_process_table():
    output = subprocess.run(
        ["/bin/ps", "-axo", "pid=,ppid=,comm="],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    rows = {}
    for line in output.splitlines():
        fields = line.strip().split(None, 2)
        if len(fields) != 3:
            continue
        try:
            pid, ppid = int(fields[0]), int(fields[1])
        except ValueError:
            continue
        executable = pathlib.Path(fields[2])
        rows[pid] = {
            "pid": pid,
            "ppid": ppid,
            "executable": executable,
            "name": executable.name,
        }
    return rows


def process_table():
    if platform.system() == "Linux":
        return linux_process_table()
    if platform.system() == "Darwin":
        return darwin_process_table()
    raise SystemExit("runtime memory probe supports Linux and macOS")


def descendant_pids(root_pids, rows):
    descendants = {root_pids} if isinstance(root_pids, int) else set(root_pids)
    changed = True
    while changed:
        changed = False
        for pid, row in rows.items():
            if pid not in descendants and row["ppid"] in descendants:
                descendants.add(pid)
                changed = True
    return sorted(descendants)


def runtime_root_pids(root_pid, rows, runtime_root):
    roots = {root_pid}
    if runtime_root is None:
        return roots
    release_directories = {
        (runtime_root / "target/release").resolve(),
        (runtime_root / "release-bin").resolve(),
    }
    for pid, row in rows.items():
        try:
            executable = row["executable"].resolve(strict=False)
        except (OSError, RuntimeError):
            continue
        if executable.parent in release_directories:
            roots.add(pid)
    return roots


def linux_memory_sample(pid):
    values = {}
    for name in ("status", "smaps_rollup"):
        try:
            lines = pathlib.Path(f"/proc/{pid}/{name}").read_text().splitlines()
        except OSError:
            continue
        for line in lines:
            key, _, raw = line.partition(":")
            if key in LINUX_MEMORY_FIELDS:
                try:
                    values[key] = int(raw.split()[0])
                except (IndexError, ValueError):
                    continue
    values.setdefault("Pss", values.get("VmRSS", 0))
    values.setdefault("SwapPss", values.get("VmSwap", 0))
    values.setdefault("VmRSS", values.get("Rss", 0))
    values.setdefault("VmHWM", values.get("VmRSS", 0))
    values.setdefault("VmSwap", values.get("SwapPss", 0))
    values.setdefault("Threads", 0)
    values["resident_and_swap_kib"] = values["Pss"] + values["SwapPss"]
    values["database_fds"] = 0
    values["fd_count"] = 0
    try:
        for fd in pathlib.Path(f"/proc/{pid}/fd").iterdir():
            values["fd_count"] += 1
            try:
                values["database_fds"] += str(fd.readlink()).endswith((".db", ".sqlite"))
            except OSError:
                pass
    except OSError:
        pass
    return values


def parse_darwin_memory_rows(output, includes_threads):
    rows = {}
    for line in output.splitlines():
        fields = line.split()
        expected_fields = 4 if includes_threads else 3
        if len(fields) != expected_fields:
            continue
        try:
            pid, rss, virtual = map(int, fields[:3])
            threads = int(fields[3]) if includes_threads else None
        except ValueError:
            continue
        rows[pid] = {
            "VmRSS": rss,
            "VmHWM": None,
            "VmSwap": None,
            "Pss": None,
            "SwapPss": None,
            "Threads": threads,
            "resident_and_swap_kib": rss,
            "virtual_kib": virtual,
            "database_fds": None,
            "fd_count": None,
        }
    return rows


def darwin_memory_rows():
    extended = subprocess.run(
        ["/bin/ps", "-axo", "pid=,rss=,vsz=,thcount="],
        check=False,
        capture_output=True,
        text=True,
    )
    if extended.returncode == 0:
        return parse_darwin_memory_rows(extended.stdout, includes_threads=True)

    # macOS ps field support varies by OS release. RSS and VSZ remain useful;
    # an unavailable thread count must stay null instead of being invented.
    portable = subprocess.run(
        ["/bin/ps", "-axo", "pid=,rss=,vsz="],
        check=True,
        capture_output=True,
        text=True,
    )
    return parse_darwin_memory_rows(portable.stdout, includes_threads=False)


def aggregate_processes(processes):
    aggregate = {}
    for field in AGGREGATE_MEMORY_FIELDS:
        present = [row["memory"].get(field) for row in processes]
        present = [value for value in present if isinstance(value, int)]
        aggregate[field] = sum(present) if present else None
    aggregate["process_count"] = len(processes)
    return aggregate


def aggregate_process_groups(processes, field):
    return {
        value: aggregate_processes([row for row in processes if row[field] == value])
        for value in sorted({row[field] for row in processes})
    }


def process_tree_sample(root_pid, runtime_root=None):
    rows = process_table()
    roots = runtime_root_pids(root_pid, rows, runtime_root)
    pids = descendant_pids(roots, rows)
    darwin_memory = darwin_memory_rows() if platform.system() == "Darwin" else {}
    processes = []
    for pid in pids:
        row = rows.get(pid)
        if row is None:
            continue
        memory = (
            linux_memory_sample(pid)
            if platform.system() == "Linux"
            else darwin_memory.get(pid)
        )
        if not memory:
            continue
        processes.append(
            {
                "pid": pid,
                "ppid": row["ppid"],
                "role": process_role(row["name"]),
                "executable_name": row["name"],
                "memory": memory,
            }
        )
    return {
        "aggregate": aggregate_processes(processes),
        "roles": aggregate_process_groups(processes, "role"),
        "executables": aggregate_process_groups(processes, "executable_name"),
        "processes": processes,
    }


def linux_cgroup_directory(pid):
    try:
        membership = pathlib.Path(f"/proc/{pid}/cgroup").read_text()
    except OSError:
        return None
    for line in membership.splitlines():
        fields = line.split(":", 2)
        if len(fields) == 3 and fields[0] == "0" and not fields[1]:
            root = pathlib.Path("/sys/fs/cgroup")
            candidate = root / fields[2].lstrip("/")
            return candidate if candidate.exists() else root
    return None


def system_pressure_sample(root_pid):
    if platform.system() != "Linux":
        return {
            "cgroup": None,
            "system_memory_pressure": None,
            "unavailable": ["cgroup", "system_memory_pressure"],
        }
    result = {"cgroup": None, "system_memory_pressure": None, "unavailable": []}
    directory = linux_cgroup_directory(root_pid)
    if directory is not None:
        cgroup = {}
        for filename in ("memory.current", "memory.peak"):
            key = filename.replace(".", "_") + "_bytes"
            try:
                cgroup[key] = int((directory / filename).read_text().strip())
            except (OSError, ValueError):
                cgroup[key] = None
        try:
            cgroup["memory_events"] = parse_key_values(
                (directory / "memory.events").read_text()
            )
        except OSError:
            cgroup["memory_events"] = None
        try:
            cgroup["memory_pressure"] = parse_pressure(
                (directory / "memory.pressure").read_text()
            )
        except OSError:
            cgroup["memory_pressure"] = None
        result["cgroup"] = cgroup
    else:
        result["unavailable"].append("cgroup")
    try:
        result["system_memory_pressure"] = parse_pressure(
            pathlib.Path("/proc/pressure/memory").read_text()
        )
    except OSError:
        result["unavailable"].append("system_memory_pressure")
    return result


def discover_root_pid(executable, explicit_pid):
    if explicit_pid is not None:
        return explicit_pid
    expected = executable.resolve()
    matches = []
    for pid, row in process_table().items():
        try:
            if row["executable"].resolve(strict=True) == expected:
                matches.append(pid)
        except (OSError, RuntimeError):
            continue
    if len(matches) != 1:
        raise SystemExit(f"expected one core process, found {len(matches)}")
    return matches[0]


def select_admin_key(sessions):
    for row in sessions.values():
        key = row.get("user_key")
        if row.get("role") == "admin" and isinstance(key, str) and key:
            return key
    raise ValueError("no nonempty administrator API key is available for the probe")


def load_admin_key(root):
    sessions = json.loads((root / "data/webd_sessions.json").read_text())["sessions"]
    return select_admin_key(sessions)


def wait_for_api(api_base, timeout_seconds):
    host_port = api_base.removeprefix("http://").removeprefix("https://").split("/", 1)[0]
    host, separator, raw_port = host_port.rpartition(":")
    if not separator:
        host, raw_port = host_port, "443" if api_base.startswith("https://") else "80"
    deadline = time.monotonic() + timeout_seconds
    while True:
        try:
            with socket.create_connection((host, int(raw_port)), timeout=1):
                return
        except OSError:
            if time.monotonic() >= deadline:
                raise SystemExit(f"core API did not become ready within {timeout_seconds} seconds")
            time.sleep(1)


def self_test():
    assert parse_key_values("low 1\nhigh 2\n") == {"low": 1, "high": 2}
    pressure = parse_pressure("some avg10=1.25 avg60=2.5 total=9\n")
    assert pressure["some"]["avg10"] == 1.25
    assert pressure["some"]["total"] == 9
    darwin_portable = parse_darwin_memory_rows("10 2048 4096\n", False)
    assert darwin_portable[10]["VmRSS"] == 2048
    assert darwin_portable[10]["Threads"] is None
    darwin_extended = parse_darwin_memory_rows("11 1024 8192 7\n", True)
    assert darwin_extended[11]["Threads"] == 7
    assert normalize_proc_executable_target(
        "/repo/target/release/clawd (deleted)"
    ) == "/repo/target/release/clawd"
    assert process_role("clawd") == "core"
    assert process_role("chromium") == "browser"
    rows = {
        10: {"ppid": 1, "executable": pathlib.Path("/repo/target/release/clawd")},
        11: {"ppid": 10, "executable": pathlib.Path("/usr/bin/python3")},
        20: {"ppid": 1, "executable": pathlib.Path("/repo/target/release/webd")},
        21: {"ppid": 20, "executable": pathlib.Path("/usr/bin/node")},
        30: {"ppid": 1, "executable": pathlib.Path("/usr/bin/unrelated")},
    }
    roots = runtime_root_pids(10, rows, pathlib.Path("/repo"))
    assert roots == {10, 20}
    assert descendant_pids(roots, rows) == [10, 11, 20, 21]
    grouped = aggregate_process_groups(
        [
            {
                "role": "channel",
                "executable_name": "wechatd",
                "memory": {"Pss": 10, "SwapPss": 2},
            },
            {
                "role": "channel",
                "executable_name": "telegramd",
                "memory": {"Pss": 5, "SwapPss": 1},
            },
        ],
        "executable_name",
    )
    assert grouped["wechatd"]["Pss"] == 10
    assert grouped["telegramd"]["SwapPss"] == 1
    assert select_admin_key(
        {
            "expired": {"role": "admin", "user_key": ""},
            "active": {"role": "admin", "user_key": "test-key"},
        }
    ) == "test-key"
    print(json.dumps({"self_test": "ok"}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=pathlib.Path, default=pathlib.Path.cwd())
    parser.add_argument("--label", default="runtime-memory")
    parser.add_argument("--cycles", type=int, default=4)
    parser.add_argument("--settle-seconds", type=int, default=90)
    parser.add_argument("--pid", type=int)
    parser.add_argument("--api-base", default="http://127.0.0.1:8787")
    parser.add_argument("--skip-http", action="store_true")
    parser.add_argument("--completion-file", type=pathlib.Path)
    parser.add_argument("--workload-timeout-seconds", type=int, default=900)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    if (
        args.cycles < 1
        or args.settle_seconds < 0
        or args.workload_timeout_seconds < 1
    ):
        parser.error(
            "cycles and workload-timeout-seconds must be positive; "
            "settle-seconds must be nonnegative"
        )
    if args.completion_file is not None and not args.skip_http:
        parser.error("completion-file requires skip-http")
    root = args.root.resolve()
    executable = (root / "target/release/clawd").resolve()
    root_pid = discover_root_pid(executable, args.pid)
    admin_key = None if args.skip_http else load_admin_key(root)
    if not args.skip_http:
        wait_for_api(args.api_base, 300)
    stop = threading.Event()
    samples = []

    def sample():
        while not stop.wait(0.5):
            try:
                tree = process_tree_sample(root_pid, root)
                samples.append(
                    {
                        "aggregate": tree["aggregate"],
                        "roles": tree["roles"],
                        "executables": tree["executables"],
                    }
                )
            except (OSError, subprocess.SubprocessError):
                continue

    def emit(event, **fields):
        print(
            json.dumps(
                {
                    "schema_version": 3,
                    "label": args.label,
                    "event": event,
                    "root_pid": root_pid,
                    "platform": platform.system().lower(),
                    **fields,
                },
                sort_keys=True,
            ),
            flush=True,
        )

    emit(
        "start",
        process_tree=process_tree_sample(root_pid, root),
        pressure=system_pressure_sample(root_pid),
    )
    monitor = threading.Thread(target=sample, daemon=True)
    monitor.start()
    failed = 0
    try:
        if args.completion_file is not None:
            deadline = time.monotonic() + args.workload_timeout_seconds
            while not args.completion_file.exists():
                if time.monotonic() >= deadline:
                    emit(
                        "workload_timeout",
                        process_tree=process_tree_sample(root_pid, root),
                        pressure=system_pressure_sample(root_pid),
                    )
                    failed += 1
                    break
                time.sleep(0.25)
        elif not args.skip_http:
            for cycle in range(args.cycles):
                for endpoint in ("/v1/health", "/v1/aipps", "/v1/skills/store"):
                    started = time.monotonic()
                    request = urllib.request.Request(
                        args.api_base + endpoint,
                        headers={"x-agent-key": admin_key},
                    )
                    with urllib.request.urlopen(request, timeout=180) as response:
                        raw = response.read()
                        data = json.loads(raw)
                        ok = response.status == 200 and data.get("ok", True) is not False
                        failed += not ok
                        emit(
                            "request",
                            cycle=cycle + 1,
                            endpoint=endpoint,
                            ok=ok,
                            response_bytes=len(raw),
                            seconds=round(time.monotonic() - started, 3),
                            process_tree=process_tree_sample(root_pid, root),
                            pressure=system_pressure_sample(root_pid),
                        )
                    time.sleep(1)
        emit(
            "workload_done",
            process_tree=process_tree_sample(root_pid, root),
            pressure=system_pressure_sample(root_pid),
        )
        time.sleep(args.settle_seconds)
        emit(
            "settled",
            process_tree=process_tree_sample(root_pid, root),
            pressure=system_pressure_sample(root_pid),
        )
    finally:
        stop.set()
        monitor.join()
        if samples:
            aggregates = [sample["aggregate"] for sample in samples]
            keys = set().union(*(sample.keys() for sample in aggregates))
            peaks = {}
            for key in keys:
                values = [sample.get(key) for sample in aggregates]
                values = [value for value in values if isinstance(value, int)]
                if values:
                    peaks[key] = max(values)
            role_peaks = {}
            role_names = set().union(*(sample["roles"].keys() for sample in samples))
            for role in sorted(role_names):
                role_samples = [
                    sample["roles"][role]
                    for sample in samples
                    if role in sample["roles"]
                ]
                role_keys = set().union(*(sample.keys() for sample in role_samples))
                role_peaks[role] = {}
                for key in role_keys:
                    values = [sample.get(key) for sample in role_samples]
                    values = [value for value in values if isinstance(value, int)]
                    if values:
                        role_peaks[role][key] = max(values)
            executable_peaks = {}
            executable_names = set().union(
                *(sample["executables"].keys() for sample in samples)
            )
            for executable_name in sorted(executable_names):
                executable_samples = [
                    sample["executables"][executable_name]
                    for sample in samples
                    if executable_name in sample["executables"]
                ]
                executable_keys = set().union(
                    *(sample.keys() for sample in executable_samples)
                )
                executable_peaks[executable_name] = {}
                for key in executable_keys:
                    values = [sample.get(key) for sample in executable_samples]
                    values = [value for value in values if isinstance(value, int)]
                    if values:
                        executable_peaks[executable_name][key] = max(values)
            emit(
                "peak",
                aggregate=peaks,
                roles=role_peaks,
                executables=executable_peaks,
                sample_count=len(samples),
            )
    return int(failed > 0)


if __name__ == "__main__":
    raise SystemExit(main())
