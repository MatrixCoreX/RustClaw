#!/usr/bin/env python3
"""Validate a native Release header without executing untrusted binaries."""

from pathlib import Path
import struct
import sys


TARGETS = {
    "x86_64-unknown-linux-gnu": ("elf", 62),
    "aarch64-unknown-linux-gnu": ("elf", 183),
    "x86_64-apple-darwin": ("mach", 0x01000007),
    "aarch64-apple-darwin": ("mach", 0x0100000C),
}


def verify_binary(path: Path, target: str) -> None:
    if target not in TARGETS:
        raise ValueError("unsupported_release_target")
    with path.open("rb") as stream:
        header = stream.read(32)
    kind, expected = TARGETS[target]
    if kind == "elf":
        if len(header) < 20 or header[:4] != b"\x7fELF" or header[4:6] != b"\x02\x01":
            raise ValueError("release_elf64_header_invalid")
        actual = int.from_bytes(header[18:20], "little")
    else:
        if len(header) < 32 or header[:4] != b"\xcf\xfa\xed\xfe":
            raise ValueError("release_macho64_header_invalid")
        actual = struct.unpack_from("<I", header, 4)[0]
    if actual != expected:
        raise ValueError("release_binary_architecture_mismatch")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: verify_release_binary.py BINARY RUST_TARGET")
    try:
        verify_binary(Path(sys.argv[1]), sys.argv[2])
    except (OSError, ValueError) as error:
        raise SystemExit(str(error)) from error
