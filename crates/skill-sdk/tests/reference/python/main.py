import json
import pathlib
import sys
import time


def response(request_id: str, extra: dict) -> dict:
    return {"request_id": request_id, "status": "ok", "text": "", "error_text": None, "extra": extra}


request = json.loads(sys.stdin.readline())
request_id = request["request_id"]
args = request.get("args", {})
action = args.get("action", "calculate")
if action == "calculate":
    output = response(request_id, {"result": {"value": args.get("a", 0) + args.get("b", 0)}})
elif action == "validation_error":
    output = {"request_id": request_id, "status": "error", "text": "", "error_text": "invalid fixture input", "extra": {"error_code": "fixture_invalid", "message_key": "fixture.invalid"}}
elif action == "artifact":
    pathlib.Path(args["artifact_path"]).write_text("reference-artifact\n", encoding="utf-8")
    output = response(request_id, {"artifact": {"created": True}})
elif action == "waiting":
    output = response(request_id, {"continuation": {"state": "waiting", "poll_after_ms": 10}})
elif action == "needs_user":
    output = response(request_id, {"continuation": {"state": "needs_user", "required_fields": ["confirmation"]}})
elif action == "timeout":
    time.sleep(5)
    output = response(request_id, {})
elif action == "malformed":
    print("{not-json")
    raise SystemExit(0)
elif action == "multiple":
    print(json.dumps(response(request_id, {}), separators=(",", ":")))
    print(json.dumps(response(request_id, {}), separators=(",", ":")))
    raise SystemExit(0)
elif action == "oversized":
    print("x" * (1024 * 1024 + 1))
    raise SystemExit(0)
elif action == "stderr":
    print("reference diagnostic", file=sys.stderr)
    output = response(request_id, {"diagnostic_preserved": True})
else:
    output = response(request_id, {})
print(json.dumps(output, separators=(",", ":")))
