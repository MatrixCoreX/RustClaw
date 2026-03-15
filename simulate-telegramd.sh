#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

KIND="ask"
TEXT=""
AGENT_MODE="on"
SKILL_NAME=""
SKILL_ARGS=""
USER_ID="1"
CHAT_ID="1"
WAIT_SECONDS="60"
BASE_URL=""

usage() {
  cat <<'EOF'
Usage:
  ./simulate-telegramd.sh [options]

Modes:
  1) Ask mode (default):
     ./simulate-telegramd.sh --text "帮我生成一个拟人小龙虾矿工挖矿的图片"

  2) Run-skill mode:
     ./simulate-telegramd.sh --kind run_skill --skill image_generate --skill-args '{"prompt":"A lobster miner","size":"1024x1024"}'

Options:
  --kind ask|run_skill         Task kind (default: ask)
  --text TEXT                  Ask text when --kind ask
  --agent-mode on|off          Ask payload agent_mode (default: on)
  --skill NAME                 Skill name when --kind run_skill
  --skill-args JSON_OR_TEXT    Skill args when --kind run_skill
  --user-id ID                 user_id (default: 1)
  --chat-id ID                 chat_id (default: 1)
  --wait-seconds N             Poll timeout seconds (default: 60)
  --base-url URL               Override clawd base URL (default from configs/config.toml)
  -h, --help                   Show help

Notes:
  - This script simulates telegramd submit + poll behavior.
  - It does not call Telegram API.
EOF
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --kind)
      KIND="${2:-}"
      shift 2
      ;;
    --text)
      TEXT="${2:-}"
      shift 2
      ;;
    --agent-mode)
      AGENT_MODE="${2:-}"
      shift 2
      ;;
    --skill)
      SKILL_NAME="${2:-}"
      shift 2
      ;;
    --skill-args)
      SKILL_ARGS="${2:-}"
      shift 2
      ;;
    --user-id)
      USER_ID="${2:-}"
      shift 2
      ;;
    --chat-id)
      CHAT_ID="${2:-}"
      shift 2
      ;;
    --wait-seconds)
      WAIT_SECONDS="${2:-}"
      shift 2
      ;;
    --base-url)
      BASE_URL="${2:-}"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown argument: $1"
      usage
      exit 1
      ;;
  esac
done

if [[ "$KIND" != "ask" && "$KIND" != "run_skill" ]]; then
  echo "Invalid --kind: $KIND (expected ask|run_skill)"
  exit 1
fi

if [[ "$AGENT_MODE" != "on" && "$AGENT_MODE" != "off" ]]; then
  echo "Invalid --agent-mode: $AGENT_MODE (expected on|off)"
  exit 1
fi

if ! [[ "$WAIT_SECONDS" =~ ^[0-9]+$ ]] || [[ "$WAIT_SECONDS" -le 0 ]]; then
  echo "--wait-seconds must be a positive integer"
  exit 1
fi

if [[ -z "$BASE_URL" ]]; then
  BASE_URL="$(
python3 - <<'PY'
import tomllib
from pathlib import Path

cfg = tomllib.loads(Path("configs/config.toml").read_text(encoding="utf-8"))
listen = str(cfg.get("server", {}).get("listen", "127.0.0.1:8787"))
print(f"http://{listen}")
PY
  )"
fi

if [[ "$KIND" == "ask" ]]; then
  if [[ -z "$TEXT" ]]; then
    echo "--text is required when --kind ask"
    exit 1
  fi
elif [[ "$KIND" == "run_skill" ]]; then
  if [[ -z "$SKILL_NAME" ]]; then
    echo "--skill is required when --kind run_skill"
    exit 1
  fi
fi

export SIM_KIND="$KIND"
export SIM_TEXT="$TEXT"
export SIM_AGENT_MODE="$AGENT_MODE"
export SIM_SKILL_NAME="$SKILL_NAME"
export SIM_SKILL_ARGS="$SKILL_ARGS"
export SIM_USER_ID="$USER_ID"
export SIM_CHAT_ID="$CHAT_ID"
export SIM_WAIT_SECONDS="$WAIT_SECONDS"
export SIM_BASE_URL="$BASE_URL"

