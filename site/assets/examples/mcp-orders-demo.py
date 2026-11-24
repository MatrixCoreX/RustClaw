#!/usr/bin/env python3
"""Local MCP demo with two read-only tools and fixed sample orders."""

import json
import re
import sys


PROTOCOL_VERSIONS = ("2025-11-25", "2025-06-18", "2025-03-26", "2024-11-05")
ORDERS = {
    3: {
        "order_id": 3,
        "status": "in_transit",
        "location": "Hangzhou sorting center",
        "carrier": "Demo Express",
        "estimated_delivery": "2026-11-25",
        "tracking_number": "7898047967",
    },
    12: {
        "order_id": 12,
        "status": "delivered",
        "location": "Shanghai",
        "carrier": "Demo Express",
        "tracking_number": "0425386197",
    },
}
TRACKING_NUMBERS = {order["tracking_number"]: order for order in ORDERS.values()}


def describe_tool(name, description, field, schema):
    return {
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": {field: schema},
            "required": [field],
            "additionalProperties": False,
        },
        "annotations": {"readOnlyHint": True},
    }


TOOLS = [
    describe_tool(
        "lookup_order",
        "Look up a sample order by its numeric order number. Return shipping status and tracking number.",
        "order_id",
        {"type": "integer", "minimum": 1, "maximum": 9999},
    ),
    describe_tool(
        "find_order_by_tracking",
        "Find the sample order associated with a ten-digit tracking number.",
        "tracking_number",
        {"type": "string", "pattern": "^[0-9]{10}$"},
    ),
]


def lookup(name, arguments):
    if name == "lookup_order":
        order_id = arguments.get("order_id")
        if type(order_id) is not int or not 1 <= order_id <= 9999:
            return {"error_code": "invalid_order_id"}, True
        order = ORDERS.get(order_id)
    else:
        tracking = arguments.get("tracking_number")
        if not isinstance(tracking, str) or not re.fullmatch(r"[0-9]{10}", tracking):
            return {"error_code": "invalid_tracking_number"}, True
        order = TRACKING_NUMBERS.get(tracking)
    if order is None:
        return {"found": False, "source": "demo_orders"}, False
    return {"found": True, "source": "demo_orders", **order}, False


def send(request_id, **response):
    message = {"jsonrpc": "2.0", "id": request_id, **response}
    print(json.dumps(message, separators=(",", ":")), flush=True)


def send_error(request_id, code, message):
    send(request_id, error={"code": code, "message": message})


def handle(request):
    if not isinstance(request, dict) or request.get("jsonrpc") != "2.0":
        send_error(None, -32600, "Invalid Request")
        return
    if "id" not in request:
        return
    request_id = request["id"]
    method = request.get("method")
    params = request.get("params", {})
    if not isinstance(params, dict):
        send_error(request_id, -32602, "Invalid params")
        return
    if method == "initialize":
        version = params.get("protocolVersion")
        send(request_id, result={
            "protocolVersion": version if version in PROTOCOL_VERSIONS else PROTOCOL_VERSIONS[0],
            "capabilities": {"tools": {"listChanged": False}},
            "serverInfo": {"name": "orders-demo", "version": "1.0.0"},
        })
    elif method == "ping":
        send(request_id, result={})
    elif method == "tools/list":
        send(request_id, result={"tools": TOOLS})
    elif method == "tools/call":
        name = params.get("name")
        arguments = params.get("arguments", {})
        if name not in ("lookup_order", "find_order_by_tracking") or not isinstance(arguments, dict):
            send_error(request_id, -32602, "Invalid tool or arguments")
            return
        output, is_error = lookup(name, arguments)
        send(request_id, result={
            "content": [{"type": "text", "text": json.dumps(output)}],
            "structuredContent": output,
            "isError": is_error,
        })
    else:
        send_error(request_id, -32601, "Method not found")


def main():
    for line in sys.stdin:
        try:
            request = json.loads(line)
        except json.JSONDecodeError:
            send_error(None, -32700, "Parse error")
            continue
        handle(request)


if __name__ == "__main__":
    main()
