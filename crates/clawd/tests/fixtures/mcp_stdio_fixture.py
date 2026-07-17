#!/usr/bin/env python3
import json
import os
import sys
import time


def send(request_id, result):
    sys.stdout.write(
        json.dumps(
            {"jsonrpc": "2.0", "id": request_id, "result": result},
            separators=(",", ":"),
        )
        + "\n"
    )
    sys.stdout.flush()


def tool(name, properties=None, required=None):
    schema = {
        "type": "object",
        "properties": properties or {},
        "required": required or [],
    }
    if os.environ.get("MCP_FIXTURE_MODE") == "invalid_schema" and name == "lookup":
        schema = {"type": "array"}
    return {
        "name": name,
        "description": f"fixture_{name}",
        "inputSchema": schema,
    }


TOOLS = [
    tool("lookup", {"query": {"type": "string"}}, ["query"]),
    tool("fail"),
    tool("slow"),
    tool("large"),
]


for line in sys.stdin:
    message = json.loads(line)
    method = message.get("method")
    request_id = message.get("id")
    if request_id is None:
        continue
    if method == "initialize":
        protocol_version = message.get("params", {}).get(
            "protocolVersion", "2025-11-25"
        )
        send(
            request_id,
            {
                "protocolVersion": protocol_version,
                "capabilities": {"tools": {"listChanged": False}},
                "serverInfo": {"name": "rustclaw-fixture", "version": "1"},
            },
        )
    elif method == "tools/list":
        cursor = message.get("params", {}).get("cursor")
        if cursor is None:
            send(request_id, {"tools": TOOLS[:2], "nextCursor": "page-2"})
        else:
            send(request_id, {"tools": TOOLS[2:]})
            marker = os.environ.get("MCP_FIXTURE_EXIT_ONCE_MARKER")
            if marker and not os.path.exists(marker):
                with open(marker, "w", encoding="utf-8") as marker_file:
                    marker_file.write("exited\n")
                raise SystemExit(0)
    elif method == "tools/call":
        params = message.get("params", {})
        name = params.get("name")
        arguments = params.get("arguments", {})
        if name == "slow":
            time.sleep(2)
        if name == "fail":
            send(
                request_id,
                {
                    "content": [{"type": "text", "text": "fixture_error_text"}],
                    "structuredContent": {"error_code": "fixture_failure"},
                    "isError": True,
                },
            )
        elif name == "large":
            send(
                request_id,
                {
                    "content": [{"type": "text", "text": "x" * 2048}],
                    "structuredContent": {"size": 2048},
                    "isError": False,
                },
            )
        else:
            send(
                request_id,
                {
                    "content": [{"type": "text", "text": "fixture_text"}],
                    "structuredContent": {
                        "query": arguments.get("query"),
                        "source": "fixture",
                    },
                    "isError": False,
                },
            )
    elif method == "ping":
        send(request_id, {})
    else:
        sys.stdout.write(
            json.dumps(
                {
                    "jsonrpc": "2.0",
                    "id": request_id,
                    "error": {"code": -32601, "message": "method_not_found"},
                },
                separators=(",", ":"),
            )
            + "\n"
        )
        sys.stdout.flush()
