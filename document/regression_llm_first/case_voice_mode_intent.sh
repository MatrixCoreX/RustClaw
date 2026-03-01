#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "${SCRIPT_DIR}/lib.sh"

health_check

VOICE_MODE_PROMPT_TEMPLATE="$(cat "${SCRIPT_DIR}/../../prompts/voice_mode_intent_prompt.md")"

parse_voice_mode_label() {
  local raw="${1:-}"
  python3 - "$raw" <<'PY'
import json
import re
import sys

raw = (sys.argv[1] or "").strip().lower()
if not raw:
    print("")
    raise SystemExit(0)

def parse_label(text: str):
    text = text.strip().lower()
    if not text:
        return None
    try:
        v = json.loads(text)
        if isinstance(v, dict) and isinstance(v.get("mode"), str):
            return parse_label(v["mode"])
    except Exception:
        pass
    start = text.find("{")
    end = text.rfind("}")
    if start >= 0 and end > start:
        part = text[start:end + 1]
        try:
            v = json.loads(part)
            if isinstance(v, dict) and isinstance(v.get("mode"), str):
                return parse_label(v["mode"])
        except Exception:
            pass
    allowed = {"voice", "text", "both", "reset", "show", "none"}
    if text in allowed:
        return text
    m = re.search(r"[a-z]+", text)
    if m:
        token = m.group(0)
        if token in allowed:
            return token
    if (
        "none" in text
        or "not a mode" in text
        or "no mode switch" in text
        or "不是模式切换" in text
        or "非模式切换" in text
    ):
        return "none"
    if "reset" in text or "default mode" in text or "恢复默认" in text or "重置" in text:
        return "reset"
    if (
        "show" in text
        or "status" in text
        or "current mode" in text
        or "查看语音模式" in text
        or "当前是语音还是文字" in text
    ):
        return "show"
    if (
        "both" in text
        or "voice and text" in text
        or "text and voice" in text
        or "语音和文字都要" in text
        or "语音和文本都发" in text
        or "两种都回复" in text
    ):
        return "both"
    if (
        "voice-only" in text
        or "voice only" in text
        or "only voice" in text
        or "切到语音" in text
        or "语音回复" in text
        or "只用语音" in text
        or "仅语音" in text
    ):
        return "voice"
    if (
        "text-only" in text
        or "text only" in text
        or "only text" in text
        or "切回文字" in text
        or "文字回复" in text
        or "只要文字" in text
        or "仅文字" in text
        or "只用文字" in text
        or "只打字" in text
    ):
        return "text"
    if "voice" in text or "语音" in text:
        return "voice"
    if "text" in text or "文字" in text or "文本" in text or "打字" in text:
        return "text"
    return None

print(parse_label(raw) or "")
PY
}

run_voice_mode_case() {
  local case_name="$1"
  local user_text="$2"
  local expected="$3"
  local prompt
  prompt="$(python3 - "$VOICE_MODE_PROMPT_TEMPLATE" "$user_text" <<'PY'
import sys
tpl = sys.argv[1]
text = sys.argv[2].strip()
print(tpl.replace("__USER_TEXT__", text))
PY
)"

  echo "[CASE] ${case_name}"
  echo "user_text: ${user_text}"
  local submit_resp task_id row status text error parsed
  submit_resp="$(submit_task_with_options "$prompt" "false" "voice_mode_intent_detect_regression")"
  task_id="$(extract_submit_task_id "$submit_resp")"
  echo "task_id: ${task_id}"
  row="$(wait_task_until_terminal "$task_id")"
  status="$(printf '%s' "$row" | awk -F'\t' '{print $1}')"
  text="$(printf '%s' "$row" | awk -F'\t' '{print $2}')"
  error="$(printf '%s' "$row" | awk -F'\t' '{print $3}')"
  if ! is_expected_status "$status" "succeeded"; then
    echo "FAIL: status=${status} error=${error}"
    return 1
  fi
  parsed="$(parse_voice_mode_label "$text")"
  if [ "$parsed" != "$expected" ]; then
    echo "FAIL: expected=${expected} parsed=${parsed}"
    echo "raw_text=${text}"
    return 1
  fi
  echo "PASS: parsed=${parsed}"
}

run_voice_mode_case "voice_mode_text_cn" "切回文字聊天模式" "text"
run_voice_mode_case "voice_mode_voice_cn" "切到语音回复" "voice"
run_voice_mode_case "voice_mode_both_cn" "语音和文字都要" "both"
run_voice_mode_case "voice_mode_reset_cn" "恢复默认回复模式" "reset"
run_voice_mode_case "voice_mode_show_cn" "现在是语音还是文字" "show"
run_voice_mode_case "voice_mode_none_cn" "帮我写个周报" "none"
run_voice_mode_case "voice_mode_text_en" "switch back to text mode" "text"
run_voice_mode_case "voice_mode_voice_en" "change to voice reply mode" "voice"

# Adversarial inputs: mixed language, negation, noisy colloquial.
run_voice_mode_case "voice_mode_adv_mixed_text" "切回 text mode，不要 voice 了" "text"
run_voice_mode_case "voice_mode_adv_mixed_voice" "改成 voice 回复，文字先别发" "voice"
run_voice_mode_case "voice_mode_adv_negation_none" "不要切模式，继续帮我总结今天会议" "none"
run_voice_mode_case "voice_mode_adv_noisy_both" "呃那个...都要吧，语音+文字一起回我" "both"
run_voice_mode_case "voice_mode_adv_noisy_reset" "emmm reset 一下，恢复默认回复模式" "reset"
run_voice_mode_case "voice_mode_adv_status_question" "不是改模式，我只是问现在是啥模式" "show"