python3 - <<'PY'
import json
import os
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

kind = os.environ["SIM_KIND"]
text = os.environ["SIM_TEXT"]
agent_mode = os.environ["SIM_AGENT_MODE"] == "on"
skill_name = os.environ["SIM_SKILL_NAME"]
skill_args_raw = os.environ["SIM_SKILL_ARGS"]
user_id = int(os.environ["SIM_USER_ID"])
chat_id = int(os.environ["SIM_CHAT_ID"])
wait_seconds = int(os.environ["SIM_WAIT_SECONDS"])
base_url = os.environ["SIM_BASE_URL"].rstrip("/")

poll_interval = 0.5
max_rounds = max(1, int(wait_seconds / poll_interval))

def http_json(method: str, url: str, payload=None):
    data = None
    headers = {}
    if payload is not None:
        data = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            body = resp.read().decode("utf-8")
            return resp.getcode(), body
    except urllib.error.HTTPError as e:
        body = e.read().decode("utf-8", errors="replace")
        return e.code, body

def parse_api_response(raw: str):
    try:
        return json.loads(raw)
    except Exception:
        return None

def parse_skill_args(raw: str):
    if raw.strip() == "":
        return ""
    try:
        return json.loads(raw)
    except Exception:
        return raw

if kind == "ask":
    payload = {"text": text, "agent_mode": agent_mode}
else:
    payload = {"skill_name": skill_name, "args": parse_skill_args(skill_args_raw)}

submit_req = {
    "user_id": user_id,
    "chat_id": chat_id,
    "kind": kind,
    "payload": payload,
}

print(f"[submit] POST {base_url}/v1/tasks")
status, body = http_json("POST", f"{base_url}/v1/tasks", submit_req)
print(f"[submit] status={status}")
if status < 200 or status >= 300:
    print(body)
    sys.exit(1)

submit_obj = parse_api_response(body)
if not submit_obj or not submit_obj.get("ok"):
    print(f"[submit] invalid/failed response: {body}")
    sys.exit(1)

task_id = (((submit_obj.get("data") or {}).get("task_id")) or "").strip()
if not task_id:
    print(f"[submit] missing task_id: {body}")
    sys.exit(1)

print(f"[submit] task_id={task_id}")

for _ in range(max_rounds):
    status, body = http_json("GET", f"{base_url}/v1/tasks/{task_id}")
    if status < 200 or status >= 300:
        print(f"[poll] status={status} body={body}")
        sys.exit(1)
    obj = parse_api_response(body)
    if not obj or not obj.get("ok"):
        print(f"[poll] invalid/failed response: {body}")
        sys.exit(1)
    data = obj.get("data") or {}
    task_status = data.get("status")
    if task_status in ("queued", "running"):
        time.sleep(poll_interval)
        continue
    if task_status == "succeeded":
        result = data.get("result_json") or {}
        answer = (result.get("text") or "").strip()
        print("[result] succeeded")
        print("----- text begin -----")
        print(answer)
        print("----- text end -------")

        file_tokens = []
        for line in answer.splitlines():
            t = line.strip()
            if t.startswith("FILE:") or t.startswith("IMAGE_FILE:"):
                _, path = t.split(":", 1)
                p = path.strip().strip('"').strip("'").strip("`")
                if p:
                    file_tokens.append(p)
        if file_tokens:
            print("[result] detected file tokens:")
            for p in file_tokens:
                exists = Path(p).exists()
                kind = "file" if Path(p).is_file() else "not-file"
                print(f"  - {p} (exists={exists}, type={kind})")
        else:
            print("[result] no FILE:/IMAGE_FILE: token found")
        sys.exit(0)

    err = data.get("error_text") or "unknown error"
    print(f"[result] status={task_status} error={err}")
    sys.exit(1)

print("[poll] timeout waiting for task result")
sys.exit(2)
PY
