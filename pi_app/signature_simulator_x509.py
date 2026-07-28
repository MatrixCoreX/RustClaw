#!/usr/bin/env python3
"""Minimal DER/X.509 encoder used by the local signature-chip simulator."""

import hashlib


def _der_length(length):
    if length < 0x80:
        return bytes([length])
    encoded = length.to_bytes((length.bit_length() + 7) // 8, "big")
    return bytes([0x80 | len(encoded)]) + encoded


def _der(tag, content=b""):
    return bytes([tag]) + _der_length(len(content)) + content


def _sequence(*items):
    return _der(0x30, b"".join(items))


def _integer(value):
    encoded = value.to_bytes(max(1, (value.bit_length() + 7) // 8), "big")
    if encoded[0] & 0x80:
        encoded = b"\x00" + encoded
    return _der(0x02, encoded)


def _oid(value):
    parts = [int(part) for part in value.split(".")]
    encoded = bytearray([parts[0] * 40 + parts[1]])
    for part in parts[2:]:
        segment = [part & 0x7F]
        part >>= 7
        while part:
            segment.append(0x80 | (part & 0x7F))
            part >>= 7
        encoded.extend(reversed(segment))
    return _der(0x06, bytes(encoded))


def _name(common_name):
    attribute = _sequence(_oid("2.5.4.3"), _der(0x0C, common_name.encode("utf-8")))
    return _sequence(_der(0x31, attribute))


SIGNATURE_ALGORITHM = _sequence(_oid("1.2.840.10045.4.3.2"))


def _subject_public_key_info(public_key):
    algorithm = _sequence(_oid("1.2.840.10045.2.1"), _oid("1.2.840.10045.3.1.7"))
    return _sequence(algorithm, _der(0x03, b"\x00\x04" + public_key))


def _ecdsa_der_signature(raw_signature):
    return _sequence(
        _integer(int.from_bytes(raw_signature[:32], "big")),
        _integer(int.from_bytes(raw_signature[32:], "big")),
    )


def build_certificate(public_key, subject_name, issuer_name, sign_digest, is_ca):
    """Build a compact P-256 certificate and sign its TBS digest via callback."""
    serial = int.from_bytes(hashlib.sha256(public_key + subject_name.encode()).digest()[:16], "big") or 1
    constraints = _sequence(_der(0x01, b"\xff")) if is_ca else _sequence()
    extension = _sequence(
        _oid("2.5.29.19"),
        _der(0x01, b"\xff"),
        _der(0x04, constraints),
    )
    tbs = _sequence(
        _der(0xA0, _integer(2)),
        _integer(serial),
        SIGNATURE_ALGORITHM,
        _name(issuer_name),
        _sequence(_der(0x17, b"250101000000Z"), _der(0x17, b"450101000000Z")),
        _name(subject_name),
        _subject_public_key_info(public_key),
        _der(0xA3, _sequence(extension)),
    )
    signature = _ecdsa_der_signature(sign_digest(hashlib.sha256(tbs).digest()))
    return _sequence(tbs, SIGNATURE_ALGORITHM, _der(0x03, b"\x00" + signature))
