#!/usr/bin/env python3
import hashlib
import json
import os
import sys
import time
from ctypes import POINTER, c_int, c_size_t, c_uint8, cast, cdll

_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))


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
    candidates = _existing_dirs(
        [
            os.environ.get("RUSTCLAW_CRYPTOAUTHLIB_ROOT"),
            os.path.join(_SCRIPT_DIR, "vendor", "cryptoauthlib"),
            os.path.join(_SCRIPT_DIR, "..", "..", "cryptoauthlib"),
            "/home/pi/cryptoauthlib",
        ]
    )
    return candidates[0] if candidates else None


def _bootstrap_python_package(root_dir):
    if not root_dir:
        return
    package_parent = os.path.join(root_dir, "python")
    if os.path.isdir(package_parent) and package_parent not in sys.path:
        sys.path.insert(0, package_parent)


def _cryptoauthlib_lib_dirs(root_dir):
    if not root_dir:
        return []
    return _existing_dirs(
        [
            os.path.join(root_dir, "build-pyfix"),
            os.path.join(root_dir, "build-pyfix", "lib"),
            os.path.join(root_dir, "build", "lib"),
            os.path.join(root_dir, "python", "cryptoauthlib"),
        ]
    )


def _bootstrap_runtime_env(root_dir):
    lib_dirs = _cryptoauthlib_lib_dirs(root_dir)
    if not lib_dirs:
        return
    current = os.environ.get("LD_LIBRARY_PATH", "")
    os.environ["LD_LIBRARY_PATH"] = ":".join(lib_dirs + ([current] if current else []))


def _maybe_reexec_with_runtime_env(root_dir):
    if __name__ != "__main__" or not root_dir:
        return
    if os.environ.get("RUSTCLAW_CRYPTOAUTHLIB_BOOTSTRAPPED") == "1":
        return
    lib_dirs = _cryptoauthlib_lib_dirs(root_dir)
    if not lib_dirs:
        return
    env = os.environ.copy()
    current = env.get("LD_LIBRARY_PATH", "")
    env["LD_LIBRARY_PATH"] = ":".join(lib_dirs + ([current] if current else []))
    env["RUSTCLAW_CRYPTOAUTHLIB_BOOTSTRAPPED"] = "1"
    os.execvpe(sys.executable, [sys.executable, os.path.abspath(__file__), *sys.argv[1:]], env)


_CRYPTOAUTHLIB_ROOT = _cryptoauthlib_root()
_maybe_reexec_with_runtime_env(_CRYPTOAUTHLIB_ROOT)
_bootstrap_python_package(_CRYPTOAUTHLIB_ROOT)
_bootstrap_runtime_env(_CRYPTOAUTHLIB_ROOT)

from cryptoauthlib import (
    ATCADeviceType,
    ATCAIfaceCfg,
    ATCAIfaceType,
    Status,
    atcab_get_pubkey,
    atcab_init,
    atcab_release,
    atcab_sign,
    get_cryptoauthlib,
    load_cryptoauthlib,
)


def hexs(data):
    return "".join(f"{x:02x}" for x in data)


def _int_env(name, default):
    raw = os.environ.get(name)
    if raw is None or str(raw).strip() == "":
        return default
    return int(str(raw).strip(), 0)


def _library_candidates(root_dir):
    if not root_dir:
        return []
    return _existing_files(
        [
            os.environ.get("RUSTCLAW_CRYPTOAUTHLIB_LIB_PATH"),
            os.path.join(root_dir, "python", "cryptoauthlib", "libcryptoauth.so"),
            os.path.join(root_dir, "build-pyfix", "libcryptoauth.so"),
            os.path.join(root_dir, "build-pyfix", "lib", "libcryptoauth.so"),
            os.path.join(root_dir, "build-pyfix", "lib", "libcryptoauth.so.3"),
            os.path.join(root_dir, "build", "lib", "libcryptoauth.so"),
            os.path.join(root_dir, "build", "lib", "libcryptoauth.so.3"),
        ]
    )


def _load_library():
    candidates = _library_candidates(_CRYPTOAUTHLIB_ROOT)
    if candidates:
        load_cryptoauthlib(cdll.LoadLibrary(candidates[0]))
        _configure_tng_ctypes()
        return candidates[0]
    load_cryptoauthlib()
    _configure_tng_ctypes()
    return None


