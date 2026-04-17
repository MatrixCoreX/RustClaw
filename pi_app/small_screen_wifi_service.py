import subprocess


def _wifi_sort_key(item):
    active_rank = 0 if item.get("active") else 1
    signal_rank = -(item.get("signal") or 0)
    name_rank = (item.get("ssid") or "").lower()
    return (active_rank, signal_rank, name_rank)


def _split_nmcli_escaped(line, expected_parts=4):
    parts = []
    current = []
    escaped = False
    for ch in line:
        if escaped:
            current.append(ch)
            escaped = False
            continue
        if ch == "\\":
            escaped = True
            continue
        if ch == ":" and len(parts) < expected_parts - 1:
            parts.append("".join(current))
            current = []
            continue
        current.append(ch)
    parts.append("".join(current))
    while len(parts) < expected_parts:
        parts.append("")
    return parts[:expected_parts]


def scan_wifi_networks():
    try:
        result = subprocess.run(
            [
                "nmcli",
                "-t",
                "--escape",
                "yes",
                "-f",
                "IN-USE,SSID,SECURITY,SIGNAL",
                "dev",
                "wifi",
                "list",
                "--rescan",
                "yes",
            ],
            capture_output=True,
            text=True,
            timeout=15,
            check=False,
        )
    except FileNotFoundError:
        return None, "nmcli not found"
    except Exception as exc:
        return None, str(exc)
    if result.returncode != 0:
        error = (result.stderr or result.stdout or "nmcli failed").strip()
        return None, error

    dedup = {}
    for raw_line in (result.stdout or "").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        active, ssid, security, signal_text = _split_nmcli_escaped(line, expected_parts=4)
        ssid = (ssid or "").strip()
        if not ssid:
            continue
        try:
            signal = int((signal_text or "0").strip() or "0")
        except Exception:
            signal = 0
        item = {
            "active": active.strip().lower() in ("*", "yes", "true", "activated"),
            "ssid": ssid,
            "security": (security or "").strip(),
            "signal": max(0, min(signal, 100)),
        }
        existing = dedup.get(ssid)
        if existing is None or _wifi_sort_key(item) < _wifi_sort_key(existing):
            dedup[ssid] = item
    return sorted(dedup.values(), key=_wifi_sort_key), None


def connect_wifi_network(ssid, password=""):
    ssid = (ssid or "").strip()
    if not ssid:
        return False, "SSID required"
    cmd = ["nmcli", "dev", "wifi", "connect", ssid]
    if password:
        cmd += ["password", password]

    def _run_connect():
        return subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=30,
            check=False,
        )

    try:
        result = _run_connect()
    except FileNotFoundError:
        return False, "nmcli not found"
    except Exception as exc:
        return False, str(exc)
    if result.returncode == 0:
        return True, (result.stdout or "").strip()
    error = (result.stderr or result.stdout or "nmcli failed").strip()
    lower_error = error.lower()
    should_retry = (
        bool(password)
        and (
            "property is missing" in lower_error
            or "secrets were required" in lower_error
            or "no valid secrets" in lower_error
        )
    )
    if should_retry:
        try:
            cleanup = subprocess.run(
                ["nmcli", "connection", "delete", "id", ssid],
                capture_output=True,
                text=True,
                timeout=15,
                check=False,
            )
            if cleanup.returncode == 0 or "unknown connection" in (cleanup.stderr or "").lower():
                retry = _run_connect()
                if retry.returncode == 0:
                    return True, (retry.stdout or "").strip()
                retry_error = (retry.stderr or retry.stdout or "nmcli failed").strip()
                return False, retry_error
        except FileNotFoundError:
            return False, "nmcli not found"
        except Exception as exc:
            return False, str(exc)
    return False, error


def disconnect_wifi_network(ssid=""):
    ssid = (ssid or "").strip()
    try:
        status = subprocess.run(
            [
                "nmcli",
                "-t",
                "--escape",
                "no",
                "-f",
                "DEVICE,TYPE,STATE,CONNECTION",
                "device",
                "status",
            ],
            capture_output=True,
            text=True,
            timeout=10,
            check=False,
        )
    except FileNotFoundError:
        return False, "nmcli not found"
    except Exception as exc:
        return False, str(exc)
    if status.returncode != 0:
        error = (status.stderr or status.stdout or "nmcli failed").strip()
        return False, error

    target_device = ""
    for raw_line in (status.stdout or "").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        parts = line.split(":")
        if len(parts) < 4:
            continue
        device, dev_type, state = parts[0].strip(), parts[1].strip(), parts[2].strip().lower()
        connection = ":".join(parts[3:]).strip()
        if dev_type != "wifi" or state != "connected":
            continue
        if not ssid or connection == ssid:
            target_device = device
            break
    if not target_device and ssid:
        try:
            result = subprocess.run(
                ["nmcli", "connection", "down", "id", ssid],
                capture_output=True,
                text=True,
                timeout=20,
                check=False,
            )
        except FileNotFoundError:
            return False, "nmcli not found"
        except Exception as exc:
            return False, str(exc)
        if result.returncode == 0:
            return True, (result.stdout or "").strip()
        error = (result.stderr or result.stdout or "nmcli failed").strip()
        return False, error
    if not target_device:
        return False, "connected wifi device not found"
    try:
        result = subprocess.run(
            ["nmcli", "device", "disconnect", target_device],
            capture_output=True,
            text=True,
            timeout=20,
            check=False,
        )
    except FileNotFoundError:
        return False, "nmcli not found"
    except Exception as exc:
        return False, str(exc)
    if result.returncode == 0:
        return True, (result.stdout or "").strip()
    error = (result.stderr or result.stdout or "nmcli failed").strip()
    return False, error
