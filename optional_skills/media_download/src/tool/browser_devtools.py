"""Small standard-library Chrome DevTools Protocol client.

The media downloader only needs a synchronous text WebSocket for a local Chrome
debugging target. Keeping that transport here avoids adding a browser automation
package to the base downloader dependencies.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import socket
import struct
import time
import urllib.parse
from typing import Any


class DevToolsError(RuntimeError):
    """Raised when the local Chrome DevTools connection cannot be used."""


class DevToolsConnection:
    """Synchronous JSON connection to a Chrome DevTools page target."""

    def __init__(self, websocket_url: str, *, timeout: float = 10.0) -> None:
        parsed = urllib.parse.urlsplit(websocket_url)
        if parsed.scheme not in {"ws", "wss"} or not parsed.hostname:
            raise DevToolsError(f"Invalid DevTools WebSocket URL: {websocket_url}")

        port = parsed.port or (443 if parsed.scheme == "wss" else 80)
        raw_socket = socket.create_connection((parsed.hostname, port), timeout=timeout)
        if parsed.scheme == "wss":  # pragma: no cover - local Chrome uses ws://.
            import ssl

            raw_socket = ssl.create_default_context().wrap_socket(raw_socket, server_hostname=parsed.hostname)

        self._socket = raw_socket
        self._buffer = bytearray()
        self._next_id = 0
        self._closed = False
        try:
            self._handshake(parsed, timeout)
        except Exception:
            raw_socket.close()
            raise

    def _handshake(self, parsed: urllib.parse.SplitResult, timeout: float) -> None:
        key = base64.b64encode(os.urandom(16)).decode("ascii")
        path = urllib.parse.urlunsplit(("", "", parsed.path or "/", parsed.query, ""))
        host = parsed.hostname or "127.0.0.1"
        if parsed.port:
            host = f"{host}:{parsed.port}"
        request = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n"
            "Origin: http://127.0.0.1\r\n"
            "\r\n"
        ).encode("ascii")
        self._socket.sendall(request)
        self._socket.settimeout(timeout)

        response = bytearray()
        while b"\r\n\r\n" not in response:
            chunk = self._socket.recv(4096)
            if not chunk:
                raise DevToolsError("Chrome closed the DevTools handshake connection.")
            response.extend(chunk)
            if len(response) > 64 * 1024:
                raise DevToolsError("Chrome returned an oversized DevTools handshake.")

        header_bytes, remainder = response.split(b"\r\n\r\n", 1)
        header_lines = header_bytes.decode("iso-8859-1").split("\r\n")
        if not header_lines or " 101 " not in f" {header_lines[0]} ":
            raise DevToolsError(f"Chrome rejected the DevTools WebSocket: {header_lines[0]}")

        headers: dict[str, str] = {}
        for line in header_lines[1:]:
            if ":" in line:
                name, value = line.split(":", 1)
                headers[name.strip().lower()] = value.strip()
        expected = base64.b64encode(
            hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode("ascii")).digest()
        ).decode("ascii")
        if headers.get("sec-websocket-accept") != expected:
            raise DevToolsError("Chrome returned an invalid DevTools WebSocket handshake.")
        self._buffer.extend(remainder)

    def send(self, method: str, params: dict[str, Any] | None = None) -> int:
        self._next_id += 1
        payload = json.dumps(
            {"id": self._next_id, "method": method, "params": params or {}},
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
        self._send_frame(0x1, payload)
        return self._next_id

    def recv(self, *, timeout: float) -> dict[str, Any] | None:
        deadline = time.monotonic() + max(0.0, timeout)
        fragments = bytearray()
        message_opcode: int | None = None

        while True:
            fin, opcode, payload = self._recv_frame(deadline)
            if opcode == 0x8:
                self._closed = True
                return None
            if opcode == 0x9:
                self._send_frame(0xA, payload)
                continue
            if opcode == 0xA:
                continue
            if opcode in {0x1, 0x2}:
                message_opcode = opcode
                fragments.extend(payload)
            elif opcode == 0x0 and message_opcode is not None:
                fragments.extend(payload)
            else:
                continue

            if not fin:
                continue
            if message_opcode != 0x1:
                fragments.clear()
                message_opcode = None
                continue
            try:
                value = json.loads(fragments.decode("utf-8"))
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise DevToolsError(f"Chrome returned an invalid DevTools message: {exc}") from exc
            if not isinstance(value, dict):
                raise DevToolsError("Chrome returned a non-object DevTools message.")
            return value

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        if self._closed:
            raise DevToolsError("The DevTools WebSocket is closed.")
        first = 0x80 | opcode
        size = len(payload)
        if size < 126:
            header = bytes((first, 0x80 | size))
        elif size <= 0xFFFF:
            header = bytes((first, 0x80 | 126)) + struct.pack("!H", size)
        else:
            header = bytes((first, 0x80 | 127)) + struct.pack("!Q", size)
        mask = os.urandom(4)
        masked = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
        self._socket.sendall(header + mask + masked)

    def _recv_frame(self, deadline: float) -> tuple[bool, int, bytes]:
        first, second = self._recv_exact(2, deadline)
        fin = bool(first & 0x80)
        opcode = first & 0x0F
        masked = bool(second & 0x80)
        size = second & 0x7F
        if size == 126:
            size = struct.unpack("!H", self._recv_exact(2, deadline))[0]
        elif size == 127:
            size = struct.unpack("!Q", self._recv_exact(8, deadline))[0]
        mask = self._recv_exact(4, deadline) if masked else b""
        payload = self._recv_exact(size, deadline)
        if masked:
            payload = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
        return fin, opcode, payload

    def _recv_exact(self, size: int, deadline: float) -> bytes:
        while len(self._buffer) < size:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("Timed out waiting for a DevTools message.")
            self._socket.settimeout(remaining)
            try:
                chunk = self._socket.recv(max(4096, size - len(self._buffer)))
            except socket.timeout as exc:
                raise TimeoutError("Timed out waiting for a DevTools message.") from exc
            if not chunk:
                self._closed = True
                raise DevToolsError("Chrome closed the DevTools WebSocket.")
            self._buffer.extend(chunk)
        result = bytes(self._buffer[:size])
        del self._buffer[:size]
        return result

    def close(self) -> None:
        if not self._closed:
            try:
                self._send_frame(0x8, struct.pack("!H", 1000))
            except (DevToolsError, OSError):
                pass
        self._closed = True
        try:
            self._socket.close()
        except OSError:
            pass

    def __enter__(self) -> DevToolsConnection:
        return self

    def __exit__(self, *_args: object) -> None:
        self.close()
