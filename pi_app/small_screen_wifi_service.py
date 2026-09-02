import logging
import os
import subprocess


logger = logging.getLogger(__name__)

_NETWORK_CONTROL_PERMISSION = "org.freedesktop.NetworkManager.network-control"
_OPEN_SECURITY_TOKENS = {"", "--", "NONE", "OPEN"}


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
    if escaped:
        current.append("\\")
    parts.append("".join(current))
    while len(parts) < expected_parts:
        parts.append("")
    return parts[:expected_parts]


def _error(error_code, detail="", return_code=None):
    payload = {"error_code": str(error_code or "operation_failed")}
    if detail:
        payload["detail"] = str(detail).strip()
    if return_code is not None:
        payload["return_code"] = int(return_code)
    return payload


def _success(detail=""):
    payload = {}
    if detail:
        payload["detail"] = str(detail).strip()
    return True, payload


def _run_nmcli(args, timeout):
    env = os.environ.copy()
    env.update({"LC_ALL": "C", "LANG": "C"})
    try:
        result = subprocess.run(
            ["nmcli", *args],
            capture_output=True,
            env=env,
            text=True,
            timeout=timeout,
            check=False,
        )
    except FileNotFoundError:
        return None, _error("nmcli_missing")
    except subprocess.TimeoutExpired as exc:
        return None, _error("timeout", detail=str(exc))
    except Exception as exc:
        logger.warning("WiFi command failed before completion: %s", exc)
        return None, _error("operation_failed", detail=str(exc))
    return result, None


def _result_error(result, default_code):
    return_code = int(getattr(result, "returncode", 1) or 1)
    code_by_exit = {
        3: "timeout",
        8: "network_manager_unavailable",
        10: "network_not_found",
    }
    detail = (getattr(result, "stderr", "") or getattr(result, "stdout", "") or "").strip()
    return _error(code_by_exit.get(return_code, default_code), detail=detail, return_code=return_code)


def _network_control_error():
    result, error = _run_nmcli(
        ["-t", "--escape", "no", "-f", "PERMISSION,VALUE", "general", "permissions"],
        timeout=10,
    )
    if error:
        return error
    if result.returncode != 0:
        return _result_error(result, "permission_check_failed")
    permission_value = None
    for raw_line in (result.stdout or "").splitlines():
        permission, _, value = raw_line.partition(":")
        if permission.strip() == _NETWORK_CONTROL_PERMISSION:
            permission_value = value.strip().lower()
            break
    if permission_value in (None, "yes"):
        return None
    return _error("permission_required", detail=permission_value)


def _normalize_security(value):
    security = str(value or "").strip()
    return "" if security.upper() in _OPEN_SECURITY_TOKENS else security


def _saved_wifi_profiles():
    result, error = _run_nmcli(
        ["-t", "--escape", "yes", "-f", "NAME,UUID,TYPE", "connection", "show"],
        timeout=10,
    )
    if error or result.returncode != 0:
        return {}
    profiles = {}
    for raw_line in (result.stdout or "").splitlines():
        profile_name, uuid, connection_type = _split_nmcli_escaped(raw_line.strip(), expected_parts=3)
        if connection_type.strip() not in {"802-11-wireless", "wifi", "wireless"}:
            continue
        profile_name = profile_name.strip()
        uuid = uuid.strip()
        if not profile_name or not uuid:
            continue
        ssid_result, ssid_error = _run_nmcli(
            ["-g", "802-11-wireless.ssid", "connection", "show", "uuid", uuid],
            timeout=10,
        )
        if ssid_error or ssid_result.returncode != 0:
            continue
        ssid = (ssid_result.stdout or "").strip() or profile_name
        profiles.setdefault(ssid, profile_name)
    return profiles


def scan_wifi_networks():
    result, error = _run_nmcli(
        [
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
        timeout=20,
    )
    if error:
        return None, error
    if result.returncode != 0:
        return None, _result_error(result, "scan_failed")

    saved_profiles = _saved_wifi_profiles()
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
        except (TypeError, ValueError):
            signal = 0
        is_active = active.strip().lower() in ("*", "yes", "true", "activated")
        profile_name = saved_profiles.get(ssid, "")
        if is_active and not profile_name:
            profile_name = ssid
        item = {
            "active": is_active,
            "ssid": ssid,
            "security": _normalize_security(security),
            "signal": max(0, min(signal, 100)),
            "saved": bool(profile_name),
            "profile_name": profile_name,
        }
        existing = dedup.get(ssid)
        if existing is None or _wifi_sort_key(item) < _wifi_sort_key(existing):
            dedup[ssid] = item
    return sorted(dedup.values(), key=_wifi_sort_key), None


def connect_wifi_network(ssid, password="", profile_name=""):
    ssid = (ssid or "").strip()
    password = str(password or "")
    profile_name = (profile_name or "").strip()
    if not ssid:
        return False, _error("ssid_required")
    permission_error = _network_control_error()
    if permission_error:
        return False, permission_error

    if profile_name and not password:
        result, error = _run_nmcli(["connection", "up", "id", profile_name], timeout=40)
        if error:
            return False, error
        if result.returncode == 0:
            return _success(result.stdout)
        return False, _result_error(result, "activation_failed")

    if profile_name and password:
        result, error = _run_nmcli(
            ["connection", "modify", "id", profile_name, "802-11-wireless-security.psk", password],
            timeout=20,
        )
        if error:
            return False, error
        if result.returncode != 0:
            return False, _result_error(result, "password_update_failed")
        result, error = _run_nmcli(["connection", "up", "id", profile_name], timeout=40)
    else:
        args = ["dev", "wifi", "connect", ssid]
        if password:
            args.extend(["password", password])
        result, error = _run_nmcli(args, timeout=40)
    if error:
        return False, error
    if result.returncode == 0:
        return _success(result.stdout)
    return False, _result_error(result, "activation_failed")


def disconnect_wifi_network(ssid="", profile_name=""):
    ssid = (ssid or "").strip()
    profile_name = (profile_name or "").strip()
    permission_error = _network_control_error()
    if permission_error:
        return False, permission_error

    status, error = _run_nmcli(
        ["-t", "--escape", "yes", "-f", "DEVICE,TYPE,STATE,CONNECTION", "device", "status"],
        timeout=10,
    )
    if error:
        return False, error
    if status.returncode != 0:
        return False, _result_error(status, "status_failed")

    target_device = ""
    for raw_line in (status.stdout or "").splitlines():
        line = raw_line.strip()
        if not line:
            continue
        device, dev_type, state, connection = _split_nmcli_escaped(line, expected_parts=4)
        if dev_type.strip() not in {"wifi", "802-11-wireless"} or state.strip().lower() != "connected":
            continue
        connection = connection.strip()
        if not ssid or connection in {ssid, profile_name}:
            target_device = device.strip()
            break
    if not target_device and profile_name:
        result, error = _run_nmcli(["connection", "down", "id", profile_name], timeout=30)
        if error:
            return False, error
        if result.returncode == 0:
            return _success(result.stdout)
        return False, _result_error(result, "deactivation_failed")
    if not target_device:
        return False, _error("connected_device_not_found")

    result, error = _run_nmcli(["device", "disconnect", target_device], timeout=30)
    if error:
        return False, error
    if result.returncode == 0:
        return _success(result.stdout)
    return False, _result_error(result, "deactivation_failed")