def _configure_tng_ctypes():
    lib = get_cryptoauthlib()
    if lib is None:
        return
    lib.tng_get_device_pubkey.restype = c_int
    lib.tng_get_device_pubkey.argtypes = [POINTER(c_uint8)]
    lib.tng_atcacert_max_device_cert_size.restype = c_int
    lib.tng_atcacert_max_device_cert_size.argtypes = [POINTER(c_size_t)]
    lib.tng_atcacert_read_device_cert.restype = c_int
    lib.tng_atcacert_read_device_cert.argtypes = [POINTER(c_uint8), POINTER(c_size_t), POINTER(c_uint8)]
    lib.tng_atcacert_max_signer_cert_size.restype = c_int
    lib.tng_atcacert_max_signer_cert_size.argtypes = [POINTER(c_size_t)]
    lib.tng_atcacert_read_signer_cert.restype = c_int
    lib.tng_atcacert_read_signer_cert.argtypes = [POINTER(c_uint8), POINTER(c_size_t)]
    lib.tng_atcacert_root_cert_size.restype = c_int
    lib.tng_atcacert_root_cert_size.argtypes = [POINTER(c_size_t)]
    lib.tng_atcacert_root_cert.restype = c_int
    lib.tng_atcacert_root_cert.argtypes = [POINTER(c_uint8), POINTER(c_size_t)]


def build_config():
    lib_path = _load_library()
    cfg = ATCAIfaceCfg()
    cfg.iface_type = int(ATCAIfaceType.ATCA_I2C_IFACE)
    cfg.devtype = _int_env("RUSTCLAW_CRYPTOAUTHLIB_DEVTYPE", int(ATCADeviceType.ATECC608))
    cfg.atcai2c.bus = _int_env("RUSTCLAW_CRYPTOAUTHLIB_I2C_BUS", 0)
    cfg.atcai2c.baud = _int_env("RUSTCLAW_CRYPTOAUTHLIB_I2C_BAUD", 100000)
    # Linux HAL 会右移 1 位后再发给内核，因此这里沿用 8-bit 地址。
    cfg.atcai2c.address = _int_env("RUSTCLAW_CRYPTOAUTHLIB_I2C_ADDRESS", 0x6A)
    cfg.wake_delay = _int_env("RUSTCLAW_CRYPTOAUTHLIB_WAKE_DELAY", 1500)
    cfg.rx_retries = _int_env("RUSTCLAW_CRYPTOAUTHLIB_RX_RETRIES", 20)
    cfg.cfg_data = None
    return cfg, lib_path


def _slot():
    return _int_env("RUSTCLAW_CRYPTOAUTHLIB_SLOT", 0)


def _config_meta(cfg, lib_path):
    return {
        "slot": _slot(),
        "i2c_bus": int(cfg.atcai2c.bus),
        "i2c_baud": int(cfg.atcai2c.baud),
        "i2c_address": f"0x{int(cfg.atcai2c.address):02x}",
        "lib_path": lib_path or "",
    }


def _read_tng_cert(max_size_func, read_func, field_name):
    max_cert_size = c_size_t(0)
    status = max_size_func(max_cert_size)
    if status != int(Status.ATCA_SUCCESS):
        raise RuntimeError(f"{field_name} size failed: {status}")
    cert = (c_uint8 * int(max_cert_size.value))()
    cert_size = c_size_t(len(cert))
    status = read_func(cast(cert, POINTER(c_uint8)), cert_size)
    if status != int(Status.ATCA_SUCCESS):
        raise RuntimeError(f"{field_name} read failed: {status}")
    cert_bytes = bytes(cert)[: int(cert_size.value)]
    return {
        field_name: cert_bytes.hex(),
        f"{field_name}_size": len(cert_bytes),
    }


def _read_tng_signer_cert_bytes():
    signer_cert = _read_tng_cert(
        get_cryptoauthlib().tng_atcacert_max_signer_cert_size,
        get_cryptoauthlib().tng_atcacert_read_signer_cert,
        "signer_cert_hex",
    )
    return bytes.fromhex(signer_cert["signer_cert_hex"])


def get_pubkey(cfg, lib_path):
    pubkey = bytearray(64)
    status = atcab_get_pubkey(_slot(), pubkey)
    if status != Status.ATCA_SUCCESS:
        raise RuntimeError(f"get_pubkey failed on slot {_slot()}: {status}")
    return {"pubkey": hexs(pubkey), **_config_meta(cfg, lib_path)}


