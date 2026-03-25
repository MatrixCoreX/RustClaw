#!/usr/bin/env python3
import hashlib
import json
import sys
import time

from cryptoauthlib import (
    ATCADeviceType,
    Status,
    atcab_get_pubkey,
    atcab_init,
    atcab_release,
    atcab_sign,
    cfg_ateccx08a_i2c_default,
    load_cryptoauthlib,
)


def hexs(data):
    return "".join(f"{x:02x}" for x in data)


def build_config():
    load_cryptoauthlib()
    cfg = cfg_ateccx08a_i2c_default()
    cfg.devtype = int(ATCADeviceType.ATECC608)
    cfg.cfg.atcai2c.bus = 0
    cfg.cfg.atcai2c.baud = 100000
    # 与 test123.py 保持一致：7-bit 0x35 对应库里的 8-bit 地址 0x6A
    cfg.cfg.atcai2c.address = 0x6A
    return cfg


def get_pubkey():
    pubkey = bytearray(64)
    status = atcab_get_pubkey(0, pubkey)
    if status != Status.ATCA_SUCCESS:
        raise RuntimeError(f"get_pubkey failed on slot 0: {status}")
    return {"pubkey": hexs(pubkey)}


def sign_timestamp(unix_time=None):
    timestamp = int(unix_time if unix_time is not None else time.time())
    digest = hashlib.sha256(str(timestamp).encode("utf-8")).digest()
    signature = bytearray(64)
    status = atcab_sign(0, digest, signature)
    if status != Status.ATCA_SUCCESS:
        raise RuntimeError(f"sign failed on slot 0: {status}")
    return {
        "timestamp": timestamp,
        "signature": hexs(signature),
    }


def main():
    action = (sys.argv[1] if len(sys.argv) > 1 else "pubkey").strip().lower()
    unix_time = sys.argv[2] if len(sys.argv) > 2 else None
    cfg = build_config()
    initialized = False
    try:
        status = atcab_init(cfg)
        if status != Status.ATCA_SUCCESS:
            raise RuntimeError(f"init failed: {status}")
        initialized = True

        if action == "pubkey":
            payload = get_pubkey()
        elif action == "sign_timestamp":
            payload = sign_timestamp(unix_time)
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
