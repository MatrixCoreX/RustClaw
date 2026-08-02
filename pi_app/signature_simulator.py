#!/usr/bin/env python3
"""Software-only ATECC608 protocol simulator for local host testing."""

import hashlib
import hmac
import json
import os
import secrets
import time

try:
    from signature_simulator_x509 import build_certificate
except ImportError:
    from .signature_simulator_x509 import build_certificate


P = 0xFFFFFFFF00000001000000000000000000000000FFFFFFFFFFFFFFFFFFFFFFFF
A = P - 3
B = 0x5AC635D8AA3A93E7B3EBBD55769886BC651D06B0CC53B0F63BCE3C3E27D2604B
GX = 0x6B17D1F2E12C4247F8BCE6E563A440F277037D812DEB33A0F4A13945D898C296
GY = 0x4FE342E2FE1A7F9B8EE7EB4A7C0F9E162BCE33576B315ECECBB6406837BF51F5
N = 0xFFFFFFFF00000000FFFFFFFFFFFFFFFFBCE6FAADA7179E84F3B9CAC2FC632551
G = (GX, GY)
SCHEMA_VERSION = 1


class SignatureSimulationError(RuntimeError):
    def __init__(self, message, error_code="signature_simulator_error"):
        super().__init__(message)
        self.error_code = error_code


def simulation_state_path():
    configured = os.environ.get("APP_SIGNATURE_SIMULATOR_STATE", "").strip()
    if configured:
        return os.path.abspath(configured)
    return os.path.abspath(
        os.path.join(os.path.dirname(__file__), "..", "data", "nni", "signature-simulator.json")
    )


def _inverse(value, modulus):
    return pow(value % modulus, -1, modulus)


def _point_add(left, right):
    if left is None:
        return right
    if right is None:
        return left
    x1, y1 = left
    x2, y2 = right
    if x1 == x2 and (y1 + y2) % P == 0:
        return None
    if left == right:
        slope = ((3 * x1 * x1 + A) * _inverse(2 * y1, P)) % P
    else:
        slope = ((y2 - y1) * _inverse(x2 - x1, P)) % P
    x3 = (slope * slope - x1 - x2) % P
    return x3, (slope * (x1 - x3) - y1) % P


def _scalar_multiply(scalar, point=G):
    result = None
    addend = point
    value = scalar % N
    while value:
        if value & 1:
            result = _point_add(result, addend)
        addend = _point_add(addend, addend)
        value >>= 1
    return result


def _public_key_bytes(private_key):
    point = _scalar_multiply(private_key)
    if point is None:
        raise SignatureSimulationError("simulated private key produced no public key")
    return point[0].to_bytes(32, "big") + point[1].to_bytes(32, "big")


def _deterministic_nonce(private_key, digest):
    secret = private_key.to_bytes(32, "big")
    normalized_digest = (int.from_bytes(digest, "big") % N).to_bytes(32, "big")
    value = b"\x01" * 32
    key = b"\x00" * 32
    key = hmac.new(key, value + b"\x00" + secret + normalized_digest, hashlib.sha256).digest()
    value = hmac.new(key, value, hashlib.sha256).digest()
    key = hmac.new(key, value + b"\x01" + secret + normalized_digest, hashlib.sha256).digest()
    value = hmac.new(key, value, hashlib.sha256).digest()
    while True:
        value = hmac.new(key, value, hashlib.sha256).digest()
        candidate = int.from_bytes(value, "big")
        if 1 <= candidate < N:
            return candidate
        key = hmac.new(key, value + b"\x00", hashlib.sha256).digest()
        value = hmac.new(key, value, hashlib.sha256).digest()


def _sign_digest(private_key, digest):
    nonce = _deterministic_nonce(private_key, digest)
    point = _scalar_multiply(nonce)
    if point is None:
        raise SignatureSimulationError("simulated signature nonce produced no point")
    r = point[0] % N
    s = (_inverse(nonce, N) * (int.from_bytes(digest, "big") + r * private_key)) % N
    if not r or not s:
        raise SignatureSimulationError("simulated signature produced an invalid scalar")
    if s > N // 2:
        s = N - s
    return r.to_bytes(32, "big") + s.to_bytes(32, "big")


def verify_raw_signature(public_key_hex, digest, signature_hex):
    try:
        public_key = bytes.fromhex(public_key_hex)
        signature = bytes.fromhex(signature_hex)
        if len(public_key) != 64 or len(signature) != 64:
            return False
        point = (int.from_bytes(public_key[:32], "big"), int.from_bytes(public_key[32:], "big"))
        if (point[1] * point[1] - (point[0] ** 3 + A * point[0] + B)) % P:
            return False
        r = int.from_bytes(signature[:32], "big")
        s = int.from_bytes(signature[32:], "big")
        if not (1 <= r < N and 1 <= s < N):
            return False
        inverse = _inverse(s, N)
        result = _point_add(
            _scalar_multiply((int.from_bytes(digest, "big") * inverse) % N),
            _scalar_multiply((r * inverse) % N, point),
        )
        return result is not None and result[0] % N == r
    except (TypeError, ValueError, ZeroDivisionError):
        return False


def _new_private_key():
    return secrets.randbelow(N - 1) + 1


