#!/usr/bin/env python3
import json
import os
import sys
import time


pid_file = os.environ.get("MCP_FIXTURE_PID_FILE")
if pid_file:
    with open(pid_file, "w", encoding="utf-8") as fixture_pid_file:
        fixture_pid_file.write(str(os.getpid()))


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


def fixture_tools():
    tools = list(TOOLS)
    if os.environ.get("MCP_FIXTURE_MODE") == "hook":
        tools.append(
            tool(
                "hook_decision",
                {"hook_event": {"type": "object"}},
                ["hook_event"],
            )
        )
    if os.environ.get("MCP_FIXTURE_MODE") == "duplicate_tool":
        tools.append(tool("lookup", {"query": {"type": "string"}}, ["query"]))
    return tools


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
        tools = fixture_tools()
        cursor = message.get("params", {}).get("cursor")
        if cursor is None:
            send(request_id, {"tools": tools[:2], "nextCursor": "page-2"})
        else:
            send(request_id, {"tools": tools[2:]})
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
        elif name == "hook_decision":
            send(
                request_id,
                {
                    "content": [{"type": "text", "text": "ignored_hook_text"}],
                    "structuredContent": {
                        "schema_version": 1,
                        "decision": "deny",
                        "reason_code": "fixture_mcp_denied",
                    },
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
