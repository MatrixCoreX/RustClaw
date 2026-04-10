#!/usr/bin/env python3
import argparse
import hashlib
import json

from cryptography import x509
from cryptography.exceptions import InvalidSignature
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.asymmetric import ec, utils


def _clean_hex(value):
    return "".join(str(value).strip().split()).lower()


def _hex_to_bytes(name, value):
    try:
        return bytes.fromhex(_clean_hex(value))
    except ValueError as exc:
        raise ValueError(f"{name} is not valid hex") from exc


def _load_cert_from_hex(name, value):
    return x509.load_der_x509_certificate(_hex_to_bytes(name, value))


def _verify_cert_signature(child_cert, issuer_cert):
    issuer_public_key = issuer_cert.public_key()
    issuer_public_key.verify(
        child_cert.signature,
        child_cert.tbs_certificate_bytes,
        ec.ECDSA(child_cert.signature_hash_algorithm),
    )


def _raw_rs_to_der(signature_hex):
    signature = _hex_to_bytes("signature_hex", signature_hex)
    if len(signature) != 64:
        raise ValueError("signature_hex must be 64 bytes raw R||S")
    r = int.from_bytes(signature[:32], "big")
    s = int.from_bytes(signature[32:], "big")
    return utils.encode_dss_signature(r, s)


def _device_public_key_hex(device_cert):
    public_key = device_cert.public_key()
    numbers = public_key.public_numbers()
    return f"{numbers.x:064x}{numbers.y:064x}"


def verify(device_cert_hex, signer_cert_hex, root_cert_hex, timestamp, signature_hex):
    root_cert = _load_cert_from_hex("root_cert_hex", root_cert_hex)
    signer_cert = _load_cert_from_hex("signer_cert_hex", signer_cert_hex)
    device_cert = _load_cert_from_hex("device_cert_hex", device_cert_hex)

    _verify_cert_signature(signer_cert, root_cert)
    _verify_cert_signature(device_cert, signer_cert)

    digest = hashlib.sha256(str(int(timestamp)).encode("utf-8")).digest()
    der_signature = _raw_rs_to_der(signature_hex)
    device_cert.public_key().verify(
        der_signature,
        digest,
        ec.ECDSA(utils.Prehashed(hashes.SHA256())),
    )

    return {
        "chain_verified": True,
        "signature_verified": True,
        "device_public_key_hex": _device_public_key_hex(device_cert),
        "device_subject": device_cert.subject.rfc4514_string(),
        "device_serial_number": format(device_cert.serial_number, "x"),
        "signer_subject": signer_cert.subject.rfc4514_string(),
        "root_subject": root_cert.subject.rfc4514_string(),
    }


def main():
    parser = argparse.ArgumentParser(description="Verify TNG certificate chain and device signature.")
    parser.add_argument("--device-cert-hex", required=True)
    parser.add_argument("--signer-cert-hex", required=True)
    parser.add_argument("--root-cert-hex", required=True)
    parser.add_argument("--timestamp", required=True, type=int)
    parser.add_argument("--signature-hex", required=True)
    args = parser.parse_args()

    try:
        result = verify(
            device_cert_hex=args.device_cert_hex,
            signer_cert_hex=args.signer_cert_hex,
            root_cert_hex=args.root_cert_hex,
            timestamp=args.timestamp,
            signature_hex=args.signature_hex,
        )
        print(json.dumps({"ok": True, **result}, ensure_ascii=False))
        return 0
    except InvalidSignature:
        print(json.dumps({"ok": False, "error": "certificate chain or signature verification failed"}, ensure_ascii=False))
        return 1
    except Exception as exc:
        print(json.dumps({"ok": False, "error": str(exc)}, ensure_ascii=False))
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
