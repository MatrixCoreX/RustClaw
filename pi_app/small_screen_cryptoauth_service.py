import json
import os
import subprocess

from small_screen_config import _pi_app_dir

CRYPTOAUTHLIB_PYTHON = "../../cryptoauthlib/python/.venv/bin/python"
CRYPTOAUTHLIB_LIB_DIR = "../../cryptoauthlib/build-pyfix"


def _run_signature_helper(args):
    script_path = os.path.join(_pi_app_dir(), "signature.py")
    if not os.path.isfile(script_path):
        return None, "helper script not found"
    if not os.path.isfile(CRYPTOAUTHLIB_PYTHON):
        return None, "cryptoauthlib python not found"
    env = os.environ.copy()
    if os.path.isdir(CRYPTOAUTHLIB_LIB_DIR):
        env["LD_LIBRARY_PATH"] = f"{CRYPTOAUTHLIB_LIB_DIR}:{env.get('LD_LIBRARY_PATH', '')}".rstrip(":")
    try:
        result = subprocess.run(
            [CRYPTOAUTHLIB_PYTHON, script_path, *args],
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


def sign_unix_time_via_helper(unix_time):
    payload, error = _run_signature_helper(["sign_timestamp", str(int(unix_time))])
    if payload and payload.get("signature"):
        return payload, ""
    return None, error or "sign failed"