def get_tng_device_pubkey(cfg, lib_path):
    pubkey = (c_uint8 * 64)()
    status = get_cryptoauthlib().tng_get_device_pubkey(cast(pubkey, POINTER(c_uint8)))
    if status != int(Status.ATCA_SUCCESS):
        raise RuntimeError(f"tng device pubkey failed: {status}")
    return {"pubkey": hexs(bytes(pubkey)), **_config_meta(cfg, lib_path)}


def get_tng_device_cert(cfg, lib_path):
    signer_cert = _read_tng_signer_cert_bytes()
    signer_cert_buffer = (c_uint8 * len(signer_cert)).from_buffer_copy(signer_cert)
    return {
        **_read_tng_cert(
            get_cryptoauthlib().tng_atcacert_max_device_cert_size,
            lambda cert, cert_size: get_cryptoauthlib().tng_atcacert_read_device_cert(
                cert, cert_size, cast(signer_cert_buffer, POINTER(c_uint8))
            ),
            "device_cert_hex",
        ),
        **_config_meta(cfg, lib_path),
    }


def get_tng_signer_cert(cfg, lib_path):
    return {
        **_read_tng_cert(
            get_cryptoauthlib().tng_atcacert_max_signer_cert_size,
            get_cryptoauthlib().tng_atcacert_read_signer_cert,
            "signer_cert_hex",
        ),
        **_config_meta(cfg, lib_path),
    }


def get_tng_root_cert(cfg, lib_path):
    cert_size = c_size_t(0)
    status = get_cryptoauthlib().tng_atcacert_root_cert_size(cert_size)
    if status != int(Status.ATCA_SUCCESS):
        raise RuntimeError(f"root_cert size failed: {status}")
    cert = (c_uint8 * int(cert_size.value))()
    read_size = c_size_t(len(cert))
    status = get_cryptoauthlib().tng_atcacert_root_cert(cast(cert, POINTER(c_uint8)), read_size)
    if status != int(Status.ATCA_SUCCESS):
        raise RuntimeError(f"root_cert read failed: {status}")
    cert_bytes = bytes(cert)[: int(read_size.value)]
    return {
        "root_cert_hex": cert_bytes.hex(),
        "root_cert_hex_size": len(cert_bytes),
        **_config_meta(cfg, lib_path),
    }


def sign_timestamp(cfg, lib_path, unix_time=None):
    timestamp = int(unix_time if unix_time is not None else time.time())
    digest = hashlib.sha256(str(timestamp).encode("utf-8")).digest()
    signature = bytearray(64)
    status = atcab_sign(_slot(), digest, signature)
    if status != Status.ATCA_SUCCESS:
        raise RuntimeError(f"sign failed on slot {_slot()}: {status}")
    return {
        "timestamp": timestamp,
        "signature": hexs(signature),
        **_config_meta(cfg, lib_path),
    }


def main():
    action = (sys.argv[1] if len(sys.argv) > 1 else "pubkey").strip().lower()
    unix_time = sys.argv[2] if len(sys.argv) > 2 else None
    cfg, lib_path = build_config()
    initialized = False
    try:
        status = atcab_init(cfg)
        if status != Status.ATCA_SUCCESS:
            raise RuntimeError(
                "init failed: "
                f"{status} "
                f"(bus={int(cfg.atcai2c.bus)}, address=0x{int(cfg.atcai2c.address):02x}, "
                f"baud={int(cfg.atcai2c.baud)}, slot={_slot()}, lib={lib_path or 'auto'})"
            )
        initialized = True

        if action == "pubkey":
            payload = get_pubkey(cfg, lib_path)
        elif action == "tng_device_pubkey":
            payload = get_tng_device_pubkey(cfg, lib_path)
        elif action == "tng_device_cert":
            payload = get_tng_device_cert(cfg, lib_path)
        elif action == "tng_signer_cert":
            payload = get_tng_signer_cert(cfg, lib_path)
        elif action == "tng_root_cert":
            payload = get_tng_root_cert(cfg, lib_path)
        elif action == "sign_timestamp":
            payload = sign_timestamp(cfg, lib_path, unix_time)
        else:
            raise RuntimeError(f"unsupported action: {action}")

        print(json.dumps({"ok": True, "action": action, **payload}))
        return 0
    except Exception as exc:
        print(json.dumps({"ok": False, "error": str(exc)}))
        return 1
    finally:
        if initialized:
            try:
                atcab_release()
            except Exception:
                pass


if __name__ == "__main__":
    sys.exit(main())
