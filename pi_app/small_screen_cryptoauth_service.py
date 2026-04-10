import json
import os
import shutil
import subprocess

from small_screen_config import _pi_app_dir


def _existing_dirs(paths):
    seen = set()
    result = []
    for path in paths:
        if not path:
            continue
        full = os.path.abspath(path)
        if full in seen or not os.path.isdir(full):
            continue
        seen.add(full)
        result.append(full)
    return result


def _existing_files(paths):
    seen = set()
    result = []
    for path in paths:
        if not path:
            continue
        full = os.path.abspath(path)
        if full in seen or not os.path.isfile(full):
            continue
        seen.add(full)
        result.append(full)
    return result


def _cryptoauthlib_root():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    candidates = _existing_dirs(
        [
            os.environ.get("RUSTCLAW_CRYPTOAUTHLIB_ROOT"),
            os.path.join(_pi_app_dir(), "vendor", "cryptoauthlib"),
            os.path.join(script_dir, "vendor", "cryptoauthlib"),
            os.path.join(script_dir, "..", "..", "cryptoauthlib"),
            os.path.join(_pi_app_dir(), "..", "..", "cryptoauthlib"),
            "/home/pi/cryptoauthlib",
        ]
    )
    return candidates[0] if candidates else None


def _cryptoauthlib_lib_dirs(root_dir):
    if not root_dir:
        return []
    return _existing_dirs(
        [
            os.environ.get("RUSTCLAW_CRYPTOAUTHLIB_LIB_DIR"),
            os.path.join(root_dir, "build-pyfix"),
            os.path.join(root_dir, "build-pyfix", "lib"),
            os.path.join(root_dir, "build", "lib"),
            os.path.join(root_dir, "python", "cryptoauthlib"),
        ]
    )


def _cryptoauthlib_pythonpath_entries(root_dir):
    if not root_dir:
        return []
    return _existing_dirs(
        [
            os.path.join(root_dir, "python"),
        ]
    )


def _cryptoauthlib_python_candidates(root_dir):
    candidates = _existing_files(
        [
            os.environ.get("RUSTCLAW_CRYPTOAUTHLIB_PYTHON"),
            os.path.join(root_dir, "python", ".venv", "bin", "python") if root_dir else None,
            os.path.join(root_dir, "python", "venv", "bin", "python") if root_dir else None,
        ]
    )
    system_python = shutil.which("python3")
    if system_python:
        candidates.append(system_python)
    return candidates


def _select_helper_python(root_dir):
    candidates = _cryptoauthlib_python_candidates(root_dir)
    if not candidates:
        return None
    return candidates[0]


def _run_signature_helper(args):
    script_path = os.path.join(_pi_app_dir(), "signature.py")
    if not os.path.isfile(script_path):
        return None, "helper script not found"
    root_dir = _cryptoauthlib_root()
    if not root_dir:
        return None, "cryptoauthlib root not found"
    python_exec = _select_helper_python(root_dir)
    if not python_exec:
        return None, "cryptoauthlib python not found"
    env = os.environ.copy()
    env["RUSTCLAW_CRYPTOAUTHLIB_ROOT"] = root_dir
    lib_dirs = _cryptoauthlib_lib_dirs(root_dir)
    if lib_dirs:
        env["LD_LIBRARY_PATH"] = ":".join(lib_dirs + [env.get("LD_LIBRARY_PATH", "")]).rstrip(":")
    pythonpath_entries = _cryptoauthlib_pythonpath_entries(root_dir)
    if pythonpath_entries:
        env["PYTHONPATH"] = ":".join(pythonpath_entries + [env.get("PYTHONPATH", "")]).rstrip(":")
    try:
        result = subprocess.run(
            [python_exec, script_path, *args],
            capture_output=True,
            text=True,
            timeout=12,
            check=False,
            env=env,
        )
    except Exception as exc:
        return None, str(exc)
    raw = (result.stdout or "").strip()
    if not raw:
        return None, (result.stderr or "empty helper response").strip()
    try:
        payload = json.loads(raw)
    except Exception:
        return None, raw
    if payload.get("ok"):
        return payload, ""
    return None, str(payload.get("error") or "helper request failed")


def read_slot0_pubkey_via_helper():
    payload, error = _run_signature_helper(["pubkey"])
    if payload and payload.get("pubkey"):
        return str(payload.get("pubkey")).strip(), ""
    return None, error or "public key unavailable"


def read_tng_device_pubkey_via_helper():
    payload, error = _run_signature_helper(["tng_device_pubkey"])
    if payload and payload.get("pubkey"):
        return payload, ""
    return None, error or "tng device public key unavailable"


def read_tng_device_cert_via_helper():
    payload, error = _run_signature_helper(["tng_device_cert"])
    if payload and payload.get("device_cert_hex"):
        return payload, ""
    return None, error or "tng device cert unavailable"


def read_tng_signer_cert_via_helper():
    payload, error = _run_signature_helper(["tng_signer_cert"])
    if payload and payload.get("signer_cert_hex"):
        return payload, ""
    return None, error or "tng signer cert unavailable"


def read_tng_root_cert_via_helper():
    payload, error = _run_signature_helper(["tng_root_cert"])
    if payload and payload.get("root_cert_hex"):
        return payload, ""
    return None, error or "tng root cert unavailable"


def sign_unix_time_via_helper(unix_time):
    payload, error = _run_signature_helper(["sign_timestamp", str(int(unix_time))])
    if payload and payload.get("signature"):
        return payload, ""
    return None, error or "sign failed"