def _load_state(path=None):
    target = path or simulation_state_path()
    try:
        with open(target, "r", encoding="utf-8") as handle:
            state = json.load(handle)
        if state.get("schema_version") != SCHEMA_VERSION:
            raise ValueError("unsupported schema version")
        for field in ("device_private_key", "signer_private_key", "root_private_key"):
            value = int(state[field], 16)
            if not 1 <= value < N:
                raise ValueError(f"invalid {field}")
        if "enabled" in state and not isinstance(state["enabled"], bool):
            raise ValueError("invalid enabled flag")
        return state
    except FileNotFoundError:
        return None
    except (KeyError, TypeError, ValueError, json.JSONDecodeError) as exc:
        raise SignatureSimulationError(
            f"simulated chip state is invalid: {exc}", "signature_simulator_state_invalid"
        ) from exc


def simulation_enabled():
    state = _load_state()
    return state is not None and state.get("enabled", True)


def _write_state(path, state):
    directory = os.path.dirname(path)
    os.makedirs(directory, mode=0o700, exist_ok=True)
    temporary = f"{path}.{os.getpid()}.{secrets.token_hex(4)}.tmp"
    try:
        descriptor = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w", encoding="utf-8") as handle:
            json.dump(state, handle, separators=(",", ":"))
            handle.write("\n")
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
        if os.name == "posix":
            os.chmod(path, 0o600)
    finally:
        try:
            os.remove(temporary)
        except FileNotFoundError:
            pass


def _state_private_key(state, field):
    return int(state[field], 16)


def _metadata():
    return {
        "slot": 0,
        "i2c_bus": None,
        "i2c_baud": None,
        "i2c_address": "virtual",
        "lib_path": "",
        "simulated": True,
        "device_kind": "simulated",
    }


def enable_simulation():
    path = simulation_state_path()
    state = _load_state(path)
    if state is None:
        state = {
            "schema_version": SCHEMA_VERSION,
            "created_at": int(time.time()),
            "enabled": True,
            "device_private_key": f"{_new_private_key():064x}",
            "signer_private_key": f"{_new_private_key():064x}",
            "root_private_key": f"{_new_private_key():064x}",
        }
    else:
        state["enabled"] = True
    _write_state(path, state)
    pubkey = _public_key_bytes(_state_private_key(state, "device_private_key")).hex()
    return {
        "signature_chip_present": True,
        "simulation_enabled": True,
        "pubkey": pubkey,
        **_metadata(),
    }


def disable_simulation():
    path = simulation_state_path()
    state = _load_state(path)
    if state is not None:
        state["enabled"] = False
        _write_state(path, state)
    return {
        "signature_chip_present": False,
        "simulation_enabled": False,
        "simulated": False,
        "device_kind": "unavailable",
    }


def run_simulated_action(action, action_arg=None):
    state = _load_state()
    if state is None or not state.get("enabled", True):
        raise SignatureSimulationError("simulated signature chip is not enabled", "signature_simulator_disabled")
    device_key = _state_private_key(state, "device_private_key")
    signer_key = _state_private_key(state, "signer_private_key")
    root_key = _state_private_key(state, "root_private_key")
    metadata = _metadata()
    pubkey = _public_key_bytes(device_key).hex()
    if action in ("pubkey", "tng_device_pubkey"):
        return {"pubkey": pubkey, **metadata}
    if action == "sign_timestamp":
        try:
            timestamp = int(action_arg if action_arg is not None else time.time())
        except (TypeError, ValueError) as exc:
            raise SignatureSimulationError("timestamp must be an integer", "timestamp_invalid") from exc
        digest = hashlib.sha256(str(timestamp).encode("utf-8")).digest()
        return {"timestamp": timestamp, "signature": _sign_digest(device_key, digest).hex(), **metadata}
    if action == "sign_challenge":
        challenge = str(action_arg or "").strip()
        if not challenge:
            raise SignatureSimulationError("challenge required", "challenge_required")
        digest = hashlib.sha256(challenge.encode("utf-8")).digest()
        return {"challenge": challenge, "signature": _sign_digest(device_key, digest).hex(), **metadata}
    root_name = "Agent Runtime Simulated Root"
    signer_name = "Agent Runtime Simulated Signer"
    if action == "tng_device_cert":
        cert = build_certificate(
            _public_key_bytes(device_key),
            "Agent Runtime Simulated Device",
            signer_name,
            lambda digest: _sign_digest(signer_key, digest),
            False,
        )
        return {"device_cert_hex": cert.hex(), "device_cert_hex_size": len(cert), **metadata}
    if action == "tng_signer_cert":
        cert = build_certificate(
            _public_key_bytes(signer_key),
            signer_name,
            root_name,
            lambda digest: _sign_digest(root_key, digest),
            True,
        )
        return {"signer_cert_hex": cert.hex(), "signer_cert_hex_size": len(cert), **metadata}
    if action == "tng_root_cert":
        cert = build_certificate(
            _public_key_bytes(root_key),
            root_name,
            root_name,
            lambda digest: _sign_digest(root_key, digest),
            True,
        )
        return {"root_cert_hex": cert.hex(), "root_cert_hex_size": len(cert), **metadata}
    raise SignatureSimulationError(f"unsupported action: {action}", "unsupported_action")
