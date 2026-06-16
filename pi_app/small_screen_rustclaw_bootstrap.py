import http.client
import os
import shlex
import subprocess
import time

from small_screen_config import _root_dir


HEALTH_HOST = "127.0.0.1"
HEALTH_PORT = 8787
HEALTH_PATH = "/v1/health"
START_LOG_NAME = "pi-small-screen-rustclaw-start.log"


def _http_health_ok(timeout=1.5):
    conn = None
    try:
        conn = http.client.HTTPConnection(HEALTH_HOST, HEALTH_PORT, timeout=timeout)
        conn.request("GET", HEALTH_PATH)
        resp = conn.getresponse()
        resp.read()
        return 200 <= int(resp.status) < 300
    except Exception:
        return False
    finally:
        if conn is not None:
            try:
                conn.close()
            except Exception:
                pass


def _pid_cmdline(pid):
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\0", b" ").decode("utf-8", errors="ignore")
    except Exception:
        return ""


def _pid_alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except Exception:
        return False


def _pid_file_clawd_running(root):
    pid_path = os.path.join(root, ".pids", "clawd.pid")
    try:
        with open(pid_path, "r", encoding="utf-8") as f:
            raw = f.read().strip()
        pid = int(raw)
    except Exception:
        return False
    if not _pid_alive(pid):
        return False
    cmdline = _pid_cmdline(pid)
    return "clawd" in cmdline if cmdline else True


def _pgrep_clawd_running():
    try:
        result = subprocess.run(
            ["pgrep", "-f", r"target/release/clawd|cargo run -p clawd|/clawd(\s|$)"],
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
        return result.returncode == 0
    except Exception:
        return False


def rustclaw_is_running(root=None):
    root = root or _root_dir()
    return _http_health_ok() or _pid_file_clawd_running(root) or _pgrep_clawd_running()


def _configured_start_command():
    raw = os.environ.get("RUSTCLAW_SMALL_SCREEN_START_CMD", "").strip()
    if not raw:
        return None
    try:
        return shlex.split(raw)
    except ValueError:
        return None


def _default_start_command(root):
    configured = _configured_start_command()
    if configured:
        return configured
    cli_path = os.path.join(root, "rustclaw")
    if os.path.isfile(cli_path) and os.access(cli_path, os.X_OK):
        return [cli_path, "-restart", "release", "all", "--quick", "--skip-setup"]
    start_all_bin = os.path.join(root, "start-all-bin.sh")
    if os.path.isfile(start_all_bin):
        return ["bash", start_all_bin, "release"]
    return None


def _spawn_start_command(root, cmd, popen_factory=subprocess.Popen):
    log_dir = os.path.join(root, "logs")
    os.makedirs(log_dir, exist_ok=True)
    log_path = os.path.join(log_dir, START_LOG_NAME)
    with open(log_path, "ab") as log:
        popen_factory(
            cmd,
            cwd=root,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
            close_fds=True,
        )
    return log_path


def ensure_rustclaw_started(root=None, wait_seconds=3.0, popen_factory=subprocess.Popen):
    root = root or _root_dir()
    if rustclaw_is_running(root):
        return False
    cmd = _default_start_command(root)
    if not cmd:
        print("RustClaw startup command not found; small screen will keep retrying health checks.")
        return False
    log_path = _spawn_start_command(root, cmd, popen_factory=popen_factory)
    deadline = time.monotonic() + max(0.0, float(wait_seconds or 0.0))
    while time.monotonic() < deadline:
        if rustclaw_is_running(root):
            break
        time.sleep(0.25)
    print(f"RustClaw startup requested by small screen; log={log_path}")
    return True
