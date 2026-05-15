#!/usr/bin/env python3
# RustClaw 小屏监控：480×320 全屏，健康状态慢刷新、日志温和刷新，左侧龙虾动图 + RustClaw 标题。
# 需先启动 clawd（8787）。按 F11 或 Escape 退出全屏/关闭。

import json
import logging
import os
import random
import re
import queue
import sys
import tkinter as tk
import threading
import time
from datetime import datetime, timedelta

from small_screen_assets import find_assets, find_image_dir, find_splash_image, list_gallery_images
from small_screen_clawd_client import (
    API_BASE,
    fetch_health,
    fetch_skills_config,
    localhost_api_request,
    reset_admin_login_account,
)
from small_screen_config import (
    _config_path,
    _default_lang_from_system,
    _load_sqlite_path_from_config,
    _pi_app_dir,
    _root_dir,
    _writable_pi_app_dir,
    ensure_small_screen_auth_key,
    load_auth_key,
    load_crypto_page_visible,
    load_gallery_page_visible,
    load_lang,
    load_logs_page_visible,
    load_messages_page_visible,
    load_skills_page_visible,
    load_stock_page_visible,
    load_theme,
    load_us_stock_page_visible,
    load_weather_page_visible,
    save_auth_key,
    save_crypto_page_visible,
    save_gallery_page_visible,
    save_lang,
    save_logs_page_visible,
    save_messages_page_visible,
    save_skills_page_visible,
    save_stock_page_visible,
    save_theme,
    save_us_stock_page_visible,
    save_weather_page_visible,
)
from small_screen_cryptoauth_service import read_slot0_pubkey_via_helper, sign_unix_time_via_helper
from small_screen_formatters import (
    _fmt_signed_pct,
    _line_clamp_text,
    _safe_float,
    _strip_trailing_zeros,
    fmt_bytes,
    fmt_duration,
)
from small_screen_market_service import (
    DEFAULT_A_SHARE_REFRESH_SEC,
    DEFAULT_CRYPTO_REFRESH_SEC,
    DEFAULT_US_STOCK_REFRESH_SEC,
    _decode_sina_body,
    _load_small_screen_crypto_config,
    _load_small_screen_market_config,
    _load_small_screen_stock_config,
    _load_small_screen_us_stock_config,
    _normalize_stock_code,
    _normalize_us_stock_symbol,
    _parse_refresh_seconds,
    _parse_sina_quotes,
    fetch_a_share_quotes,
    fetch_crypto_prices,
    fetch_us_stock_quotes,
)
from small_screen_overview_ui import (
    build_overview_layout as overview_build_layout,
    render_dashboard_overview as overview_render_dashboard,
)
from small_screen_settings_ui import (
    close_wifi_to_settings as settings_close_wifi_to_settings,
    open_wifi_from_settings as settings_open_wifi_from_settings,
    prepare_settings_view as settings_prepare_view,
    refresh_settings_choice_labels as settings_refresh_choice_labels,
    show_settings_category as settings_show_category,
    show_settings_menu as settings_show_menu,
)
from small_screen_strings import STRINGS
from small_screen_themes import MATRIX_CHARS, THEMES
from small_screen_weather_service import (
    _fetch_today_weather_once,
    _format_weather_wind,
    _load_small_screen_weather_config,
    _pick_weather_text,
    _weather_day_label,
    _weather_desc_for_code,
    _weather_icon_for_code,
    _wind_level_from_kmh,
    fetch_today_weather,
)
from small_screen_wifi_service import connect_wifi_network, disconnect_wifi_network, scan_wifi_networks

logger = logging.getLogger(__name__)

HEALTH_REFRESH_SEC = 5
LOGS_REFRESH_SEC = 5
W, H = 480, 320
ASSETS_DIR = None


def _strip_ansi(text):
    out = []
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "\x1b":
            i += 1
            while i < len(text) and text[i] != "m":
                i += 1
            i += 1
            continue
        out.append(ch)
        i += 1
    return "".join(out)


def _sanitize_display_text(text):
    normalized = _strip_ansi(str(text or "")).replace("\r\n", "\n").replace("\r", "\n")
    out = []
    for ch in normalized:
        code = ord(ch)
        if ch in ("\n", "\t"):
            out.append(ch)
            continue
        if code < 0x20 or 0x7F <= code <= 0x9F:
            continue
        if 0xD800 <= code <= 0xDFFF:
            continue
        out.append(ch)
    return "".join(out).strip()


def _flatten_nonempty_lines(text):
    lines = [line.strip() for line in str(text or "").split("\n")]
    return " ".join(line for line in lines if line)


def _extract_log_time_label(line):
    line = line or ""
    iso = re.search(r"\b(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?Z)\b", line)
    if iso:
        try:
            dt = datetime.fromisoformat(iso.group(1).replace("Z", "+00:00"))
            return dt.astimezone().strftime("%H:%M:%S")
        except Exception:
            pass
    m = re.search(r"\b(\d{2}:\d{2}:\d{2})\b", line)
    if m:
        return m.group(1)
    return "--:--:--"


def _status_tag(line):
    lower = (line or "").lower()
    if any(key in lower for key in (" stage=error", " status=failed", " failed status=", " failed:", " panic", " error")):
        return "fail"
    if any(key in lower for key in (" stage=response", " status=ok", " success", " completed", " done")):
        return "ok"
    if " stage=request" in lower:
        return "req"
    return "run"


def _short_stage_name(stage):
    stage = (stage or "").strip().lower()
    mapping = {"request": "req", "response": "resp", "error": "err"}
    return mapping.get(stage, stage[:8] or "run")


def _shorten_model_name(model, limit=22):
    model = (model or "").strip()
    if len(model) <= limit:
        return model
    return model[: limit - 3] + "..."


def _prompt_token(value, limit=16):
    token = (value or "").strip()
    if not token:
        return ""
    token = token.replace(chr(92), '/').rsplit('/', 1)[-1]
    if token.endswith('.md'):
        token = token[:-3]
    if len(token) <= limit:
        return token
    return token[: limit - 3] + "..."


def _trim_evt_detail(line, limit=56):
    compact = re.sub(r"\s+", " ", line or "").strip()
    compact = re.sub(
        r"^(?:\d{4}-\d{2}-\d{2}[T ]\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)\s*",
        "",
        compact,
    )
    compact = re.sub(r"^\d{2}:\d{2}:\d{2}\s*", "", compact)
    if len(compact) > limit:
        compact = compact[:limit] + "..."
    return compact


def _extract_log_detail(line):
    line = line or ""
    status = _status_tag(line)
    llm_call = re.search(
        r"\[LLM_CALL\].*stage=([A-Za-z0-9_./:-]+).*?vendor=([A-Za-z0-9_./:-]+)\s+model=([A-Za-z0-9_./:-]+)(?:\s+model_kind=([A-Za-z0-9_./:-]+))?.*?prompt_file=([^\s]+)",
        line,
    )
    if llm_call:
        stage = _short_stage_name(llm_call.group(1))
        vendor = llm_call.group(2)
        model = _shorten_model_name(llm_call.group(3))
        prompt = _prompt_token(llm_call.group(5))
        prompt_part = f" p={prompt}" if prompt else ""
        return f"{stage} {vendor}/{model}{prompt_part} {status}"
    prompt_invocation = re.search(
        r"prompt_invocation .* prompt_name=([^\s]+)\s+prompt_file=([^\s]+)",
        line,
    )
    if prompt_invocation:
        prompt_name = _prompt_token(prompt_invocation.group(1), limit=14)
        prompt_file = _prompt_token(prompt_invocation.group(2), limit=14)
        prompt = prompt_file or prompt_name
        return f"prompt {prompt} ok"
    skill_model = re.search(
        r"skill_model_selected .* skill=([A-Za-z0-9_./:-]+)\s+provider=([A-Za-z0-9_./:-]+)\s+model=([A-Za-z0-9_./:-]+)(?:\s+model_kind=([A-Za-z0-9_./:-]+))?",
        line,
    )
    if skill_model:
        skill_name = skill_model.group(1)
        provider = skill_model.group(2)
        model = _shorten_model_name(skill_model.group(3), limit=18)
        return f"sel {skill_name} {provider}/{model} ok"
    skill_llm = re.search(
        r"skill_llm_call .* skill=([A-Za-z0-9_./:-]+)\s+prompt=([A-Za-z0-9_./:-]+)\s+model=([A-Za-z0-9_./:-]+)",
        line,
    )
    if skill_llm:
        skill_name = skill_llm.group(1)
        prompt = _prompt_token(skill_llm.group(2), limit=14)
        model = _shorten_model_name(skill_llm.group(3), limit=18)
        prompt_part = f" p={prompt}" if prompt else ""
        return f"call {skill_name} {model}{prompt_part} {status}"
    decision = re.search(r"first_layer_decision=(clarify|direct_answer|planner_execute)\b", line)
    if decision:
        return f"route {decision.group(1)} {status}"
    label = re.search(r"derived_route_label=(ChatAct|AskClarify|Chat|Act)\b", line)
    if label:
        return f"route {label.group(1)} {status}"
    tool = re.search(r"type=call_tool tool=([A-Za-z0-9_./:-]+)", line)
    if tool:
        return f"tool {tool.group(1)} {status}"
    skill = re.search(r"type=call_skill(?:\(rerouted\))? skill=([A-Za-z0-9_./:-]+)", line)
    if skill:
        return f"skill {skill.group(1)} {status}"
    task_status = re.search(r"task_call_end .* status=([A-Za-z0-9_-]+)", line)
    if task_status:
        return f"task {task_status.group(1)}"
    compact = _trim_evt_detail(line)
    return f"evt {compact}" if compact else "evt"

def _collect_clawd_log_items(raw_text, lang="CN", limit=8):
    items = []
    lines = (raw_text or "").splitlines()
    for raw in reversed(lines):
        line = _strip_ansi(raw).strip()
        if not line:
            continue
        lower = line.lower()
        time_label = _extract_log_time_label(line)
        item = None
        if any(key in lower for key in ("error", " status=failed", " warn!", "panic", "failed:")):
            item = "ERROR"
        elif any(key in lower for key in ("prompt_invocation", "prompt_debug", "llm_call", "[llm]", "[prompt]")):
            item = "LLM"
        elif any(key in lower for key in ("first_layer_decision", "derived_route_label", "resolve_user_request", "context_resolver", "[routing]")):
            item = "ROUTING"
        elif any(key in lower for key in ("type=call_skill", "skill=", "[skill]", "[skill_llm]")):
            item = "SKILL"
        elif any(key in lower for key in ("type=call_tool", "executor_step_execute", "[tool]")):
            item = "TOOL"
        elif any(key in lower for key in (
            "task_call_end",
            "worker_once:",
            "loop_round_",
            "act_split_trace",
            "executor_step_",
            "executor_result_",
            "[loop]",
        )):
            item = "TASK"
        elif "[" in line and "]" in line:
            item = "OTHER"
        if item:
            items.append({
                "time": time_label,
                "kind": item,
                "detail": _extract_log_detail(line),
                "raw": line,
            })
        if limit and len(items) >= limit:
            break
    return items


def _strip_message_log_suffix(text):
    return re.split(r"\s+call_id=[^\s]+", text or "", maxsplit=1)[0].strip()


def _split_message_lines_for_display(text):
    s = _sanitize_display_text(text or "")
    if not s:
        return []
    normalized = (
        s.replace("\\r\\n", "\n")
        .replace("\\n", "\n")
        .replace("\r\n", "\n")
        .replace("\r", "\n")
    )
    return [line.strip() for line in normalized.split("\n") if line.strip()]


def _message_more_suffix(lang="CN"):
    hint = (STRINGS.get(lang, STRINGS["CN"]).get("recent_message_more_hint") or "").strip()
    if not hint:
        hint = STRINGS["CN"]["recent_message_more_hint"]
    if lang == "EN":
        return f"... ({hint})"
    return f"...（{hint}）"


def _extract_task_id(line):
    if not line:
        return ""
    match = re.search(r'\btask_id[=:]"?([0-9A-Za-z-]+)"?', line)
    if not match:
        return ""
    return (match.group(1) or "").strip().strip('",;]}')


def _single_line_message_preview(text, lang="CN"):
    raw = _sanitize_display_text(text or "")
    lines = _split_message_lines_for_display(raw)
    if not lines:
        return ""
    has_multiline_marker = (
        ("\n" in raw)
        or ("\r" in raw)
        or ("\\n" in raw)
        or ("\\r" in raw)
    )
    if len(lines) > 1 or has_multiline_marker:
        return lines[0] + _message_more_suffix(lang)
    return lines[0]


def _collect_recent_user_messages(raw_text, limit=5, lang="CN"):
    items_by_key = {}
    ordered_keys = []
    for raw in reversed((raw_text or "").splitlines()):
        line = _strip_ansi(raw).strip()
        if not line:
            continue
        user_id = ""
        chat_id = ""
        task_id = ""
        user_match = re.search(r"\buser_id=([^\s]+)", line)
        chat_match = re.search(r"\bchat_id=([^\s]+)", line)
        task_match = re.search(r"\btask_id=([^\s]+)", line)
        if user_match:
            user_id = user_match.group(1)
        if chat_match:
            chat_id = chat_match.group(1)
        if task_match:
            task_id = task_match.group(1)
        task_id = task_id or _extract_task_id(line)
        if task_id:
            task_id = task_id.strip().strip('",;]}')

        text = ""
        field = None
        priority = 0
        if "task_call_end" in line and " kind=ask " in line and " result=" in line:
            result_segment = line.split(" result=", 1)[1].strip()
            text = _strip_message_log_suffix(result_segment)
            if "call_id=" not in result_segment and text:
                text = text.rstrip(". ") + _message_more_suffix(lang)
            field = "reply"
            priority = 10
        elif "worker_once: ask raw_message" in line and " text=" in line:
            text = _strip_message_log_suffix(line.split(" text=", 1)[1].strip())
            field = "question"
            priority = 4
        elif "worker_once: ask received_message" in line and " text=" in line:
            text = _strip_message_log_suffix(line.split(" text=", 1)[1].strip())
            field = "question"
            priority = 3
        elif "plan_llm_request" in line and " user_request=" in line:
            text = _strip_message_log_suffix(line.split(" user_request=", 1)[1].strip())
            field = "question"
            priority = 2
        elif "worker_once: ask resolved_message" in line and " resolved_text=" in line:
            text = _strip_message_log_suffix(line.split(" resolved_text=", 1)[1].strip())
            field = "question"
            priority = 1
        if not field or not text:
            continue

        key = task_id or f"{_extract_log_time_label(line)}|{text}"
        item = items_by_key.get(key)
        if item is None:
            item = {
                "time": _extract_log_time_label(line),
                "text": "",
                "question": "",
                "reply": "",
                "user_id": user_id,
                "chat_id": chat_id,
                "task_id": task_id,
                "_question_priority": -1,
                "_reply_priority": -1,
            }
            items_by_key[key] = item
            ordered_keys.append(key)

        if user_id and not item.get("user_id"):
            item["user_id"] = user_id
        if chat_id and not item.get("chat_id"):
            item["chat_id"] = chat_id
        if task_id and not item.get("task_id"):
            item["task_id"] = task_id
        if not item.get("time"):
            item["time"] = _extract_log_time_label(line)

        if field == "question":
            if priority > item.get("_question_priority", -1):
                item["question"] = text
                item["text"] = text
                item["_question_priority"] = priority
        else:
            if priority > item.get("_reply_priority", -1):
                item["reply"] = text
                item["_reply_priority"] = priority

    items = []
    for key in ordered_keys:
        item = dict(items_by_key[key])
        item.pop("_question_priority", None)
        item.pop("_reply_priority", None)
        if item.get("question") or item.get("reply"):
            items.append(item)
        if limit and len(items) >= limit:
            break
    # 按 task_id 强制唯一：同一 task_id 只保留第一次出现的项
    deduped_items = []
    seen_task_ids = set()
    for item in items:
        tid = str(item.get("task_id") or "").strip()
        if tid:
            if tid in seen_task_ids:
                continue
            seen_task_ids.add(tid)
        deduped_items.append(item)
        if limit and len(deduped_items) >= limit:
            break
    return deduped_items


def fetch_clawd_logs(user_key="", lang="CN", lines=120, limit=24):
    try:
        query = urllib.parse.urlencode({"file": "clawd.log", "lines": lines})
        raw = localhost_api_request("GET", "/v1/logs/latest?" + query, user_key)
        body = json.loads(raw.decode())
        data = body.get("data") or body or {}
        return _collect_clawd_log_items(data.get("text") or "", lang=lang, limit=limit), None
    except Exception as e:
        return None, str(e)


def fetch_clawd_activity(user_key="", lang="CN", lines=300, log_limit=24, message_limit=5):
    try:
        query = urllib.parse.urlencode({"file": "clawd.log", "lines": lines})
        raw = localhost_api_request("GET", "/v1/logs/latest?" + query, user_key)
        body = json.loads(raw.decode())
        data = body.get("data") or body or {}
        raw_text = data.get("text") or ""
        return (
            _collect_clawd_log_items(raw_text, lang=lang, limit=log_limit),
            _collect_recent_user_messages(raw_text, limit=message_limit, lang=lang),
            None,
        )
    except Exception as e:
        return None, None, str(e)


def fetch_clawd_log_summary(user_key="", lang="CN"):
    return fetch_clawd_logs(user_key=user_key, lang=lang, lines=80, limit=8)


def _legacy_pick_weather_text(values):
    if not isinstance(values, list):
        return ""
    for item in values:
        if not isinstance(item, dict):
            continue
        text = str(item.get("value") or "").strip()
        if text:
            return text
    return ""


def _legacy_weather_icon_for_code(code):
    try:
        value = int(str(code or "").strip() or "-1")
    except Exception:
        value = -1
    if value == 113:
        return "☀"
    if value == 116:
        return "☁"
    if value in (119, 122):
        return "☁"
    if value in (143, 248, 260):
        return "≋"
    if value in (176, 263, 266, 293, 296, 299, 353, 356):
        return "☂"
    if value in (182, 185, 281, 284, 302, 305, 308, 311, 314, 317, 320, 359, 362, 365):
        return "☂"
    if value in (179, 227, 230, 323, 326, 329, 332, 335, 338, 368, 371):
        return "❄"
    if value in (200, 386, 389, 392, 395):
        return "⚡"
    return "◌"


def _legacy_weather_desc_for_code(code, lang="CN", fallback=""):
    try:
        value = int(str(code or "").strip() or "-1")
    except Exception:
        value = -1
    lang = "EN" if str(lang).upper() == "EN" else "CN"
    mapping = {
        113: {"CN": "晴", "EN": "Clear"},
        116: {"CN": "局部多云", "EN": "Partly cloudy"},
        119: {"CN": "多云", "EN": "Cloudy"},
        122: {"CN": "阴", "EN": "Overcast"},
        143: {"CN": "薄雾", "EN": "Mist"},
        176: {"CN": "局地阵雨", "EN": "Patchy rain"},
        179: {"CN": "局地小雪", "EN": "Patchy snow"},
        182: {"CN": "局地雨夹雪", "EN": "Patchy sleet"},
        185: {"CN": "局地冻毛雨", "EN": "Patchy freezing drizzle"},
        200: {"CN": "局地雷暴", "EN": "Thundery nearby"},
        227: {"CN": "吹雪", "EN": "Blowing snow"},
        230: {"CN": "暴风雪", "EN": "Blizzard"},
        248: {"CN": "雾", "EN": "Fog"},
        260: {"CN": "冻雾", "EN": "Freezing fog"},
        263: {"CN": "零星毛毛雨", "EN": "Patchy drizzle"},
        266: {"CN": "毛毛雨", "EN": "Drizzle"},
        281: {"CN": "冻毛雨", "EN": "Freezing drizzle"},
        284: {"CN": "强冻毛雨", "EN": "Heavy freezing drizzle"},
        293: {"CN": "零星小雨", "EN": "Patchy light rain"},
        296: {"CN": "小雨", "EN": "Light rain"},
        299: {"CN": "间歇中雨", "EN": "Moderate rain at times"},
        302: {"CN": "中雨", "EN": "Moderate rain"},
        305: {"CN": "间歇大雨", "EN": "Heavy rain at times"},
        308: {"CN": "大雨", "EN": "Heavy rain"},
        311: {"CN": "轻度冻雨", "EN": "Light freezing rain"},
        314: {"CN": "冻雨", "EN": "Freezing rain"},
        317: {"CN": "轻度雨夹雪", "EN": "Light sleet"},
        320: {"CN": "雨夹雪", "EN": "Sleet"},
        323: {"CN": "零星小雪", "EN": "Patchy light snow"},
        326: {"CN": "小雪", "EN": "Light snow"},
        329: {"CN": "间歇中雪", "EN": "Patchy moderate snow"},
        332: {"CN": "中雪", "EN": "Moderate snow"},
        335: {"CN": "间歇大雪", "EN": "Patchy heavy snow"},
        338: {"CN": "大雪", "EN": "Heavy snow"},
        353: {"CN": "小阵雨", "EN": "Light shower"},
        356: {"CN": "阵雨", "EN": "Rain shower"},
        359: {"CN": "暴雨", "EN": "Torrential rain"},
        362: {"CN": "轻度雨夹雪阵雨", "EN": "Light sleet shower"},
        365: {"CN": "雨夹雪阵雨", "EN": "Sleet shower"},
        368: {"CN": "小阵雪", "EN": "Light snow shower"},
        371: {"CN": "阵雪", "EN": "Snow shower"},
        386: {"CN": "局地雷阵雨", "EN": "Patchy thunder rain"},
        389: {"CN": "强雷雨", "EN": "Heavy thunder rain"},
        392: {"CN": "局地雷阵雪", "EN": "Patchy thunder snow"},
        395: {"CN": "强雷阵雪", "EN": "Heavy thunder snow"},
    }
    if value in mapping:
        return mapping[value][lang]
    fallback = str(fallback or "").strip()
    if fallback:
        return fallback
    return {"CN": "多云", "EN": "Cloudy"}[lang]


def _legacy_wind_level_from_kmh(speed_kmh):
    try:
        speed = float(str(speed_kmh or "").strip())
    except Exception:
        return None
    thresholds = [1, 5, 11, 19, 28, 38, 49, 61, 74, 88, 102, 117]
    for level, upper in enumerate(thresholds):
        if speed <= upper:
            return level
    return 12


def _legacy_format_weather_wind(speed_kmh, direction="", lang="CN"):
    speed_text = str(speed_kmh or "--").strip() or "--"
    direction_text = str(direction or "").strip()
    base = " ".join(part for part in (f"{speed_text} km/h", direction_text) if part).strip() or "--"
    level = _wind_level_from_kmh(speed_kmh)
    if level is None:
        return base
    if str(lang).upper() == "EN":
        return f"{base} (L{level})"
    return f"{base} ({level}级)"


def _legacy_load_small_screen_weather_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("weather") or {}) if isinstance(cfg, dict) else {}
    city = str(section.get("city") or "").strip()
    return {"city": city}


def _legacy_weather_day_label(date_text, offset=0, lang="CN"):
    lang = "EN" if str(lang).upper() == "EN" else "CN"
    if offset == 0:
        return "Today" if lang == "EN" else "今天"
    if offset == 1:
        return "Tomorrow" if lang == "EN" else "明天"
    try:
        dt = datetime.strptime(str(date_text or "").strip(), "%Y-%m-%d")
        idx = dt.weekday()
    except Exception:
        return f"D+{offset}" if lang == "EN" else f"{offset}天后"
    cn_days = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
    en_days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
    return (en_days if lang == "EN" else cn_days)[idx]


def _legacy_fetch_today_weather_once(lang="CN", city=""):
    city = str(city or "").strip()
    query = urllib.parse.urlencode(
        {
            "format": "j1",
            "lang": "zh" if str(lang).upper() == "CN" else "en",
        }
    )
    base_url = "https://wttr.in/"
    if city:
        base_url += urllib.parse.quote(city)
    req = urllib.request.Request(
        base_url + "?" + query,
        headers={
            "User-Agent": "RustClawSmallScreen/1.0",
            "Accept": "application/json",
        },
    )
    with urllib.request.urlopen(req, timeout=12) as resp:
        payload = json.loads(resp.read().decode("utf-8", "replace"))

    current = ((payload or {}).get("current_condition") or [{}])[0]
    today = ((payload or {}).get("weather") or [{}])[0]
    daily_items = (payload or {}).get("weather") or []
    nearest = ((payload or {}).get("nearest_area") or [{}])[0]
    if not current or not today:
        raise ValueError("invalid weather payload")

    astronomy = (today.get("astronomy") or [{}])[0]
    hourly = today.get("hourly") or []
    area_name = _pick_weather_text(nearest.get("areaName"))
    region_name = _pick_weather_text(nearest.get("region"))
    country_name = _pick_weather_text(nearest.get("country"))
    location = area_name or region_name or country_name or "--"
    if country_name and location and country_name != location:
        location = f"{location}, {country_name}"

    rain_values = []
    for item in hourly:
        try:
            rain_values.append(int(str(item.get("chanceofrain") or "0").strip() or "0"))
        except Exception:
            continue
    rain_chance = f"{max(rain_values)}%" if rain_values else "--"
    details = [
        {
            "day": _weather_day_label(today.get("date"), offset=0, lang=lang),
            "location": location,
            "code": str(current.get("weatherCode") or "").strip(),
            "icon": _weather_icon_for_code(current.get("weatherCode")),
            "temperature": f"{str(current.get('temp_C') or '--').strip()}°C",
            "description": _weather_desc_for_code(
                current.get("weatherCode"),
                lang=lang,
                fallback=_pick_weather_text(current.get("weatherDesc")) or "--",
            ),
            "feels_like": f"{str(current.get('FeelsLikeC') or '--').strip()}°C",
            "high_low": (
                f"{str(today.get('maxtempC') or '--').strip()}°C / "
                f"{str(today.get('mintempC') or '--').strip()}°C"
            ),
            "humidity": f"{str(current.get('humidity') or '--').strip()}%",
            "wind": _format_weather_wind(
                current.get("windspeedKmph"),
                current.get("winddir16Point"),
                lang=lang,
            ),
            "rain": rain_chance,
            "sunrise": str(astronomy.get("sunrise") or "--").strip() or "--",
            "sunset": str(astronomy.get("sunset") or "--").strip() or "--",
            "updated_at": datetime.now().strftime("%H:%M"),
        }
    ]
    forecast = []
    for offset, item in enumerate(daily_items[1:4], start=1):
        if not isinstance(item, dict):
            continue
        hourly_items = item.get("hourly") or []
        sample = hourly_items[min(4, len(hourly_items) - 1)] if hourly_items else {}
        code = str(sample.get("weatherCode") or "").strip()
        astronomy_item = (item.get("astronomy") or [{}])[0]
        day_rain_values = []
        for hourly_item in hourly_items:
            try:
                day_rain_values.append(int(str(hourly_item.get("chanceofrain") or "0").strip() or "0"))
            except Exception:
                continue
        day_rain = f"{max(day_rain_values)}%" if day_rain_values else "--"
        detail = {
            "day": _weather_day_label(item.get("date"), offset=offset, lang=lang),
            "location": location,
            "code": code,
            "icon": _weather_icon_for_code(code),
            "temperature": f"{str(sample.get('tempC') or item.get('avgtempC') or '--').strip()}°C",
            "description": _weather_desc_for_code(
                code,
                lang=lang,
                fallback=_pick_weather_text(sample.get("weatherDesc")) or "--",
            ),
            "feels_like": f"{str(sample.get('FeelsLikeC') or sample.get('tempC') or item.get('avgtempC') or '--').strip()}°C",
            "high_low": (
                f"{str(item.get('maxtempC') or '--').strip()}°C / "
                f"{str(item.get('mintempC') or '--').strip()}°C"
            ),
            "humidity": f"{str(sample.get('humidity') or '--').strip()}%",
            "wind": _format_weather_wind(
                sample.get("windspeedKmph"),
                sample.get("winddir16Point"),
                lang=lang,
            ),
            "rain": day_rain,
            "sunrise": str(astronomy_item.get("sunrise") or "--").strip() or "--",
            "sunset": str(astronomy_item.get("sunset") or "--").strip() or "--",
            "updated_at": datetime.now().strftime("%H:%M"),
        }
        forecast.append(
            {
                "offset": offset,
                "day": detail["day"],
                "icon": detail["icon"],
                "description": detail["description"],
                "high_low": (
                    f"{str(item.get('maxtempC') or '--').strip()}° / "
                    f"{str(item.get('mintempC') or '--').strip()}°"
                ),
            }
        )
        details.append(detail)

    return {
        "location": location,
        "code": str(current.get("weatherCode") or "").strip(),
        "icon": _weather_icon_for_code(current.get("weatherCode")),
        "temperature": f"{str(current.get('temp_C') or '--').strip()}°C",
        "description": _weather_desc_for_code(
            current.get("weatherCode"),
            lang=lang,
            fallback=_pick_weather_text(current.get("weatherDesc")) or "--",
        ),
        "feels_like": f"{str(current.get('FeelsLikeC') or '--').strip()}°C",
        "high_low": (
            f"{str(today.get('maxtempC') or '--').strip()}°C / "
            f"{str(today.get('mintempC') or '--').strip()}°C"
        ),
        "humidity": f"{str(current.get('humidity') or '--').strip()}%",
        "wind": _format_weather_wind(
            current.get("windspeedKmph"),
            current.get("winddir16Point"),
            lang=lang,
        ),
        "rain": rain_chance,
        "sunrise": str(astronomy.get("sunrise") or "--").strip() or "--",
        "sunset": str(astronomy.get("sunset") or "--").strip() or "--",
        "details": details,
        "forecast": forecast,
        "updated_at": datetime.now().strftime("%H:%M"),
    }


def _legacy_fetch_today_weather(lang="CN"):
    weather_cfg = _load_small_screen_weather_config()
    city = str(weather_cfg.get("city") or "").strip()
    if city:
        try:
            return _fetch_today_weather_once(lang=lang, city=city), None
        except Exception:
            pass
    try:
        return _fetch_today_weather_once(lang=lang, city=""), None
    except Exception as exc:
        return None, str(exc)


LEGACY_BINANCE_TICKER_URL = "https://api.binance.com/api/v3/ticker/price"
LEGACY_SINA_HQ_URL = "http://hq.sinajs.cn/list="
LEGACY_SINA_REFERER = "https://finance.sina.com.cn"
LEGACY_DEFAULT_A_SHARE_REFRESH_SEC = 15
LEGACY_DEFAULT_CRYPTO_REFRESH_SEC = 15
LEGACY_DEFAULT_US_STOCK_REFRESH_SEC = 15
from small_screen_clawd_client import fetch_skills_config as fetch_skills_config
from small_screen_market_service import (
    _decode_sina_body as _decode_sina_body,
    _load_small_screen_crypto_config as _load_small_screen_crypto_config,
    _load_small_screen_market_config as _load_small_screen_market_config,
    _load_small_screen_stock_config as _load_small_screen_stock_config,
    _load_small_screen_us_stock_config as _load_small_screen_us_stock_config,
    _normalize_stock_code as _normalize_stock_code,
    _normalize_us_stock_symbol as _normalize_us_stock_symbol,
    _parse_refresh_seconds as _parse_refresh_seconds,
    _parse_sina_quotes as _parse_sina_quotes,
    fetch_a_share_quotes as fetch_a_share_quotes,
    fetch_crypto_prices as fetch_crypto_prices,
    fetch_us_stock_quotes as fetch_us_stock_quotes,
)
from small_screen_weather_service import (
    _fetch_today_weather_once as _fetch_today_weather_once,
    _format_weather_wind as _format_weather_wind,
    _load_small_screen_weather_config as _load_small_screen_weather_config,
    _pick_weather_text as _pick_weather_text,
    _weather_day_label as _weather_day_label,
    _weather_desc_for_code as _weather_desc_for_code,
    _weather_icon_for_code as _weather_icon_for_code,
    _wind_level_from_kmh as _wind_level_from_kmh,
    fetch_today_weather as fetch_today_weather,
)

WEATHER_REFRESH_SEC = 15 * 60
OVERVIEW_SCROLL_SEC = 4
OVERVIEW_DOUBLE_TAP_SEC = 0.65
OVERVIEW_US_STOCK_HEIGHT = 90
OVERVIEW_A_STOCK_WIDTH = 216
OVERVIEW_MARKET_HEIGHT = 150
OVERVIEW_MARKET_GAP = 10
OVERVIEW_CRYPTO_HEIGHT = 74
OVERVIEW_RUNTIME_HEIGHT = 64
DEFAULT_A_SHARE_ITEMS = [
    {"name": "中国移动", "code": "600941"},
    {"name": "贵州茅台", "code": "600519"},
    {"name": "宁德时代", "code": "300750"},
    {"name": "比亚迪", "code": "002594"},
]
DEFAULT_CRYPTO_ITEMS = [
    {"name": "BTC", "symbol": "BTCUSDT"},
    {"name": "ETH", "symbol": "ETHUSDT"},
    {"name": "BCH", "symbol": "BCHUSDT"},
    {"name": "LTC", "symbol": "LTCUSDT"},
    {"name": "SOL", "symbol": "SOLUSDT"},
    {"name": "BNB", "symbol": "BNBUSDT"},
    {"name": "XRP", "symbol": "XRPUSDT"},
    {"name": "DOGE", "symbol": "DOGEUSDT"},
    {"name": "PEPE", "symbol": "PEPEUSDT"},
    {"name": "SHIB", "symbol": "SHIBUSDT"},
]
DEFAULT_US_STOCK_ITEMS = [
    {"name": "Apple", "symbol": "AAPL"},
    {"name": "NVIDIA", "symbol": "NVDA"},
    {"name": "Microsoft", "symbol": "MSFT"},
    {"name": "Tesla", "symbol": "TSLA"},
]


def _legacy_small_screen_market_config_path():
    return os.path.join(_pi_app_dir(), "small_screen_markets.toml")


def _legacy_load_small_screen_market_config():
    if tomllib is None:
        return {}
    try:
        with open(_small_screen_market_config_path(), "rb") as f:
            cfg = tomllib.load(f)
        return cfg if isinstance(cfg, dict) else {}
    except Exception:
        return {}


def _legacy_parse_refresh_seconds(value, default_value):
    if isinstance(value, (int, float)):
        return max(5, min(int(value), 3600))
    return default_value


def _legacy_load_small_screen_crypto_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("crypto") or {}) if isinstance(cfg, dict) else {}
    refresh_seconds = _parse_refresh_seconds(section.get("refresh_seconds"), DEFAULT_CRYPTO_REFRESH_SEC)
    items = []
    for item in section.get("items") or []:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name") or "").strip()
        symbol = str(item.get("symbol") or "").strip().upper()
        if name and symbol:
            items.append({"name": name, "symbol": symbol})
    if not items:
        items = [dict(item) for item in DEFAULT_CRYPTO_ITEMS]
    return items, refresh_seconds


def _legacy_fetch_crypto_prices(crypto_items=None):
    """从币安 API 拉取 USDT 价格，返回 { "BTC": "43210.5", ... }，失败返回 None。去掉小数点后尾部的 0。"""
    items = crypto_items or _load_small_screen_crypto_config()[0]
    try:
        req = urllib.request.Request(BINANCE_TICKER_URL)
        with urllib.request.urlopen(req, timeout=8) as r:
            data = json.loads(r.read().decode())
        if not isinstance(data, list):
            return None
        by_symbol = {item.get("symbol"): item.get("price") for item in data if isinstance(item, dict) and item.get("symbol") and item.get("price")}
        out = {}
        for item in items:
            name = item.get("name")
            symbol = item.get("symbol")
            if not name or not symbol:
                continue
            p = by_symbol.get(symbol)
            if p is not None:
                out[name] = _strip_trailing_zeros(p)
            else:
                out[name] = "--"
        return out
    except Exception:
        return None


def _legacy_normalize_stock_code(input_text):
    s = str(input_text or "").strip().lower()
    digits = "".join(ch for ch in s if ch.isdigit())
    if s.startswith(("sh", "sz")) and len(digits) == 6:
        return s[:2] + digits
    if len(digits) == 6:
        return ("sh" if digits.startswith("6") else "sz") + digits
    return ""


def _legacy_load_small_screen_stock_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("stocks") or {}) if isinstance(cfg, dict) else {}
    refresh_seconds = _parse_refresh_seconds(section.get("refresh_seconds"), DEFAULT_A_SHARE_REFRESH_SEC)
    items = []
    for item in section.get("items") or []:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name") or "").strip()
        code = _normalize_stock_code(item.get("code"))
        if code:
            items.append({"name": name or code.upper(), "code": code})
    if not items:
        items = [
            {"name": item["name"], "code": _normalize_stock_code(item["code"])}
            for item in DEFAULT_A_SHARE_ITEMS
        ]
    return items, refresh_seconds


def _legacy_normalize_us_stock_symbol(input_text):
    s = str(input_text or "").strip().upper()
    return re.sub(r"[^A-Z0-9\.\-]", "", s)


def _legacy_load_small_screen_us_stock_config():
    cfg = _load_small_screen_market_config()
    section = (cfg.get("us_stocks") or {}) if isinstance(cfg, dict) else {}
    refresh_seconds = _parse_refresh_seconds(section.get("refresh_seconds"), DEFAULT_US_STOCK_REFRESH_SEC)
    items = []
    for item in section.get("items") or []:
        if not isinstance(item, dict):
            continue
        name = str(item.get("name") or "").strip()
        symbol = _normalize_us_stock_symbol(item.get("symbol"))
        if symbol:
            items.append({"name": name or symbol, "symbol": symbol})
    if not items:
        items = [dict(item) for item in DEFAULT_US_STOCK_ITEMS]
    return items, refresh_seconds


def _decode_sina_body(raw):
    try:
        text = raw.decode("utf-8")
        if "var hq_str_" in text:
            return text
    except UnicodeDecodeError:
        pass
    return raw.decode("gbk", errors="ignore")


def _parse_sina_quotes(body):
    out = {}
    for code, payload in re.findall(r'var hq_str_([a-z]{2}\d{6})="([^"]*)";', body, flags=re.I):
        parts = [part.strip() for part in payload.split(",")]
        if len(parts) < 32:
            continue
        name = parts[0]
        if not name:
            continue
        norm_code = code.lower()
        out[norm_code] = {
            "name": name,
            "code": norm_code[2:],
            "open": parts[1] or "--",
            "prev_close": parts[2] or "--",
            "current": parts[3] or "--",
            "high": parts[4] or "--",
            "low": parts[5] or "--",
            "time": parts[31] or "--",
        }
        out[norm_code]["pct"] = _fmt_signed_pct(out[norm_code]["current"], out[norm_code]["prev_close"])
    return out


def _legacy_fetch_a_share_quotes(stock_items=None):
    items = stock_items or _load_small_screen_stock_config()[0]
    stock_codes = [item["code"] for item in items if item.get("code")]
    quotes = {}
    error = None
    if stock_codes:
        try:
            req = urllib.request.Request(SINA_HQ_URL + ",".join(stock_codes))
            req.add_header("Referer", SINA_REFERER)
            req.add_header("User-Agent", "RustClaw-Small-Screen/1.0")
            with urllib.request.urlopen(req, timeout=8) as r:
                quotes = _parse_sina_quotes(_decode_sina_body(r.read()))
        except Exception as exc:
            error = str(exc)

    out = []
    for item in items:
        code = item.get("code") or ""
        quote = quotes.get(code.lower()) if code else None
        if quote:
            display_name = item.get("name") or quote.get("name") or code.upper()
            out.append({
                "title": f"{display_name} · {quote.get('code') or '--'}",
                "price": quote.get("current") or "--",
                "pct": quote.get("pct") or "--",
                "meta1": f"今开 {quote.get('open') or '--'}  昨收 {quote.get('prev_close') or '--'}",
                "meta2": f"高/低 {quote.get('high') or '--'}/{quote.get('low') or '--'}  {quote.get('time') or '--'}",
            })
            continue
        reason = "行情获取失败" if error else "暂无今日行情"
        out.append({
            "title": item.get("name") or code.upper() or "--",
            "price": "--",
            "pct": "--",
            "meta1": reason[:28],
            "meta2": code.upper()[:28],
        })

    return {"items": out, "error": error}


def _legacy_fetch_us_stock_quotes(stock_items=None):
    items = stock_items or _load_small_screen_us_stock_config()[0]
    quotes = {}
    error = None
    for item in items:
        symbol = item.get("symbol") or ""
        if not symbol:
            continue
        try:
            url = (
                "https://query1.finance.yahoo.com/v8/finance/chart/"
                + urllib.parse.quote(symbol)
                + "?interval=1d&range=5d"
            )
            req = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
            with urllib.request.urlopen(req, timeout=8) as r:
                data = json.loads(r.read().decode("utf-8", "replace"))
            result = (((data or {}).get("chart") or {}).get("result") or [None])[0] or {}
            meta = (result.get("meta") or {}) if isinstance(result, dict) else {}
            if meta:
                quotes[symbol] = meta
        except Exception as exc:
            error = str(exc)

    out = []
    for item in items:
        symbol = item.get("symbol") or ""
        quote = quotes.get(symbol)
        if quote:
            display_name = item.get("name") or quote.get("shortName") or quote.get("longName") or symbol
            price = quote.get("regularMarketPrice")
            price_text = _strip_trailing_zeros(str(price)) if price is not None else "--"
            prev_close = quote.get("previousClose")
            if prev_close is None:
                prev_close = quote.get("chartPreviousClose")
            pct_text = _fmt_signed_pct(price, prev_close)
            exchange = str(quote.get("fullExchangeName") or quote.get("exchangeName") or "").strip()
            open_price = quote.get("regularMarketOpen")
            if open_price is None:
                open_price = quote.get("chartPreviousClose")
            high = quote.get("regularMarketDayHigh")
            low = quote.get("regularMarketDayLow")
            market_ts = quote.get("regularMarketTime")
            meta1 = "Open {open}  Prev {prev}".format(
                open=_strip_trailing_zeros(str(open_price)) if open_price is not None else "--",
                prev=_strip_trailing_zeros(str(prev_close)) if prev_close is not None else "--",
            )
            meta2_parts = [
                "H/L {high}/{low}".format(
                    high=_strip_trailing_zeros(str(high)) if high is not None else "--",
                    low=_strip_trailing_zeros(str(low)) if low is not None else "--",
                )
            ]
            if exchange:
                meta2_parts.append(exchange[:18])
            if isinstance(market_ts, (int, float)) and market_ts > 0:
                try:
                    meta2_parts.append(datetime.fromtimestamp(market_ts).strftime("%H:%M"))
                except Exception:
                    pass
            out.append({
                "title": f"{display_name} · {symbol}",
                "price": price_text,
                "pct": pct_text,
                "meta1": meta1,
                "meta2": "  ".join(meta2_parts),
            })
            continue
        reason = "行情获取失败" if error else "暂无行情"
        out.append({
            "title": item.get("name") or symbol or "--",
            "price": "--",
            "pct": "--",
            "meta1": reason[:28],
            "meta2": symbol[:28],
        })
    return {"items": out, "error": error}


def _legacy_fetch_skills_config(user_key=""):
    """GET /v1/skills/config，返回 (all_skills, enabled_set) 或 (None, None) 表示失败。"""
    try:
        raw = localhost_api_request("GET", "/v1/skills/config", user_key)
        body = json.loads(raw.decode())
        data = (body.get("data") or body) or {}
        # 全部技能：managed_skills 或 skills_list + skill_switches 的 key
        all_list = data.get("managed_skills") or data.get("skills_list") or []
        switches = data.get("skill_switches") or {}
        all_names = sorted(set(all_list) | set(switches.keys()))
        # 当前开启的：runtime_enabled_skills
        enabled_list = data.get("runtime_enabled_skills") or data.get("effective_enabled_skills_preview") or []
        enabled_set = set(enabled_list)
        return all_names, enabled_set
    except Exception:
        return None, None


def _single_instance_lock():
    """单实例锁：返回 (lock_fd, None) 成功，已被占用则返回 (None, True) 并建议退出。"""
    try:
        import fcntl
    except ImportError:
        return (None, None)
    lock_path = f"/tmp/rustclaw-small-screen-{os.getuid()}.lock"
    try:
        fd = os.open(lock_path, os.O_CREAT | os.O_RDWR, 0o600)
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        return (fd, None)
    except (OSError, IOError) as e:
        if e.errno in (errno.EAGAIN, errno.EWOULDBLOCK):
            return (None, True)
        return (None, None)
    except Exception:
        return (None, None)


# Ensure the split service implementations are the active bindings.
from small_screen_clawd_client import fetch_skills_config as _service_fetch_skills_config
from small_screen_market_service import (
    _load_small_screen_crypto_config as _service_load_small_screen_crypto_config,
    _load_small_screen_market_config as _service_load_small_screen_market_config,
    _load_small_screen_stock_config as _service_load_small_screen_stock_config,
    _load_small_screen_us_stock_config as _service_load_small_screen_us_stock_config,
    _normalize_stock_code as _service_normalize_stock_code,
    _normalize_us_stock_symbol as _service_normalize_us_stock_symbol,
    _parse_refresh_seconds as _service_parse_refresh_seconds,
    fetch_a_share_quotes as _service_fetch_a_share_quotes,
    fetch_crypto_prices as _service_fetch_crypto_prices,
    fetch_us_stock_quotes as _service_fetch_us_stock_quotes,
)

_load_small_screen_market_config = _service_load_small_screen_market_config
_parse_refresh_seconds = _service_parse_refresh_seconds
_load_small_screen_crypto_config = _service_load_small_screen_crypto_config
fetch_crypto_prices = _service_fetch_crypto_prices
_normalize_stock_code = _service_normalize_stock_code
_load_small_screen_stock_config = _service_load_small_screen_stock_config
_normalize_us_stock_symbol = _service_normalize_us_stock_symbol
_load_small_screen_us_stock_config = _service_load_small_screen_us_stock_config
fetch_a_share_quotes = _service_fetch_a_share_quotes
fetch_us_stock_quotes = _service_fetch_us_stock_quotes
fetch_skills_config = _service_fetch_skills_config


class SmallScreenApp:
    def __init__(self):
        fd, already_running = _single_instance_lock()
        if already_running:
            root = tk.Tk()
            root.withdraw()
            root.after(0, root.destroy)
            try:
                from tkinter import messagebox
                messagebox.showinfo(
                    STRINGS.get(load_lang(), STRINGS["CN"])["app_title"],
                    "小屏监控已在运行，请勿重复启动。\nRustClaw small screen is already running."
                )
            except Exception:
                pass
            try:
                root.destroy()
            except Exception:
                pass
            sys.exit(0)
        self._lock_fd = fd
        if not os.environ.get("DISPLAY"):
            os.environ["DISPLAY"] = ":0"
        try:
            self.root = tk.Tk()
        except tk.TclError as e:
            if "display" in str(e).lower() or "DISPLAY" in str(e) or "no display" in str(e).lower():
                print("无法连接图形显示（无 DISPLAY）。", file=sys.stderr)
                print("请任选其一：", file=sys.stderr)
                print("  1) 在树莓派本机桌面/HDMI 下运行： export DISPLAY=:0 后再启动", file=sys.stderr)
                print("  2) 无桌面时用网页版小屏： cd pi_app && ./open-small-screen.sh", file=sys.stderr)
            else:
                print(f"Tk 初始化失败: {e}", file=sys.stderr)
            sys.exit(1)
        self._lang = load_lang()
        self._theme = load_theme()
        self._show_messages_page = load_messages_page_visible()
        self._show_logs_page = load_logs_page_visible()
        self._show_gallery_page = load_gallery_page_visible()
        self._show_skills_page = load_skills_page_visible()
        self._show_weather_page = load_weather_page_visible()
        self._show_stock_page = load_stock_page_visible()
        self._show_us_stock_page = load_us_stock_page_visible()
        self._show_crypto_page = load_crypto_page_visible()
        self._auth_key = ensure_small_screen_auth_key()
        self._ui_queue = queue.SimpleQueue()
        self._ui_pump_job = None
        self._refresh_thread = None
        self._time_job = None
        self._gif_job = None
        self._after_splash_job = None
        self._clear_topmost_job = None
        self._raise_window_job = None
        self._settings_restart_job = None
        self._i18n = []  # [(widget, key), ...] 用于切换语言时更新
        self.root.title(STRINGS.get(self._lang, STRINGS["CN"])["app_title"])
        self.root.geometry(f"{W}x{H}")
        self.root.resizable(False, False)
        self.root.configure(bg=self._c("bg"))
        self._start_ui_pump()
        self.health = None
        self.log_summary = []
        self.log_entries = []
        self.user_messages = []
        self._last_user_messages_signature = None
        self._last_logs_signature = None
        self._last_overview_render_signature = None
        self._log_entry_limit = 24
        self._pending_log_entries = []
        self._log_append_job = None
        self._logs_empty_label = None
        self._logs_list_wrapper = None
        self._logs_canvas = None
        self._logs_inner = None
        self._logs_canvas_window_id = None
        self._logs_rows = []
        self.error = None
        self._wifi_networks = []
        self._wifi_scan_error = None
        self._wifi_status_text = ""
        self._wifi_selected_ssid = ""
        self._wifi_selected_security = ""
        self._wifi_password_var = tk.StringVar(value="")
        self._wifi_password_visible = False
        self._wifi_keyboard_mode = "lower"
        self._wifi_page_index = 0
        self._wifi_scan_in_progress = False
        self._wifi_connect_in_progress = False
        self._wifi_disconnect_in_progress = False
        self._wifi_keyboard_window = None
        self._llm_pubkey_hex = ""
        self._llm_pubkey_error = ""
        self._llm_pubkey_loading = False
        self._llm_signing = False
        self._llm_signature_hex = ""
        self._llm_signature_error = ""
        self._llm_signature_timestamp = ""
        self._llm_info_hidden = True
        self._llm_clear_job = None
        self._llm_info_frame = None
        self._llm_info_pady = (0, 8)
        self._llm_join_in_progress = False
        self._llm_content = None
        self._llm_dot_labels = []
        self._llm_lobster_count = 0
        self._llm_lobster_photo = None
        self._llm_matrix_cols = []
        self._llm_matrix_max_rows = 0
        self._weather_data = None
        self._weather_error = ""
        self._weather_loading = False
        self._weather_selected_offset = 0
        self._weather_refresh_job = None
        self._weather_manual_refresh_job = None
        self._overview_return_on_swipe = False
        self._weather_return_view_on_swipe = None
        self._overview_tap_press_x = 0
        self._overview_tap_press_y = 0
        self._overview_last_tap_at = 0.0
        self._overview_last_tap_mode = None
        self._overview_last_tap_x = 0
        self._overview_last_tap_y = 0
        self._overview_scroll_job = None
        self._overview_stock_scroll_idx = 0
        self._overview_us_stock_scroll_idx = 0
        self._overview_crypto_scroll_idx = 0
        self._overview_skills_loading = False
        self._overview_skills_updated_at = 0.0
        self._overview_skills_summary = None
        self._overview_stock_loading = False
        self._overview_stock_updated_at = 0.0
        self._overview_stock_summary = None
        self._overview_us_stock_loading = False
        self._overview_us_stock_updated_at = 0.0
        self._overview_us_stock_summary = None
        self._overview_crypto_loading = False
        self._overview_crypto_updated_at = 0.0
        self._overview_crypto_summary = None
        self.gif_frames = []
        self.gif_delays = []
        self.gif_frame_idx = 0
        self._closing = False
        splash_path = find_splash_image()
        if not splash_path:
            splash_images = list_gallery_images()
            splash_path = splash_images[0] if splash_images else None
        if splash_path:
            self._show_splash(splash_path)
            self._start_fullscreen()
            self.root.protocol("WM_DELETE_WINDOW", self._on_close)
            self._after_splash_job = self.root.after(2000, self._after_splash)
        else:
            self._build_ui()
            self._schedule_refresh()
            self._start_weather_refresh_cycle()
            self._start_fullscreen()
            self.root.protocol("WM_DELETE_WINDOW", self._on_close)
            self._tick_time()
            if self.gif_frames:
                self._animate_gif()

    def _show_splash(self, image_path):
        """启动等待界面：全屏显示图片（优先脚本目录下 RustClaw480X320.png）。"""
        self._splash_frame = tk.Frame(self.root, bg=self._c("bg"))
        self._splash_frame.pack(fill=tk.BOTH, expand=True)
        self._splash_photo = None
        try:
            from PIL import Image, ImageTk
            with Image.open(image_path) as img:
                splash = img.convert("RGB")
                # 全屏：缩放到窗口大小 W×H 填满
                splash = splash.resize((W, H), Image.Resampling.LANCZOS)
                self._splash_photo = ImageTk.PhotoImage(splash)
        except Exception:
            try:
                self._splash_photo = tk.PhotoImage(file=image_path)
            except Exception:
                pass
        if self._splash_photo:
            lbl = tk.Label(self._splash_frame, image=self._splash_photo, bg=self._c("bg"))
            lbl.place(x=0, y=0, relwidth=1, relheight=1)
        else:
            tk.Label(self._splash_frame, text="RustClaw", font=("", 24, "bold"), bg=self._c("bg"), fg=self._c("accent")).place(relx=0.5, rely=0.5, anchor=tk.CENTER)

    def _start_ui_pump(self):
        if getattr(self, "_closing", False):
            return
        try:
            self._ui_pump_job = self.root.after(100, self._drain_ui_queue)
        except tk.TclError:
            self._ui_pump_job = None

    def _post_ui(self, callback):
        if getattr(self, "_closing", False):
            return
        self._ui_queue.put(callback)

    def _cancel_job(self, attr):
        job = getattr(self, attr, None)
        if job is None:
            return
        try:
            self.root.after_cancel(job)
        except tk.TclError:
            pass
        setattr(self, attr, None)

    def _stop_market_jobs(self):
        self._cancel_job("_crypto_job")
        self._cancel_job("_stock_job")
        self._cancel_job("_us_stock_job")

    def _teardown_gallery_view(self):
        self._cancel_job("_gallery_job")
        self._cancel_llm_clear_job()
        self._stop_llm_animation()

    def _teardown_current_view(self):
        mode = getattr(self, "_view_mode", None)
        if mode in {"crypto", "stock", "us_stock"}:
            self._stop_market_jobs()
        elif mode == "overview":
            self._cancel_job("_overview_scroll_job")
        elif mode == "gallery":
            self._teardown_gallery_view()

    def _prepare_for_ui_rebuild(self):
        self._teardown_current_view()
        self._teardown_gallery_view()
        self._stop_market_jobs()
        self._cancel_log_append_job()
        self._cancel_llm_clear_job()
        for attr in ("_blink_job", "_gif_job", "_time_job", "_after_splash_job", "_clear_topmost_job", "_raise_window_job", "_settings_restart_job", "_weather_refresh_job", "_weather_manual_refresh_job", "_overview_scroll_job"):
            self._cancel_job(attr)

    def _drain_ui_queue(self):
        self._ui_pump_job = None
        if getattr(self, "_closing", False):
            return
        try:
            while True:
                callback = self._ui_queue.get_nowait()
                try:
                    callback()
                except tk.TclError:
                    if getattr(self, "_closing", False):
                        return
                except Exception:
                    logger.exception("UI callback failed")
        except queue.Empty:
            pass
        self._start_ui_pump()

    def _after_splash(self):
        """等待界面结束后构建主界面。"""
        self._after_splash_job = None
        if getattr(self, "_closing", False):
            return
        if hasattr(self, "_splash_frame") and self._splash_frame.winfo_exists():
            self._splash_frame.destroy()
        self._build_ui()
        self._schedule_refresh()
        self._start_weather_refresh_cycle()
        self._tick_time()
        if self.gif_frames:
            self._animate_gif()
        self._cancel_job("_raise_window_job")
        self._raise_window_job = self.root.after(200, self._raise_window)

    def _build_ui(self):
        global ASSETS_DIR
        ASSETS_DIR = find_assets()
        # 顶栏：左侧龙虾或占位 + 标题 + 状态
        top = tk.Frame(self.root, bg=self._c("bg"), height=56)
        top.pack(fill=tk.X, padx=6, pady=4)
        top.pack_propagate(False)
        # 左侧 48x48：龙虾动图（lobster.gif）或 🦞 占位
        gif_path = os.path.join(ASSETS_DIR, "lobster.gif")
        self.lobster_label = tk.Label(top, bg=self._c("bg"))
        self.lobster_label.pack(side=tk.LEFT, padx=(0, 6))
        try:
            if os.path.isfile(gif_path):
                try:
                    from PIL import Image, ImageTk
                    with Image.open(gif_path) as img:
                        try:
                            n = 0
                            while True:
                                img.seek(n)
                                frame = img.copy().convert("RGBA")
                                self.gif_frames.append(ImageTk.PhotoImage(frame.resize((48, 48), Image.Resampling.LANCZOS)))
                                delay = img.info.get("duration", 100)
                                self.gif_delays.append(max(50, int(delay)))
                                n += 1
                        except EOFError:
                            pass
                    if self.gif_frames:
                        self.lobster_label.configure(image=self.gif_frames[0])
                    else:
                        self.lobster_label.configure(text="🦞", font=("", 28), fg=self._c("fg"))
                except ImportError:
                    self.photo = tk.PhotoImage(file=gif_path)
                    self.lobster_label.configure(image=self.photo)
            else:
                self.lobster_label.configure(text="🦞", font=("", 28), fg=self._c("fg"))
        except Exception:
            self.lobster_label.configure(text="🦞", font=("", 28), fg=self._c("fg"))
        # 标题 RustClaw
        top_text = tk.Frame(top, bg=self._c("bg"), bd=0, highlightthickness=0)
        top_text.pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 8))
        self._top_title_label = tk.Label(
            top_text, text="RustClaw", font=("", 20, "bold"),
            bg=self._c("bg"), fg=self._c("accent"), anchor="w",
            bd=0, relief=tk.FLAT, highlightthickness=0
        )
        self._top_title_label.pack(anchor=tk.W)
        self._top_recent_message_var = tk.StringVar(value="")
        self._top_recent_message_label = tk.Label(
            top_text,
            textvariable=self._top_recent_message_var,
            font=("", 10),
            bg=self._c("bg"),
            fg=self._c("fg_dim"),
            anchor="w",
            justify=tk.LEFT,
        )
        self._top_recent_message_label.pack(fill=tk.X, anchor=tk.W)
        # 右侧：天气图标 + 简短天气说明 + 当前时间（左） + 状态在线/离线（右）
        self._top_weather_icon_var = tk.StringVar(value="◌")
        self._top_weather_text_var = tk.StringVar(value="--")
        self.time_var = tk.StringVar(value="--:--:--")
        right_frame = tk.Frame(top, bg=self._c("bg"))
        right_frame.pack(side=tk.RIGHT)
        self._top_weather_icon_label = tk.Label(
            right_frame,
            textvariable=self._top_weather_icon_var,
            font=("DejaVu Sans", 20),
            bg=self._c("bg"),
            fg=self._c("accent"),
            width=2,
            anchor="e",
        )
        self._top_weather_icon_label.pack(side=tk.LEFT, padx=(0, 6))
        self._top_weather_icon_label.bind("<Button-1>", self._open_weather_from_topbar)
        self._top_weather_text_label = tk.Label(
            right_frame,
            textvariable=self._top_weather_text_var,
            font=("", 10),
            bg=self._c("bg"),
            fg=self._c("fg_dim"),
            anchor="e",
            justify=tk.RIGHT,
            width=16,
        )
        self._top_weather_text_label.pack(side=tk.LEFT, padx=(0, 8))
        self._top_weather_text_label.bind("<Button-1>", self._open_weather_from_topbar)
        tk.Label(
            right_frame, textvariable=self.time_var, font=("", 12),
            bg=self._c("bg"), fg=self._c("fg_dim")
        ).pack(side=tk.LEFT, padx=(0, 10))
        # 状态：在线=绿色圆圈闪烁，离线=红色圆圈不闪
        self._online = False
        self._blink_job = None
        self.status_canvas = tk.Canvas(right_frame, width=16, height=16, bg=self._c("bg"), highlightthickness=0)
        self.status_canvas.pack(side=tk.LEFT)
        self.status_oval = self.status_canvas.create_oval(2, 2, 14, 14, outline=self._c("status_outline"), fill=self._c("status_off"))
        # 可切换内容区：仪表盘 | 技能列表
        self.switch_container = tk.Frame(self.root, bg=self._c("bg"))
        self.switch_container.pack(fill=tk.BOTH, expand=True)
        self.dashboard_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=8, pady=4)
        self.dashboard_frame.pack(fill=tk.BOTH, expand=True)
        self.overview_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=8, pady=4)
        self.skills_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.gallery_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.weather_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.crypto_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.stock_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.us_stock_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.wifi_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=12, pady=8)
        self.users_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=20, pady=18)
        self.logs_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=10, pady=8)
        self.settings_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=24, pady=20)
        # 顺序（左滑下一页）：首页 → 总览 → 用户 → 日志 → 技能 → 天气 → A股 → 加密货币 → 挖矿 → 设置；右滑=上一页
        self._view_mode = "dashboard"  # dashboard | overview | users | logs | skills | weather | stock | crypto | gallery | wifi | settings
        self._crypto_job = None
        self._stock_job = None
        self._us_stock_job = None
        self._gallery_images = []
        self._gallery_index = 0
        self._gallery_photos = []
        self._gallery_job = None
        self._llm_lobster_job = None
        content = self.dashboard_frame
        # 内容区：一行 2 个信息，每项固定大小方框
        box_bg = self._c("box_bg")
        box_border = self._c("box_border")
        label_font = ("", 13)
        value_font = ("", 14, "bold")
        cell_gap = 6
        # 固定方框宽高（两列等大）
        cell_w = (W - 16 - cell_gap) // 2
        cell_h = 40

        def _t(key):
            return STRINGS.get(self._lang, STRINGS["CN"]).get(key, key)

        def cell(parent, label_key, var, right_gap=True):
            f = tk.Frame(parent, bg=box_border, padx=2, pady=2, width=cell_w, height=cell_h)
            f.pack_propagate(False)
            f.pack(side=tk.LEFT, padx=(0, cell_gap if right_gap else 0))
            inner = tk.Frame(f, bg=box_bg, padx=6, pady=4)
            inner.pack(fill=tk.BOTH, expand=True)
            lbl = tk.Label(inner, text=_t(label_key) + " ", font=label_font, bg=box_bg, fg=self._c("fg_dim"))
            lbl.pack(side=tk.LEFT)
            self._i18n.append((lbl, label_key))
            tk.Label(inner, textvariable=var, font=value_font, bg=box_bg, fg=self._c("fg")).pack(side=tk.RIGHT)

        def row2(parent, l_key, l_var, r_key, r_var):
            rw = tk.Frame(parent, bg=self._c("bg"))
            rw.pack(fill=tk.X, pady=3)
            cell(rw, l_key, l_var, right_gap=True)
            cell(rw, r_key, r_var, right_gap=False)

        self.ver_var = tk.StringVar(value="--")
        self.uptime_var = tk.StringVar(value="--")
        self.queue_var = tk.StringVar(value="--")
        self.running_var = tk.StringVar(value="--")
        self.worker_var = tk.StringVar(value="--")
        self.rss_var = tk.StringVar(value="--")
        self.adapters_var = tk.StringVar(value="--")
        self.adapters_rss_var = tk.StringVar(value="--")
        row2(content, "version", self.ver_var, "uptime", self.uptime_var)
        row2(content, "queue", self.queue_var, "running", self.running_var)
        row2(content, "worker", self.worker_var, "memory_rss", self.rss_var)
        adapters_font = ("", 12, "bold")
        adapters_row = tk.Frame(content, bg=self._c("bg"))
        adapters_row.pack(fill=tk.X, pady=4)
        a1 = tk.Label(adapters_row, text=_t("adapters") + " ", font=adapters_font, bg=self._c("bg"), fg=self._c("adapters_fg"))
        a1.pack(side=tk.LEFT)
        self._i18n.append((a1, "adapters"))
        self._adapters_value_label = tk.Label(
            adapters_row,
            textvariable=self.adapters_var,
            font=adapters_font,
            bg=self._c("bg"),
            fg=self._c("adapters_value_fg"),
            justify=tk.LEFT,
            anchor=tk.W,
            wraplength=380,
        )
        self._adapters_value_label.pack(side=tk.LEFT, fill=tk.X, expand=True)
        self.users_count_var = tk.StringVar(value="--")
        self.bound_channels_var = tk.StringVar(value="--")
        self._dashboard_summary_row = tk.Frame(content, bg=self._c("bg"))
        self._dashboard_summary_row.pack(fill=tk.X, pady=(0, 4))
        self._dashboard_users_label = tk.Label(self._dashboard_summary_row, text=_t("users_count") + ": ", font=("", 10), bg=self._c("bg"), fg=self._c("fg_dim"))
        self._dashboard_users_label.pack(side=tk.LEFT)
        self._dashboard_users_value = tk.Label(self._dashboard_summary_row, textvariable=self.users_count_var, font=("", 12, "bold"), bg=self._c("bg"), fg=self._c("fg"))
        self._dashboard_users_value.pack(side=tk.LEFT)
        self._dashboard_channels_label = tk.Label(self._dashboard_summary_row, text="    " + _t("bound_channels") + ": ", font=("", 10), bg=self._c("bg"), fg=self._c("fg_dim"))
        self._dashboard_channels_label.pack(side=tk.LEFT)
        self._dashboard_channels_value = tk.Label(self._dashboard_summary_row, textvariable=self.bound_channels_var, font=("", 12, "bold"), bg=self._c("bg"), fg=self._c("adapters_value_fg"))
        self._dashboard_channels_value.pack(side=tk.LEFT)
        self.foot_var = tk.StringVar(value=_t("foot_prefix"))
        tk.Label(content, textvariable=self.foot_var, font=("", 11), bg=self._c("bg"), fg=self._c("foot_fg")).pack(anchor=tk.W)
        self.clawd_summary_var = tk.StringVar(value=_t("clawd_summary_empty"))
        self._overview_body = tk.Frame(self.overview_frame, bg=self._c("bg"))
        self._overview_body.pack(fill=tk.BOTH, expand=True)
        self._overview_us_stock_icon_var = tk.StringVar(value="◌")
        self._overview_us_stock_main_var = tk.StringVar(value="--")
        self._overview_us_stock_meta_var = tk.StringVar(value="")
        self._overview_us_stock_detail_var = tk.StringVar(value="")
        self._overview_stock_title_var = tk.StringVar(value=self._t("show_stock_page"))
        self._overview_stock_value_var = tk.StringVar(value="--")
        self._overview_stock_meta_var = tk.StringVar(value="")
        self._overview_crypto_title_var = tk.StringVar(value="Crypto")
        self._overview_crypto_value_var = tk.StringVar(value="-- USDT")
        self._overview_crypto_meta_var = tk.StringVar(value="")
        self._overview_dashboard_value_var = tk.StringVar(value="--")
        self._overview_dashboard_meta_var = tk.StringVar(value="")
        self._build_overview_layout()
        self._render_dashboard_overview()
        self._users_body = tk.Frame(self.users_frame, bg=self._c("bg"))
        self._users_body.pack(fill=tk.BOTH, expand=True)
        self._users_messages_body = tk.Frame(self._users_body, bg=self._c("bg"))
        self._users_messages_body.pack(fill=tk.BOTH, expand=True)
        self._logs_body = tk.Frame(self.logs_frame, bg=self._c("bg"))
        self._logs_body.pack(fill=tk.BOTH, expand=True)
        # 翻页：左右滑屏可到仪表盘 / 技能 / 加密货币 / 图库 / 用户 / 设置
        # 设置页（内嵌在主窗口，左滑可进入）
        self._settings_lang_var = tk.StringVar(value=self._lang)
        self._settings_theme_var = tk.StringVar(value=self._theme)
        self._settings_show_messages_var = tk.BooleanVar(value=self._show_messages_page)
        self._settings_show_logs_var = tk.BooleanVar(value=self._show_logs_page)
        self._settings_show_gallery_var = tk.BooleanVar(value=self._show_gallery_page)
        self._settings_show_skills_var = tk.BooleanVar(value=self._show_skills_page)
        self._settings_show_weather_var = tk.BooleanVar(value=self._show_weather_page)
        self._settings_show_stock_var = tk.BooleanVar(value=self._show_stock_page)
        self._settings_show_us_stock_var = tk.BooleanVar(value=self._show_us_stock_page)
        self._settings_show_crypto_var = tk.BooleanVar(value=self._show_crypto_page)
        self._settings_category = "menu"
        self._settings_header_row = tk.Frame(self.settings_frame, bg=self._c("bg"))
        self._settings_header_title_var = tk.StringVar(value=_t("settings_title"))
        self._settings_header_title_label = tk.Label(
            self._settings_header_row,
            textvariable=self._settings_header_title_var,
            font=("", 14, "bold"),
            bg=self._c("bg"),
            fg=self._c("fg"),
        )
        self._settings_back_btn = tk.Button(
            self._settings_header_row,
            text=_t("back"),
            font=("", 10),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
            command=self._show_settings_menu,
        )
        self._settings_content_frame = tk.Frame(self.settings_frame, bg=self._c("bg"))
        self._settings_content_frame.pack(fill=tk.BOTH, expand=True)
        self._settings_menu_frame = tk.Frame(self._settings_content_frame, bg=self._c("bg"))
        self._settings_menu_language_btn = tk.Button(
            self._settings_menu_frame,
            font=("", 13, "bold"),
            relief=tk.FLAT,
            anchor="w",
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("bg"),
            fg=self._c("accent"),
            activebackground=self._c("bg"),
            activeforeground=self._c("fg"),
            command=lambda: self._show_settings_category("language"),
            padx=2,
            pady=6,
        )
        self._settings_menu_language_btn.pack(fill=tk.X, pady=(0, 8))
        self._settings_menu_theme_btn = tk.Button(
            self._settings_menu_frame,
            font=("", 13, "bold"),
            relief=tk.FLAT,
            anchor="w",
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("bg"),
            fg=self._c("accent"),
            activebackground=self._c("bg"),
            activeforeground=self._c("fg"),
            command=lambda: self._show_settings_category("theme"),
            padx=2,
            pady=6,
        )
        self._settings_menu_theme_btn.pack(fill=tk.X, pady=(0, 8))
        self._settings_menu_pages_btn = tk.Button(
            self._settings_menu_frame,
            font=("", 13, "bold"),
            relief=tk.FLAT,
            anchor="w",
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("bg"),
            fg=self._c("accent"),
            activebackground=self._c("bg"),
            activeforeground=self._c("fg"),
            command=lambda: self._show_settings_category("pages"),
            padx=2,
            pady=6,
        )
        self._settings_menu_pages_btn.pack(fill=tk.X, pady=(0, 8))
        self._settings_menu_system_btn = tk.Button(
            self._settings_menu_frame,
            font=("", 13, "bold"),
            relief=tk.FLAT,
            anchor="w",
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("bg"),
            fg=self._c("accent"),
            activebackground=self._c("bg"),
            activeforeground=self._c("fg"),
            command=lambda: self._show_settings_category("system"),
            padx=2,
            pady=6,
        )
        self._settings_menu_system_btn.pack(fill=tk.X)
        self._settings_language_frame = tk.Frame(self._settings_content_frame, bg=self._c("bg"))
        self._settings_lang_en_btn = tk.Radiobutton(
            self._settings_language_frame,
            text="EN",
            variable=self._settings_lang_var,
            value="EN",
            command=lambda: self._apply_settings_changes("language"),
            font=("", 12),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_lang_en_btn.pack(anchor=tk.W, pady=(4, 10), fill=tk.X)
        self._settings_lang_cn_btn = tk.Radiobutton(
            self._settings_language_frame,
            text="CN",
            variable=self._settings_lang_var,
            value="CN",
            command=lambda: self._apply_settings_changes("language"),
            font=("", 12),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_lang_cn_btn.pack(anchor=tk.W, fill=tk.X)
        self._settings_theme_frame = tk.Frame(self._settings_content_frame, bg=self._c("bg"))
        self._settings_theme_default_btn = tk.Radiobutton(
            self._settings_theme_frame,
            text=_t("theme_default"),
            variable=self._settings_theme_var,
            value="default",
            command=lambda: self._apply_settings_changes("theme"),
            font=("", 12),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_theme_default_btn.pack(anchor=tk.W, pady=(4, 10), fill=tk.X)
        self._settings_theme_matrix_btn = tk.Radiobutton(
            self._settings_theme_frame,
            text=_t("theme_matrix"),
            variable=self._settings_theme_var,
            value="matrix",
            command=lambda: self._apply_settings_changes("theme"),
            font=("", 12),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_theme_matrix_btn.pack(anchor=tk.W, fill=tk.X)
        self._settings_pages_frame = tk.Frame(self._settings_content_frame, bg=self._c("bg"))
        self._settings_pages_row1 = tk.Frame(self._settings_pages_frame, bg=self._c("bg"))
        self._settings_pages_row1.pack(fill=tk.X, pady=(4, 10))
        self._settings_pages_row2 = tk.Frame(self._settings_pages_frame, bg=self._c("bg"))
        self._settings_pages_row2.pack(fill=tk.X, pady=(0, 10))
        self._settings_pages_row3 = tk.Frame(self._settings_pages_frame, bg=self._c("bg"))
        self._settings_pages_row3.pack(fill=tk.X, pady=(0, 10))
        self._settings_pages_row4 = tk.Frame(self._settings_pages_frame, bg=self._c("bg"))
        self._settings_pages_row4.pack(fill=tk.X, pady=(0, 10))
        self._settings_pages_row5 = tk.Frame(self._settings_pages_frame, bg=self._c("bg"))
        self._settings_pages_row5.pack(fill=tk.X)
        for row in (
            self._settings_pages_row1,
            self._settings_pages_row2,
            self._settings_pages_row3,
            self._settings_pages_row4,
            self._settings_pages_row5,
        ):
            row.grid_columnconfigure(0, weight=1, uniform="settings-pages")
            row.grid_columnconfigure(1, weight=1, uniform="settings-pages")
        self._settings_show_messages_btn = tk.Checkbutton(
            self._settings_pages_row1,
            text=_t("show_messages_page"),
            variable=self._settings_show_messages_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_messages_btn.grid(row=0, column=0, sticky="ew", padx=(0, 6))
        self._settings_show_logs_btn = tk.Checkbutton(
            self._settings_pages_row1,
            text=_t("show_logs_page"),
            variable=self._settings_show_logs_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_logs_btn.grid(row=0, column=1, sticky="ew", padx=(6, 0))
        self._settings_show_skills_btn = tk.Checkbutton(
            self._settings_pages_row2,
            text=_t("show_skills_page"),
            variable=self._settings_show_skills_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_skills_btn.grid(row=0, column=0, sticky="ew", padx=(0, 6))
        self._settings_show_gallery_btn = tk.Checkbutton(
            self._settings_pages_row2,
            text=_t("show_nni_page"),
            variable=self._settings_show_gallery_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_gallery_btn.grid(row=0, column=1, sticky="ew", padx=(6, 0))
        self._settings_show_weather_btn = tk.Checkbutton(
            self._settings_pages_row3,
            text=_t("show_weather_page"),
            variable=self._settings_show_weather_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_weather_btn.grid(row=0, column=0, sticky="ew", padx=(0, 6))
        self._settings_show_stock_btn = tk.Checkbutton(
            self._settings_pages_row3,
            text=_t("show_stock_page"),
            variable=self._settings_show_stock_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_stock_btn.grid(row=0, column=1, sticky="ew", padx=(6, 0))
        self._settings_show_crypto_btn = tk.Checkbutton(
            self._settings_pages_row4,
            text=_t("show_crypto_page"),
            variable=self._settings_show_crypto_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_crypto_btn.grid(row=0, column=0, sticky="ew", padx=(0, 6))
        self._settings_show_us_stock_btn = tk.Checkbutton(
            self._settings_pages_row4,
            text=_t("show_us_stock_page"),
            variable=self._settings_show_us_stock_var,
            onvalue=True,
            offvalue=False,
            command=lambda: self._apply_settings_changes("pages"),
            font=("", 12),
            indicatoron=False,
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            selectcolor=self._c("button_bg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            anchor="w",
            padx=10,
            pady=6,
        )
        self._settings_show_us_stock_btn.grid(row=0, column=1, sticky="ew", padx=(6, 0))
        self._settings_pages_row5_spacer = tk.Frame(self._settings_pages_row5, bg=self._c("bg"))
        self._settings_pages_row5_spacer.grid(row=0, column=1, sticky="ew", padx=(6, 0))
        self._settings_system_frame = tk.Frame(self._settings_content_frame, bg=self._c("bg"))
        self._settings_wifi_btn = tk.Button(
            self._settings_system_frame,
            text=_t("wifi_title"),
            font=("", 11),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            takefocus=0,
            command=self._open_wifi_from_settings,
        )
        self._settings_wifi_btn.pack(fill=tk.X, pady=(4, 8))
        self._settings_restart_btn = tk.Button(
            self._settings_system_frame,
            text=_t("restart"),
            font=("", 11),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            disabledforeground=self._c("fg_dim"),
            takefocus=0,
            command=self._on_settings_restart,
        )
        self._settings_restart_btn.pack(fill=tk.X, pady=(0, 8))
        bf2 = tk.Frame(self._settings_system_frame, bg=self._c("bg"))
        bf2.pack(fill=tk.X)
        self._settings_reset_admin_btn = tk.Button(
            bf2,
            text=_t("reset_admin_login"),
            font=("", 11),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("button_fg"),
            disabledforeground=self._c("fg_dim"),
            takefocus=0,
            command=self._on_settings_reset_admin_login,
        )
        self._settings_reset_admin_btn.pack(fill=tk.X)
        self._settings_reset_status_var = tk.StringVar(value="")
        self._settings_reset_status_label = tk.Label(
            self._settings_system_frame,
            textvariable=self._settings_reset_status_var,
            font=("", 10),
            bg=self._c("bg"),
            fg=self._c("fg_dim"),
            anchor="w",
            justify=tk.LEFT,
            wraplength=440,
        )
        self._settings_reset_status_label.pack(anchor=tk.W, pady=(8, 0))
        self._show_settings_menu()
        self._wifi_status_var = tk.StringVar(value=_t("wifi_scan_hint"))
        self._wifi_status_label = tk.Label(
            self.wifi_frame,
            textvariable=self._wifi_status_var,
            font=("", 10),
            bg=self._c("bg"),
            fg=self._c("fg_dim"),
            anchor="w",
            justify=tk.LEFT,
            wraplength=450,
        )
        self._wifi_status_label.pack(fill=tk.X, pady=(0, 4))
        self._wifi_list_frame = tk.Frame(self.wifi_frame, bg=self._c("bg"))
        self._wifi_list_frame.pack(fill=tk.X)
        self._wifi_pager_row = tk.Frame(self.wifi_frame, bg=self._c("bg"))
        self._wifi_pager_row.pack(fill=tk.X, pady=(4, 4))
        self._wifi_prev_btn = tk.Button(
            self._wifi_pager_row,
            text=_t("wifi_prev_page"),
            font=("", 10),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
            disabledforeground=self._c("fg_dim"),
            takefocus=0,
            command=lambda: self._change_wifi_page(-1),
        )
        self._wifi_prev_btn.pack(side=tk.LEFT)
        self._wifi_page_var = tk.StringVar(value="1/1")
        self._wifi_page_label = tk.Label(
            self._wifi_pager_row,
            textvariable=self._wifi_page_var,
            font=("", 10),
            bg=self._c("bg"),
            fg=self._c("fg_dim"),
        )
        self._wifi_page_label.pack(side=tk.LEFT, padx=10)
        self._wifi_next_btn = tk.Button(
            self._wifi_pager_row,
            text=_t("wifi_next_page"),
            font=("", 10),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
            disabledforeground=self._c("fg_dim"),
            takefocus=0,
            command=lambda: self._change_wifi_page(1),
        )
        self._wifi_next_btn.pack(side=tk.LEFT, padx=(0, 8))
        self._wifi_right_actions = tk.Frame(self._wifi_pager_row, bg=self._c("bg"))
        self._wifi_right_actions.pack(side=tk.RIGHT)
        self._wifi_refresh_btn = tk.Button(
            self._wifi_right_actions,
            text=_t("wifi_refresh"),
            font=("", 10),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
            disabledforeground=self._c("fg_dim"),
            takefocus=0,
            command=self._refresh_wifi_networks,
        )
        self._wifi_refresh_btn.pack(side=tk.LEFT)
        self._wifi_back_btn = tk.Button(
            self._wifi_right_actions,
            text=_t("back"),
            font=("", 10),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
            takefocus=0,
            command=self._close_wifi_to_settings,
        )
        self._wifi_back_btn.pack(side=tk.LEFT, padx=(8, 0))
        self._wifi_join_btn = tk.Button(
            self._wifi_right_actions,
            text=_t("wifi_join"),
            font=("", 10),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
            disabledforeground=self._c("fg_dim"),
            takefocus=0,
            command=self._on_wifi_join_click,
        )
        self._wifi_selected_var = tk.StringVar(value="--")
        self._wifi_selected_label = tk.Label(
            self.wifi_frame,
            textvariable=self._wifi_selected_var,
            font=("", 11, "bold"),
            bg=self._c("bg"),
            fg=self._c("accent"),
            anchor="w",
            justify=tk.LEFT,
            wraplength=450,
        )
        self._wifi_selected_label.pack(fill=tk.X, pady=(2, 2))
        self._wifi_hint_var = tk.StringVar(value="")
        self._wifi_hint_label = tk.Label(
            self.wifi_frame,
            textvariable=self._wifi_hint_var,
            font=("", 10),
            bg=self._c("bg"),
            fg=self._c("fg_dim"),
            anchor="w",
            justify=tk.LEFT,
            wraplength=450,
        )
        self._wifi_hint_label.pack(fill=tk.X, pady=(0, 4))
        self._refresh_topbar()
        self._render_wifi_view()

    def _t(self, key):
        return STRINGS.get(self._lang, STRINGS["CN"]).get(key, key)

    def _is_valid_tk_color(self, value):
        if not isinstance(value, str):
            return False
        color = value.strip()
        if not color:
            return False
        if re.fullmatch(r"#[0-9a-fA-F]{3}([0-9a-fA-F]{3})?", color):
            return True
        try:
            self.root.winfo_rgb(color)
            return True
        except Exception:
            return False

    def _c(self, key):
        theme = THEMES.get(self._theme) or THEMES["default"]
        if key == "bg_rgb":
            bg_rgb = theme.get("bg_rgb")
            if isinstance(bg_rgb, (list, tuple)) and len(bg_rgb) >= 3:
                try:
                    return tuple(int(x) for x in bg_rgb[:3])
                except Exception:
                    pass
            return THEMES["default"]["bg_rgb"]

        value = theme.get(key)
        if self._is_valid_tk_color(value):
            s = (value or "").strip()
            return s if s else "#e8e6e3"
        fallback = THEMES["default"].get(key)
        if self._is_valid_tk_color(fallback):
            s = (fallback or "").strip()
            return s if s else "#e8e6e3"

        theme_fg = theme.get("fg")
        if self._is_valid_tk_color(theme_fg):
            s = (theme_fg or "").strip()
            return s if s else "#e8e6e3"
        default_fg = THEMES["default"].get("fg", "#e8e6e3")
        if self._is_valid_tk_color(default_fg):
            s = (default_fg or "").strip()
            return s if s else "#e8e6e3"
        return "#e8e6e3"

    def _tk_color(self, key):
        """与 _c 相同，但保证返回非空字符串，避免 Tk 报 unknown color name \"\"。"""
        out = self._c(key)
        if out is None or (isinstance(out, str) and not out.strip()):
            return "#e8e6e3"
        if isinstance(out, str) and self._is_valid_tk_color(out):
            return out.strip()
        return "#e8e6e3"

    def _safe_color(self, *candidates, fallback_key="fg"):
        """返回可用于 Tk 的颜色字符串，保证非空，避免 unknown color name \"\"。"""
        theme = THEMES.get(self._theme) or THEMES["default"]
        default_theme = THEMES["default"]
        for candidate in candidates:
            value = None
            if isinstance(candidate, str):
                key = candidate.strip()
                if not key:
                    continue
                if key in theme or key in default_theme:
                    value = self._c(key)
                else:
                    value = key
            else:
                value = candidate
            if value is not None and self._is_valid_tk_color(value):
                s = str(value).strip()
                if s:
                    return s
        out = self._c(fallback_key)
        s = (str(out).strip() if out is not None else "") or "#e8e6e3"
        return s if s else "#e8e6e3"

    def _style_scrollbar(self, scrollbar):
        """为右侧滚动条套用主题色，避免默认灰色过于突兀。"""
        try:
            scrollbar.configure(
                bg=self._safe_color("button_bg", "accent"),
                activebackground=self._safe_color("button_active_bg", "accent"),
                troughcolor=self._safe_color("box_bg", "bg"),
                highlightbackground=self._safe_color("bg"),
                highlightcolor=self._safe_color("accent", "fg"),
                relief=tk.FLAT,
                activerelief=tk.FLAT,
                borderwidth=0,
                elementborderwidth=1,
                width=12,
            )
        except tk.TclError:
            pass

    def _apply_lang(self):
        self.root.title(self._t("app_title"))
        for w, k in self._i18n:
            try:
                if k in ("version", "uptime", "queue", "running", "worker", "memory_rss"):
                    w.config(text=self._t(k) + " ")
                else:
                    w.config(text=self._t(k))
            except tk.TclError:
                pass
        try:
            self._dashboard_users_label.config(text=self._t("users_count") + ": ")
            self._dashboard_channels_label.config(text="    " + self._t("bound_channels") + ": ")
        except tk.TclError:
            pass
        self.foot_var.set(self._t("foot_prefix"))
        self._refresh_topbar()
        self._refresh_dashboard_overview_if_needed()

    def _refresh_weather_icon_display(self):
        icon = "◌"
        summary = "--"
        if isinstance(getattr(self, "_weather_data", None), dict):
            weather = self._weather_data
            icon = str(weather.get("icon") or "").strip() or "◌"
            desc = str(weather.get("description") or "").strip()
            temp = str(weather.get("temperature") or "").strip()
            summary = " ".join(part for part in (desc, temp) if part).strip() or "--"
        try:
            self._top_weather_icon_var.set(icon)
            self._top_weather_text_var.set(summary)
            self._top_weather_icon_label.config(bg=self._c("bg"), fg=self._c("accent"))
            self._top_weather_text_label.config(bg=self._c("bg"), fg=self._c("fg_dim"))
        except tk.TclError:
            pass

    def _refresh_topbar(self):
        try:
            self._top_title_label.config(text="RustClaw", bg=self._c("bg"), fg=self._c("accent"))
            if self._top_title_label.winfo_manager() != "pack":
                self._top_title_label.pack(anchor=tk.W, before=self._top_recent_message_label)
        except tk.TclError:
            pass
        self._refresh_weather_icon_display()
        if self._view_mode == "users":
            self._top_recent_message_var.set(self._t("recent_messages_title"))
        elif self._view_mode == "logs":
            self._top_recent_message_var.set("logs")
        elif self._view_mode == "skills":
            self._top_recent_message_var.set("skills")
        elif self._view_mode == "weather":
            self._top_recent_message_var.set(self._t("weather_title"))
        elif self._view_mode == "us_stock":
            self._top_recent_message_var.set(self._t("show_us_stock_page"))
        elif self._view_mode == "settings":
            self._top_recent_message_var.set(self._t("settings_title"))
        elif self._view_mode == "wifi":
            self._top_recent_message_var.set(self._t("wifi_title"))
        else:
            if self._top_recent_message_label.winfo_manager():
                self._top_recent_message_label.pack_forget()
            return
        self._top_recent_message_label.config(bg=self._c("bg"), fg=self._c("fg_dim"))
        if self._top_recent_message_label.winfo_manager() != "pack":
            self._top_recent_message_label.pack(fill=tk.X, anchor=tk.W)

    def _refresh_dashboard_overview_if_needed(self):
        if getattr(self, "_view_mode", None) != "overview":
            return
        signature = self._overview_render_signature()
        if signature == getattr(self, "_last_overview_render_signature", None):
            return
        self._render_dashboard_overview()

    def _overview_render_signature(self):
        weather = self._weather_data if isinstance(getattr(self, "_weather_data", None), dict) else {}
        return (
            tuple(str(item) for item in self._visible_view_modes()),
            self._overview_stock_updated_at,
            self._overview_us_stock_updated_at,
            self._overview_crypto_updated_at,
            self._overview_skills_updated_at,
            self._overview_stock_scroll_idx,
            self._overview_us_stock_scroll_idx,
            self._overview_crypto_scroll_idx,
            bool(self._overview_stock_loading),
            bool(self._overview_us_stock_loading),
            bool(self._overview_crypto_loading),
            bool(self._overview_skills_loading),
            self.uptime_var.get(),
            self.rss_var.get(),
            str(weather.get("temperature") or ""),
            str(weather.get("description") or ""),
            str(weather.get("updated_at") or ""),
            len(self.log_entries if isinstance(self.log_entries, list) else []),
            len(self.user_messages if isinstance(self.user_messages, list) else []),
        )

    def _overview_loading_count(self):
        return sum(
            1
            for attr in (
                "_overview_stock_loading",
                "_overview_us_stock_loading",
                "_overview_crypto_loading",
                "_overview_skills_loading",
            )
            if getattr(self, attr, False)
        )

    def _theme_label(self):
        return self._t("theme_matrix") if self._theme == "matrix" else self._t("theme_default")

    def _overview_dashboard_summary(self):
        uptime = self.uptime_var.get().strip() or "--"
        return uptime

    def _overview_dashboard_meta(self):
        rss = self.rss_var.get().strip() or "--"
        return f"RSS: {rss}"

    def _overview_stock_meta_text(self):
        summary = self._overview_stock_summary if isinstance(self._overview_stock_summary, dict) else {}
        items = summary.get("items") or []
        if items:
            first = items[0]
            meta1 = str(first.get("meta1") or "").strip()
            meta2 = str(first.get("meta2") or "").strip()
            detail = meta1 or meta2
            if detail:
                return detail
        stock_items, _refresh_sec = _load_small_screen_stock_config()
        if not stock_items:
            return ""
        first = stock_items[0]
        code = str(first.get("code") or "").strip()
        return code.upper()

    def _overview_market_primary_text(self, title, prefer_symbol=False):
        text = str(title or "").strip()
        if not text:
            return "--"
        if "·" in text:
            left, right = [part.strip() for part in text.split("·", 1)]
            if prefer_symbol and right:
                return right
            if left:
                return left
        return text

    def _overview_market_lines(self, items, visible_count, offset=0, prefer_symbol=False):
        normalized = [item for item in (items or []) if isinstance(item, dict)]
        if not normalized:
            return []
        total = len(normalized)
        if total <= visible_count:
            window = normalized
        else:
            start = offset % total
            window = [normalized[(start + idx) % total] for idx in range(visible_count)]
        lines = []
        for item in window:
            title = self._overview_market_primary_text(item.get("title"), prefer_symbol=prefer_symbol)
            price = str(item.get("price") or "--").strip() or "--"
            pct = str(item.get("pct") or "--").strip() or "--"
            lines.append(f"{title} {price} {pct}".strip())
        return lines

    def _overview_compact_rows(self, lines, per_row=2, separator="    "):
        normalized = [str(line).strip() for line in (lines or []) if str(line).strip()]
        if not normalized:
            return []
        rows = []
        for idx in range(0, len(normalized), per_row):
            rows.append(separator.join(normalized[idx:idx + per_row]))
        return rows

    def _overview_crypto_compact_per_row(self):
        self._ensure_overview_crypto_summary()
        summary = self._overview_crypto_summary if isinstance(self._overview_crypto_summary, dict) else {}
        if not summary:
            return 2
        crypto_items, _refresh_sec = _load_small_screen_crypto_config()
        if not crypto_items:
            return 2
        total = len(crypto_items)
        if total <= 4:
            window = crypto_items
        else:
            start = self._overview_crypto_scroll_idx % total
            window = [crypto_items[(start + idx) % total] for idx in range(4)]
        for item in window:
            name = str(item.get("name") or item.get("symbol") or "--").strip().upper() or "--"
            price = str((summary or {}).get(name) or "--").strip()
            if "." not in price:
                continue
            frac = price.split(".", 1)[1].strip()
            if len(frac) >= 5:
                return 1
        return 2

    def _overview_stock_display_lines(self):
        self._ensure_overview_stock_summary()
        summary = self._overview_stock_summary if isinstance(self._overview_stock_summary, dict) else {}
        items = summary.get("items") or []
        if items:
            return self._overview_market_lines(items, visible_count=4, offset=self._overview_stock_scroll_idx)
        stock_items, _refresh_sec = _load_small_screen_stock_config()
        if not stock_items:
            return [self._t("stock_empty")]
        lines = []
        for item in stock_items[:4]:
            name = str(item.get("name") or item.get("code") or "--").strip() or "--"
            code = str(item.get("code") or "").strip().upper()
            lines.append(f"{name} {code}".strip())
        return lines

    def _ensure_overview_us_stock_summary(self):
        us_stock_items, refresh_sec = _load_small_screen_us_stock_config()
        now_ts = time.time()
        ttl = max(30, refresh_sec)
        if self._overview_us_stock_loading or (now_ts - self._overview_us_stock_updated_at) < ttl:
            return
        if self._overview_loading_count() >= 2:
            return
        self._overview_us_stock_loading = True

        def worker():
            summary = fetch_us_stock_quotes(us_stock_items)

            def finish():
                self._overview_us_stock_loading = False
                self._overview_us_stock_updated_at = time.time()
                self._overview_us_stock_summary = summary
                self._refresh_dashboard_overview_if_needed()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _overview_us_stock_summary_text(self):
        self._ensure_overview_us_stock_summary()
        summary = self._overview_us_stock_summary if isinstance(self._overview_us_stock_summary, dict) else {}
        if self._overview_us_stock_loading and not summary:
            return self._t("overview_loading")
        items = summary.get("items") or []
        if items:
            first = items[0]
            return f"{first.get('title') or '--'}\n{first.get('price') or '--'}  {first.get('pct') or '--'}"
        us_stock_items, _refresh_sec = _load_small_screen_us_stock_config()
        if not us_stock_items:
            return self._t("us_stock_empty")
        first = us_stock_items[0]
        name = first.get("name") or first.get("symbol") or "--"
        return f"{name}\n{self._t('overview_configured_count').format(count=len(us_stock_items))}"

    def _overview_us_stock_meta_text(self):
        summary = self._overview_us_stock_summary if isinstance(self._overview_us_stock_summary, dict) else {}
        items = summary.get("items") or []
        if items:
            first = items[0]
            meta1 = str(first.get("meta1") or "").strip()
            meta2 = str(first.get("meta2") or "").strip()
            detail = meta1 or meta2
            if detail:
                return detail
        us_stock_items, _refresh_sec = _load_small_screen_us_stock_config()
        if not us_stock_items:
            return ""
        first = us_stock_items[0]
        return str(first.get("symbol") or "").strip().upper()

    def _overview_us_stock_display_lines(self):
        self._ensure_overview_us_stock_summary()
        summary = self._overview_us_stock_summary if isinstance(self._overview_us_stock_summary, dict) else {}
        items = summary.get("items") or []
        if self._overview_us_stock_loading and not items:
            return [self._t("overview_loading")]
        if items:
            return self._overview_market_lines(items, visible_count=6, offset=self._overview_us_stock_scroll_idx, prefer_symbol=True)
        us_stock_items, _refresh_sec = _load_small_screen_us_stock_config()
        if not us_stock_items:
            return [self._t("us_stock_empty")]
        lines = []
        for item in us_stock_items[:6]:
            name = str(item.get("symbol") or item.get("name") or "--").strip().upper() or "--"
            lines.append(name)
        return lines

    def _schedule_overview_scroll(self):
        self._cancel_job("_overview_scroll_job")
        if getattr(self, "_view_mode", None) != "overview":
            return
        stock_items = self._overview_stock_summary.get("items") if isinstance(self._overview_stock_summary, dict) else []
        us_stock_items = self._overview_us_stock_summary.get("items") if isinstance(self._overview_us_stock_summary, dict) else []
        crypto_items, _refresh_sec = _load_small_screen_crypto_config()
        needs_scroll = len(stock_items or []) > 4 or len(us_stock_items or []) > 6 or len(crypto_items or []) > 4
        if not needs_scroll:
            return
        self._overview_scroll_job = self.root.after(OVERVIEW_SCROLL_SEC * 1000, self._overview_scroll_step)

    def _overview_scroll_step(self):
        self._overview_scroll_job = None
        if getattr(self, "_view_mode", None) != "overview":
            return
        stock_items = self._overview_stock_summary.get("items") if isinstance(self._overview_stock_summary, dict) else []
        us_stock_items = self._overview_us_stock_summary.get("items") if isinstance(self._overview_us_stock_summary, dict) else []
        crypto_items, _refresh_sec = _load_small_screen_crypto_config()
        if len(stock_items or []) > 4:
            self._overview_stock_scroll_idx = (self._overview_stock_scroll_idx + 1) % len(stock_items)
        if len(us_stock_items or []) > 6:
            self._overview_us_stock_scroll_idx = (self._overview_us_stock_scroll_idx + 1) % len(us_stock_items)
        if len(crypto_items or []) > 4:
            self._overview_crypto_scroll_idx = (self._overview_crypto_scroll_idx + 1) % len(crypto_items)
        self._render_dashboard_overview()

    def _overview_users_summary(self):
        items = self.user_messages if isinstance(self.user_messages, list) else []
        lines = []
        if items:
            latest = items[0]
            preview = _single_line_message_preview(
                latest.get("question") or latest.get("text") or "",
                self._lang,
            )
            if preview:
                lines.append(preview)
        user_count = self.users_count_var.get().strip() or "--"
        bound_count = self.bound_channels_var.get().strip() or "--"
        lines.append(f"{self._t('users_count')}: {user_count}  {self._t('bound_channels')}: {bound_count}")
        return "\n".join(lines[:2]) if lines else self._t("recent_messages_empty")

    def _overview_logs_summary(self):
        items = self.log_entries if isinstance(self.log_entries, list) else []
        if not items:
            return self._t("logs_empty")
        latest = items[0]
        detail = _sanitize_display_text(latest.get("detail") or latest.get("raw") or "").strip()
        prefix = f"{latest.get('kind') or 'LOG'}  {latest.get('time') or '--:--:--'}"
        return prefix if not detail else prefix + "\n" + detail

    def _ensure_overview_skills_summary(self):
        now_ts = time.time()
        if self._overview_skills_loading or (now_ts - self._overview_skills_updated_at) < 60:
            return
        if self._overview_loading_count() >= 2:
            return
        self._overview_skills_loading = True

        def worker():
            all_skills, enabled_set = fetch_skills_config(self._auth_key)

            def finish():
                self._overview_skills_loading = False
                self._overview_skills_updated_at = time.time()
                if all_skills is None:
                    self._overview_skills_summary = {"error": True}
                else:
                    self._overview_skills_summary = {
                        "total": len(all_skills),
                        "enabled": len(enabled_set or set()),
                        "sample": list(all_skills[:2]),
                    }
                self._refresh_dashboard_overview_if_needed()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _overview_skills_summary_text(self):
        self._ensure_overview_skills_summary()
        summary = self._overview_skills_summary if isinstance(self._overview_skills_summary, dict) else {}
        if self._overview_skills_loading and not summary:
            return self._t("overview_loading")
        if summary.get("error"):
            return self._t("skills_load_fail")
        total = summary.get("total")
        enabled = summary.get("enabled")
        if total is None:
            return self._t("overview_tap_hint")
        sample = ", ".join(summary.get("sample") or [])
        first_line = self._t("overview_skills_enabled").format(enabled=enabled, total=total)
        return first_line if not sample else first_line + "\n" + sample

    def _ensure_overview_stock_summary(self):
        stock_items, refresh_sec = _load_small_screen_stock_config()
        now_ts = time.time()
        ttl = max(30, refresh_sec)
        if self._overview_stock_loading or (now_ts - self._overview_stock_updated_at) < ttl:
            return
        if self._overview_loading_count() >= 2:
            return
        self._overview_stock_loading = True

        def worker():
            summary = fetch_a_share_quotes(stock_items)

            def finish():
                self._overview_stock_loading = False
                self._overview_stock_updated_at = time.time()
                self._overview_stock_summary = summary
                self._refresh_dashboard_overview_if_needed()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _overview_stock_summary_text(self):
        self._ensure_overview_stock_summary()
        summary = self._overview_stock_summary if isinstance(self._overview_stock_summary, dict) else {}
        if self._overview_stock_loading and not summary:
            return self._t("overview_loading")
        items = summary.get("items") or []
        if items:
            first = items[0]
            return f"{first.get('title') or '--'}\n{first.get('price') or '--'}  {first.get('pct') or '--'}"
        stock_items, _refresh_sec = _load_small_screen_stock_config()
        if not stock_items:
            return self._t("stock_empty")
        first = stock_items[0]
        name = first.get("name") or (first.get("code") or "--").upper()
        return f"{name}\n{self._t('overview_configured_count').format(count=len(stock_items))}"

    def _ensure_overview_crypto_summary(self):
        crypto_items, refresh_sec = _load_small_screen_crypto_config()
        now_ts = time.time()
        ttl = max(30, refresh_sec)
        if self._overview_crypto_loading or (now_ts - self._overview_crypto_updated_at) < ttl:
            return
        if self._overview_loading_count() >= 2:
            return
        self._overview_crypto_loading = True

        def worker():
            prices = fetch_crypto_prices(crypto_items)

            def finish():
                self._overview_crypto_loading = False
                self._overview_crypto_updated_at = time.time()
                self._overview_crypto_summary = prices
                self._refresh_dashboard_overview_if_needed()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _overview_crypto_summary_text(self):
        self._ensure_overview_crypto_summary()
        summary = self._overview_crypto_summary if isinstance(self._overview_crypto_summary, dict) else {}
        if self._overview_crypto_loading and not summary:
            return self._t("overview_loading")
        crypto_items, _refresh_sec = _load_small_screen_crypto_config()
        if not crypto_items:
            return self._t("crypto_empty")
        target = crypto_items[self._overview_crypto_scroll_idx % len(crypto_items)]
        if summary and target:
            name = str(target.get("name") or target.get("symbol") or "--").strip().upper() or "--"
            return f"{name}\n{summary.get(name) or '--'} USDT"
        name = str(target.get("name") or target.get("symbol") or "--").strip().upper() or "--"
        return f"{name}\n-- USDT"

    def _overview_crypto_meta_text(self):
        crypto_items, _refresh_sec = _load_small_screen_crypto_config()
        if not crypto_items:
            return ""
        target = crypto_items[self._overview_crypto_scroll_idx % len(crypto_items)]
        symbol = str(target.get("symbol") or "").strip().upper()
        return symbol or str(target.get("name") or "--").strip().upper()

    def _overview_crypto_display_lines(self):
        self._ensure_overview_crypto_summary()
        summary = self._overview_crypto_summary if isinstance(self._overview_crypto_summary, dict) else {}
        if self._overview_crypto_loading and not summary:
            return [self._t("overview_loading")]
        crypto_items, _refresh_sec = _load_small_screen_crypto_config()
        if not crypto_items:
            return [self._t("crypto_empty")]
        total = len(crypto_items)
        if total <= 4:
            window = crypto_items
        else:
            start = self._overview_crypto_scroll_idx % total
            window = [crypto_items[(start + idx) % total] for idx in range(4)]
        lines = []
        for item in window:
            name = str(item.get("name") or item.get("symbol") or "--").strip().upper() or "--"
            price = str((summary or {}).get(name) or "--").strip() or "--"
            lines.append(f"{name} {price}")
        return lines

    def _overview_gallery_summary(self):
        state = self._t("overview_gallery_running") if self._llm_lobster_job else self._t("overview_gallery_idle")
        if self._llm_pubkey_loading or self._llm_signing or self._llm_join_in_progress:
            return state + "\n" + self._t("overview_loading")
        if self._llm_signature_hex:
            return state + "\n" + self._t("llm_sign_signature")
        if self._llm_pubkey_hex:
            return state + "\n" + self._t("llm_pubkey_slot0")
        return state + "\n" + self._t("overview_tap_hint")

    def _overview_settings_summary(self):
        visible_pages = len([mode for mode in self._visible_view_modes() if mode not in {"dashboard", "settings"}])
        return (
            f"{self._t('language')}: {self._lang}  {self._t('theme')}: {self._theme_label()}\n"
            + self._t("overview_visible_pages").format(count=visible_pages)
        )

    def _open_view_from_overview(self, mode):
        self._overview_return_on_swipe = True
        self._switch_view(mode)

    def _open_weather_from_topbar(self, _event=None):
        current_mode = getattr(self, "_view_mode", None)
        if current_mode is None or current_mode == "weather":
            return
        self._weather_return_view_on_swipe = current_mode
        self._switch_view("weather")

    def _reset_overview_double_tap(self):
        self._overview_last_tap_at = 0.0
        self._overview_last_tap_mode = None
        self._overview_last_tap_x = 0
        self._overview_last_tap_y = 0

    def _start_overview_open_tap(self, event):
        self._overview_tap_press_x = getattr(event, "x_root", 0)
        self._overview_tap_press_y = getattr(event, "y_root", 0)

    def _finish_overview_open_tap(self, event, target_mode):
        if getattr(self, "_view_mode", None) != "overview":
            self._reset_overview_double_tap()
            return
        x = getattr(event, "x_root", 0)
        y = getattr(event, "y_root", 0)
        press_dx = abs(x - getattr(self, "_overview_tap_press_x", 0))
        press_dy = abs(y - getattr(self, "_overview_tap_press_y", 0))
        if press_dx > 18 or press_dy > 18:
            self._reset_overview_double_tap()
            return
        now = time.monotonic()
        last_at = float(getattr(self, "_overview_last_tap_at", 0.0) or 0.0)
        last_mode = getattr(self, "_overview_last_tap_mode", None)
        last_x = getattr(self, "_overview_last_tap_x", 0)
        last_y = getattr(self, "_overview_last_tap_y", 0)
        same_mode = last_mode == target_mode
        close_enough = abs(x - last_x) <= 28 and abs(y - last_y) <= 28
        if same_mode and close_enough and (now - last_at) <= OVERVIEW_DOUBLE_TAP_SEC:
            self._reset_overview_double_tap()
            self._open_view_from_overview(target_mode)
            return
        self._overview_last_tap_at = now
        self._overview_last_tap_mode = target_mode
        self._overview_last_tap_x = x
        self._overview_last_tap_y = y

    def _bind_overview_open(self, widget, target_mode):
        widget.bind("<ButtonPress-1>", self._start_overview_open_tap)
        widget.bind("<ButtonRelease-1>", lambda evt, m=target_mode: self._finish_overview_open_tap(evt, m))

    def _build_overview_layout(self):
        return overview_build_layout(
            self,
            OVERVIEW_US_STOCK_HEIGHT,
            OVERVIEW_A_STOCK_WIDTH,
            OVERVIEW_MARKET_HEIGHT,
            OVERVIEW_MARKET_GAP,
            OVERVIEW_CRYPTO_HEIGHT,
            OVERVIEW_RUNTIME_HEIGHT,
        )

    def _render_dashboard_overview(self):
        result = overview_render_dashboard(self)
        self._last_overview_render_signature = self._overview_render_signature()
        return result

    def _prepare_settings_view(self):
        return settings_prepare_view(self)

    def _refresh_settings_choice_labels(self):
        return settings_refresh_choice_labels(self)

    def _show_settings_category(self, category):
        return settings_show_category(self, category)

    def _show_settings_menu(self):
        return settings_show_menu(self)

    def _open_wifi_from_settings(self):
        return settings_open_wifi_from_settings(self)

    def _close_wifi_to_settings(self):
        return settings_close_wifi_to_settings(self)

    def _visible_view_modes(self):
        modes = ["dashboard", "overview"]
        if self._show_messages_page:
            modes.append("users")
        if self._show_logs_page:
            modes.append("logs")
        if self._show_skills_page:
            modes.append("skills")
        if self._show_weather_page:
            modes.append("weather")
        if self._show_stock_page:
            modes.append("stock")
        if self._show_us_stock_page:
            modes.append("us_stock")
        if self._show_crypto_page:
            modes.append("crypto")
        if self._show_gallery_page:
            modes.append("gallery")
        modes.append("settings")
        return modes

    def _switch_view(self, mode):
        if mode not in {"dashboard", "overview", "users", "logs", "skills", "weather", "stock", "us_stock", "crypto", "gallery", "settings", "wifi"}:
            mode = "dashboard"
        self._reset_overview_double_tap()
        self._teardown_current_view()
        for frame in (
            self.dashboard_frame,
            self.overview_frame,
            self.users_frame,
            self.logs_frame,
            self.skills_frame,
            self.weather_frame,
            self.stock_frame,
            self.us_stock_frame,
            self.crypto_frame,
            self.gallery_frame,
            self.settings_frame,
            self.wifi_frame,
        ):
            if frame.winfo_manager():
                frame.pack_forget()
        self._view_mode = mode
        if mode != "weather":
            self._weather_return_view_on_swipe = None
        if mode == "overview":
            self._overview_return_on_swipe = False
        if mode == "dashboard":
            self.dashboard_frame.pack(fill=tk.BOTH, expand=True)
        elif mode == "overview":
            self.overview_frame.pack(fill=tk.BOTH, expand=True)
            self._render_dashboard_overview()
        elif mode == "users":
            self._prepare_users_view()
            self.users_frame.pack(fill=tk.BOTH, expand=True)
        elif mode == "logs":
            self._prepare_logs_view()
            self.logs_frame.pack(fill=tk.BOTH, expand=True)
        elif mode == "skills":
            self.skills_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._refresh_skills_view()
        elif mode == "weather":
            self.weather_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_weather()
        elif mode == "stock":
            self.stock_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_stock()
        elif mode == "us_stock":
            self.us_stock_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_us_stock()
        elif mode == "crypto":
            self.crypto_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_crypto()
        elif mode == "gallery":
            self.gallery_frame.pack(fill=tk.BOTH, expand=True, padx=(2, 14), pady=4)
            self._show_gallery()
        elif mode == "settings":
            self._prepare_settings_view()
            self.settings_frame.pack(fill=tk.BOTH, expand=True)
        elif mode == "wifi":
            self._prepare_wifi_view()
            self.wifi_frame.pack(fill=tk.BOTH, expand=True)
            if not self._wifi_networks and not self._wifi_scan_in_progress:
                self._refresh_wifi_networks()
        self._refresh_topbar()

    def _prepare_wifi_view(self):
        self._wifi_pager_row.config(bg=self._c("bg"))
        self._wifi_right_actions.config(bg=self._c("bg"))
        self._wifi_back_btn.config(
            text=self._t("back"),
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
        )
        self._wifi_refresh_btn.config(
            text=self._t("wifi_refreshing") if self._wifi_scan_in_progress else self._t("wifi_refresh"),
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
        )
        self._wifi_join_btn.config(
            text=self._t("wifi_connecting") if self._wifi_connect_in_progress else self._t("wifi_join"),
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
        )
        self._wifi_prev_btn.config(text=self._t("wifi_prev_page"), bg=self._c("button_bg"), fg=self._c("button_fg"))
        self._wifi_next_btn.config(text=self._t("wifi_next_page"), bg=self._c("button_bg"), fg=self._c("button_fg"))
        self._wifi_prev_btn.config(activebackground=self._c("button_active_bg"), activeforeground=self._c("fg"), disabledforeground=self._c("fg_dim"))
        self._wifi_next_btn.config(activebackground=self._c("button_active_bg"), activeforeground=self._c("fg"), disabledforeground=self._c("fg_dim"))
        self._wifi_status_label.config(bg=self._c("bg"), fg=self._c("fg_dim"))
        self._wifi_selected_label.config(bg=self._c("bg"), fg=self._c("accent"))
        self._wifi_hint_label.config(bg=self._c("bg"), fg=self._c("fg_dim"))
        self._wifi_page_label.config(bg=self._c("bg"), fg=self._c("fg_dim"))
        self._render_wifi_view()

    def _format_wifi_name(self, item):
        ssid = str(item.get("ssid") or "--")
        if len(ssid) > 24:
            ssid = ssid[:21] + "..."
        signal = item.get("signal")
        signal_text = f"{signal}%" if signal is not None else "--"
        secure = str(item.get("security") or "").strip()
        suffix = " 🔒" if secure else ""
        if item.get("active"):
            suffix += " " + self._t("wifi_connected_tag")
        return f"{ssid}  {signal_text}{suffix}"

    def _wifi_keyboard_rows(self):
        if self._wifi_keyboard_mode == "symbol":
            return [
                list("1234567890"),
                list("!@#$%^&*()"),
                list("-_=+[]{}"),
                list(";:,.?/\\|"),
            ]
        if self._wifi_keyboard_mode == "upper":
            return [
                list("1234567890"),
                list("QWERTYUIOP"),
                list("ASDFGHJKL"),
                list("ZXCVBNM"),
            ]
        return [
            list("1234567890"),
            list("qwertyuiop"),
            list("asdfghjkl"),
            list("zxcvbnm"),
        ]

    def _render_wifi_view(self):
        if not hasattr(self, "_wifi_list_frame"):
            return
        for child in self._wifi_list_frame.winfo_children():
            child.destroy()
        page_size = 3
        total = len(self._wifi_networks)
        page_count = max(1, (total + page_size - 1) // page_size)
        self._wifi_page_index = max(0, min(self._wifi_page_index, page_count - 1))
        start = self._wifi_page_index * page_size
        current = self._wifi_networks[start:start + page_size]
        if not current:
            tk.Label(
                self._wifi_list_frame,
                text=self._t("wifi_empty") if not self._wifi_scan_error else self._t("wifi_scan_failed").format(error=self._wifi_scan_error),
                font=("", 11),
                bg=self._c("bg"),
                fg=self._c("fg_dim"),
                anchor="w",
                justify=tk.LEFT,
                wraplength=450,
            ).pack(fill=tk.X, pady=(6, 0))
        else:
            for item in current:
                selected = item.get("ssid") == self._wifi_selected_ssid
                row = tk.Frame(self._wifi_list_frame, bg=self._c("bg"))
                row.pack(fill=tk.X, pady=(0, 4))
                active = bool(item.get("active"))
                actions = tk.Frame(row, bg=self._c("bg"))
                actions.pack(side=tk.RIGHT, padx=(6, 0))
                btn = tk.Button(
                    row,
                    text=self._format_wifi_name(item),
                    font=("", 10, "bold" if selected else "normal"),
                    relief=tk.FLAT,
                    borderwidth=0,
                    highlightthickness=0,
                    anchor="w",
                    justify=tk.LEFT,
                    bg=self._c("accent") if selected else self._c("box_bg"),
                    fg=self._c("button_fg") if selected else self._c("fg"),
                    activebackground=self._c("button_active_bg"),
                    activeforeground=self._c("fg"),
                    takefocus=0,
                    command=lambda data=item: self._select_wifi_network(data),
                )
                btn.pack(side=tk.LEFT, fill=tk.X, expand=True, ipady=4)
                if active:
                    disconnect_btn = tk.Button(
                        actions,
                        text=self._t("wifi_disconnecting") if self._wifi_disconnect_in_progress else self._t("wifi_disconnect"),
                        font=("", 9),
                        relief=tk.FLAT,
                        borderwidth=0,
                        highlightthickness=0,
                        bg=self._c("button_bg"),
                        fg=self._c("button_fg"),
                        activebackground=self._c("button_active_bg"),
                        activeforeground=self._c("fg"),
                        disabledforeground=self._c("fg_dim"),
                        takefocus=0,
                        state=tk.DISABLED if self._wifi_disconnect_in_progress else tk.NORMAL,
                        command=lambda ssid=item.get("ssid") or "": self._disconnect_wifi(ssid),
                    )
                    disconnect_btn.pack(side=tk.RIGHT)
        self._wifi_page_var.set(f"{self._wifi_page_index + 1}/{page_count}")
        self._wifi_prev_btn.config(state=tk.NORMAL if self._wifi_page_index > 0 else tk.DISABLED)
        self._wifi_next_btn.config(state=tk.NORMAL if self._wifi_page_index < page_count - 1 else tk.DISABLED)
        selected_text = self._wifi_selected_ssid or "--"
        self._wifi_selected_var.set(f"{self._t('wifi_selected')}: {selected_text}")
        if self._wifi_selected_security:
            self._wifi_hint_var.set(self._t("wifi_secure_hint"))
        else:
            self._wifi_hint_var.set(self._t("wifi_open_hint") if self._wifi_selected_ssid else self._t("wifi_scan_hint"))
        self._wifi_status_var.set(self._wifi_status_text or self._t("wifi_scan_hint"))
        selected_active = any(
            item.get("active") and str(item.get("ssid") or "") == self._wifi_selected_ssid
            for item in self._wifi_networks
        )
        self._wifi_join_btn.config(
            text=self._t("wifi_connecting") if self._wifi_connect_in_progress else self._t("wifi_join"),
            state=tk.DISABLED if self._wifi_connect_in_progress or self._wifi_disconnect_in_progress or not self._wifi_selected_ssid else tk.NORMAL,
        )
        if self._wifi_selected_ssid and not selected_active:
            if self._wifi_join_btn.winfo_manager() != "pack":
                self._wifi_join_btn.pack(side=tk.LEFT, padx=(8, 0))
        else:
            if self._wifi_join_btn.winfo_manager():
                self._wifi_join_btn.pack_forget()
        self._wifi_refresh_btn.config(
            text=self._t("wifi_refreshing") if self._wifi_scan_in_progress else self._t("wifi_refresh"),
            state=tk.DISABLED if self._wifi_scan_in_progress or self._wifi_connect_in_progress or self._wifi_disconnect_in_progress else tk.NORMAL,
        )

    def _change_wifi_page(self, delta):
        self._wifi_page_index = max(0, self._wifi_page_index + delta)
        self._render_wifi_view()

    def _select_wifi_network(self, item):
        self._wifi_selected_ssid = str(item.get("ssid") or "").strip()
        self._wifi_selected_security = str(item.get("security") or "").strip()
        self._wifi_password_var.set("")
        self._wifi_status_text = self._t("wifi_secure_hint") if self._wifi_selected_security else self._t("wifi_open_hint")
        self._render_wifi_view()

    def _on_wifi_join_click(self):
        if not self._wifi_selected_ssid.strip():
            self._wifi_status_text = self._t("wifi_no_selection")
            self._render_wifi_view()
            return
        if self._wifi_selected_security:
            self._open_wifi_keyboard()
            return
        self._connect_selected_wifi()

    def _wifi_append_char(self, text):
        self._wifi_password_var.set(self._wifi_password_var.get() + text)

    def _wifi_backspace(self):
        current = self._wifi_password_var.get()
        self._wifi_password_var.set(current[:-1] if current else "")

    def _toggle_wifi_keyboard_case(self):
        if self._wifi_keyboard_mode == "symbol":
            self._wifi_keyboard_mode = "lower"
        else:
            self._wifi_keyboard_mode = "upper" if self._wifi_keyboard_mode == "lower" else "lower"
        self._render_wifi_view()
        self._refresh_wifi_keyboard_popup()

    def _toggle_wifi_keyboard_symbols(self):
        self._wifi_keyboard_mode = "symbol" if self._wifi_keyboard_mode != "symbol" else "lower"
        self._render_wifi_view()
        self._refresh_wifi_keyboard_popup()

    def _toggle_wifi_password_visibility(self):
        self._wifi_password_visible = not self._wifi_password_visible
        self._render_wifi_view()
        self._refresh_wifi_keyboard_popup()

    def _refresh_wifi_keyboard_popup(self):
        popup = getattr(self, "_wifi_keyboard_window", None)
        if not popup or not popup.winfo_exists():
            return
        for child in popup.winfo_children():
            child.destroy()
        self._build_wifi_keyboard_popup(popup)

    def _build_wifi_keyboard_popup(self, popup):
        header = tk.Frame(popup, bg=self._c("bg"))
        header.pack(fill=tk.X, padx=8, pady=(6, 3))
        tk.Label(
            header,
            text=self._t("wifi_password") + ": " + (self._wifi_selected_ssid or "--"),
            font=("", 11, "bold"),
            bg=self._c("bg"),
            fg=self._c("fg"),
            anchor="w",
        ).pack(side=tk.LEFT, fill=tk.X, expand=True)
        tk.Button(
            header,
            text=self._t("wifi_join"),
            font=("", 10),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=lambda: self._connect_selected_wifi(close_keyboard_on_success=True),
        ).pack(side=tk.RIGHT, padx=(0, 6))
        tk.Button(
            header,
            text=self._t("wifi_keyboard_done"),
            font=("", 10),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=self._close_wifi_keyboard,
        ).pack(side=tk.RIGHT)
        entry_row = tk.Frame(popup, bg=self._c("bg"))
        entry_row.pack(fill=tk.X, padx=8, pady=(0, 4))
        entry = tk.Entry(
            entry_row,
            textvariable=self._wifi_password_var,
            font=("", 10),
            relief=tk.FLAT,
            show="" if self._wifi_password_visible else "*",
            bg=self._c("box_bg"),
            fg=self._c("fg"),
            insertbackground=self._c("fg"),
        )
        entry.pack(side=tk.LEFT, fill=tk.X, expand=True)
        tk.Button(
            entry_row,
            text=self._t("wifi_hide_password") if self._wifi_password_visible else self._t("wifi_show_password"),
            font=("", 9),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=self._toggle_wifi_password_visibility,
        ).pack(side=tk.LEFT, padx=(8, 0))
        keyboard = tk.Frame(popup, bg=self._c("bg"))
        keyboard.pack(fill=tk.BOTH, expand=True, padx=8, pady=(0, 6))
        rows = self._wifi_keyboard_rows()
        for keys in rows:
            row = tk.Frame(keyboard, bg=self._c("bg"))
            row.pack(fill=tk.X, pady=(0, 2))
            for key in keys:
                tk.Button(
                    row,
                    text=key,
                    font=("", 9),
                    relief=tk.FLAT,
                    bg=self._c("button_bg"),
                    fg=self._c("button_fg"),
                    command=lambda ch=key: self._wifi_append_char(ch),
                ).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 2), ipady=0)
        control = tk.Frame(keyboard, bg=self._c("bg"))
        control.pack(fill=tk.X, pady=(1, 0))
        tk.Button(
            control,
            text=self._t("wifi_shift"),
            font=("", 8),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=self._toggle_wifi_keyboard_case,
        ).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 2), ipady=0)
        tk.Button(
            control,
            text=self._t("wifi_symbols") if self._wifi_keyboard_mode != "symbol" else self._t("wifi_letters"),
            font=("", 8),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=self._toggle_wifi_keyboard_symbols,
        ).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 2), ipady=0)
        tk.Button(
            control,
            text=self._t("wifi_space"),
            font=("", 8),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=lambda: self._wifi_append_char(" "),
        ).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 2), ipady=0)
        tk.Button(
            control,
            text=self._t("wifi_backspace"),
            font=("", 8),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=self._wifi_backspace,
        ).pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 2), ipady=0)
        tk.Button(
            control,
            text=self._t("wifi_clear"),
            font=("", 8),
            relief=tk.FLAT,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            command=lambda: self._wifi_password_var.set(""),
        ).pack(side=tk.LEFT, fill=tk.X, expand=True, ipady=0)

    def _open_wifi_keyboard(self):
        popup = getattr(self, "_wifi_keyboard_window", None)
        if popup and popup.winfo_exists():
            popup.lift()
            return
        popup = tk.Toplevel(self.root)
        popup.configure(bg=self._c("bg"))
        popup.geometry(f"{W}x{H}")
        popup.resizable(False, False)
        popup.transient(self.root)
        popup.grab_set()
        popup.attributes("-topmost", True)
        self._wifi_keyboard_window = popup
        popup.protocol("WM_DELETE_WINDOW", self._close_wifi_keyboard)
        self._build_wifi_keyboard_popup(popup)

    def _close_wifi_keyboard(self):
        popup = getattr(self, "_wifi_keyboard_window", None)
        if not popup:
            return
        try:
            popup.grab_release()
        except Exception:
            pass
        try:
            popup.destroy()
        except Exception:
            pass
        self._wifi_keyboard_window = None

    def _refresh_wifi_networks(self):
        if self._wifi_scan_in_progress:
            return
        self._wifi_scan_in_progress = True
        self._wifi_scan_error = None
        self._wifi_status_text = self._t("wifi_refreshing")
        self._render_wifi_view()

        def worker():
            items, err = scan_wifi_networks()

            def finish():
                self._wifi_scan_in_progress = False
                self._wifi_scan_error = err
                if isinstance(items, list):
                    self._wifi_networks = items
                    if self._wifi_selected_ssid and not any(
                        str(item.get("ssid") or "") == self._wifi_selected_ssid for item in items
                    ):
                        self._wifi_selected_ssid = ""
                        self._wifi_selected_security = ""
                        self._wifi_password_var.set("")
                    self._wifi_status_text = self._t("wifi_scan_hint") if items else self._t("wifi_empty")
                else:
                    self._wifi_networks = []
                    self._wifi_status_text = self._t("wifi_scan_failed").format(error=(err or "unknown error"))
                self._render_wifi_view()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _connect_selected_wifi(self, close_keyboard_on_success=False):
        ssid = self._wifi_selected_ssid.strip()
        if not ssid:
            self._wifi_status_text = self._t("wifi_no_selection")
            self._render_wifi_view()
            return
        password = self._wifi_password_var.get()
        if self._wifi_selected_security and not str(password).strip():
            self._wifi_status_text = self._t("wifi_password_required")
            self._render_wifi_view()
            try:
                from tkinter import messagebox

                messagebox.showerror(self._t("wifi_title"), self._wifi_status_text)
            except Exception:
                pass
            return
        if self._wifi_connect_in_progress:
            return
        self._wifi_connect_in_progress = True
        self._wifi_status_text = self._t("wifi_connecting")
        self._render_wifi_view()

        def worker():
            ok, message = connect_wifi_network(ssid, password=password)

            def finish():
                self._wifi_connect_in_progress = False
                if ok:
                    if close_keyboard_on_success:
                        self._close_wifi_keyboard()
                    self._wifi_status_text = self._t("wifi_connect_success").format(ssid=ssid)
                    try:
                        from tkinter import messagebox

                        messagebox.showinfo(self._t("wifi_title"), self._t("wifi_connect_success").format(ssid=ssid))
                    except Exception:
                        pass
                    self._refresh_wifi_networks()
                else:
                    self._wifi_status_text = self._t("wifi_connect_failed").format(error=(message or "unknown error"))
                    try:
                        from tkinter import messagebox

                        messagebox.showerror(self._t("wifi_title"), self._wifi_status_text)
                    except Exception:
                        pass
                    self._render_wifi_view()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _disconnect_wifi(self, ssid):
        ssid = (ssid or "").strip()
        if not ssid or self._wifi_disconnect_in_progress or self._wifi_connect_in_progress:
            return
        self._wifi_disconnect_in_progress = True
        self._wifi_status_text = self._t("wifi_disconnecting")
        self._render_wifi_view()

        def worker():
            ok, message = disconnect_wifi_network(ssid)

            def finish():
                self._wifi_disconnect_in_progress = False
                if ok:
                    if self._wifi_selected_ssid == ssid:
                        self._wifi_password_var.set("")
                    self._wifi_status_text = self._t("wifi_disconnect_success").format(ssid=ssid)
                    try:
                        from tkinter import messagebox

                        messagebox.showinfo(self._t("wifi_title"), self._t("wifi_disconnect_success").format(ssid=ssid))
                    except Exception:
                        pass
                    self._refresh_wifi_networks()
                else:
                    self._wifi_status_text = self._t("wifi_disconnect_failed").format(error=(message or "unknown error"))
                    try:
                        from tkinter import messagebox

                        messagebox.showerror(self._t("wifi_title"), self._wifi_status_text)
                    except Exception:
                        pass
                    self._render_wifi_view()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _prepare_logs_view(self):
        self._last_logs_signature = None
        self._render_logs_view()

    def _activity_fetch_profile(self):
        mode = getattr(self, "_view_mode", None)
        if mode == "logs":
            return {"lines": 300, "log_limit": 24, "message_limit": 5}
        if mode == "users":
            return {"lines": 220, "log_limit": 12, "message_limit": 5}
        if mode == "overview":
            return {"lines": 120, "log_limit": 10, "message_limit": 4}
        return {"lines": 90, "log_limit": 8, "message_limit": 3}

    def _prepare_users_view(self):
        self._dashboard_summary_row.config(bg=self._c("bg"))
        self._dashboard_users_label.config(text=self._t("users_count") + ": ", bg=self._c("bg"), fg=self._c("fg_dim"))
        self._dashboard_users_value.config(bg=self._c("bg"), fg=self._c("fg"))
        self._dashboard_channels_label.config(text="    " + self._t("bound_channels") + ": ", bg=self._c("bg"), fg=self._c("fg_dim"))
        self._dashboard_channels_value.config(bg=self._c("bg"), fg=self._c("adapters_value_fg"))
        self._adapters_value_label.config(bg=self._c("bg"), fg=self._c("adapters_value_fg"))
        self._refresh_topbar()
        self._last_user_messages_signature = None
        self._update_user_summary_view()

    def _update_user_summary_view(self):
        data = self.health if isinstance(self.health, dict) else {}
        user_count = data.get("user_count")
        bound_channel_count = data.get("bound_channel_count")
        self.users_count_var.set(str(user_count) if user_count is not None else "--")
        self.bound_channels_var.set(str(bound_channel_count) if bound_channel_count is not None else "--")
        if self._view_mode == "users":
            self._render_user_messages()

    def _format_llm_pubkey_text(self):
        if (
            self._llm_info_hidden
            and not self._llm_pubkey_loading
            and not self._llm_signing
            and not self._llm_pubkey_hex
            and not self._llm_signature_hex
            and not self._llm_pubkey_error
            and not self._llm_signature_error
        ):
            return ""
        parts = []
        if self._llm_pubkey_loading:
            parts.append(self._t("llm_pubkey_loading"))
        elif self._llm_pubkey_hex:
            chunks = [
                self._llm_pubkey_hex[i:i + 32]
                for i in range(0, len(self._llm_pubkey_hex), 32)
            ]
            parts.append(self._t("llm_pubkey_slot0") + ":\n" + "\n".join(chunks))
        elif self._llm_pubkey_error:
            parts.append(self._t("llm_pubkey_error"))
        else:
            parts.append(self._t("llm_pubkey_empty"))

        if self._llm_signing:
            parts.append(self._t("llm_signing"))
        elif self._llm_signature_hex:
            sig_chunks = [
                self._llm_signature_hex[i:i + 32]
                for i in range(0, len(self._llm_signature_hex), 32)
            ]
            parts.append(
                self._t("llm_sign_timestamp") + f": {self._llm_signature_timestamp}\n"
                + self._t("llm_sign_signature") + ":\n"
                + "\n".join(sig_chunks)
            )
        elif self._llm_signature_error:
            parts.append(self._t("llm_sign_failed"))

        return "\n\n".join(parts)

    def _refresh_llm_pubkey_label(self):
        info_frame = getattr(self, "_llm_info_frame", None)
        label = getattr(self, "_llm_pubkey_label", None)
        if not label or not label.winfo_exists() or not info_frame or not info_frame.winfo_exists():
            return
        text = self._format_llm_pubkey_text()
        try:
            if text:
                if not info_frame.winfo_manager():
                    content = getattr(self, "_llm_content", None)
                    if content and content.winfo_exists():
                        info_frame.pack(fill=tk.X, pady=self._llm_info_pady, before=content)
                    else:
                        info_frame.pack(fill=tk.X, pady=self._llm_info_pady)
            else:
                if info_frame.winfo_manager():
                    info_frame.pack_forget()
            label.config(
                text=text,
                bg=self._c("box_bg"),
                fg=self._c("fg"),
            )
        except tk.TclError:
            pass

    def _cancel_llm_clear_job(self):
        if self._llm_clear_job:
            try:
                self.root.after_cancel(self._llm_clear_job)
            except tk.TclError:
                pass
            self._llm_clear_job = None

    def _clear_llm_info_display(self):
        self._cancel_llm_clear_job()
        self._llm_pubkey_hex = ""
        self._llm_pubkey_error = ""
        self._llm_pubkey_loading = False
        self._llm_signature_hex = ""
        self._llm_signature_error = ""
        self._llm_signature_timestamp = ""
        self._llm_signing = False
        self._llm_info_hidden = True
        self._refresh_llm_pubkey_label()

    def _schedule_llm_info_clear(self):
        self._cancel_llm_clear_job()
        self._llm_clear_job = self.root.after(5000, self._clear_llm_info_display)

    def _stop_llm_animation(self):
        if self._llm_lobster_job:
            try:
                self.root.after_cancel(self._llm_lobster_job)
            except tk.TclError:
                pass
            self._llm_lobster_job = None
        content = getattr(self, "_llm_content", None)
        if content and content.winfo_exists():
            for w in content.winfo_children():
                try:
                    w.destroy()
                except tk.TclError:
                    pass
        dot_labels = getattr(self, "_llm_dot_labels", None)
        if isinstance(dot_labels, list):
            dot_labels.clear()
        else:
            self._llm_dot_labels = []
        self._llm_lobster_count = 0

    def _start_llm_pubkey_and_sign_flow(self):
        if self._llm_join_in_progress:
            return
        self._cancel_llm_clear_job()
        self._stop_llm_animation()
        self._llm_join_in_progress = True
        self._llm_info_hidden = False
        self._llm_pubkey_hex = ""
        self._llm_pubkey_error = ""
        self._llm_signature_hex = ""
        self._llm_signature_error = ""
        self._llm_signature_timestamp = ""
        self._llm_pubkey_loading = True
        self._llm_signing = False
        self._refresh_llm_pubkey_label()

        def worker():
            pubkey_hex, pubkey_error = read_slot0_pubkey_via_helper()
            if not pubkey_hex:
                def finish_pubkey_failed():
                    self._stop_llm_animation()
                    self._llm_join_in_progress = False
                    self._llm_pubkey_loading = False
                    self._llm_pubkey_hex = ""
                    self._llm_pubkey_error = (pubkey_error or "").strip()
                    self._llm_signing = False
                    self._llm_signature_hex = ""
                    self._llm_signature_error = ""
                    self._llm_signature_timestamp = ""
                    try:
                        self._llm_join_btn.config(text=self._t("llm_join"))
                    except tk.TclError:
                        pass
                    self._refresh_llm_pubkey_label()

                self._post_ui(finish_pubkey_failed)
                return

            now_ts = int(time.time())

            def switch_to_signing():
                self._llm_pubkey_loading = False
                self._llm_pubkey_hex = pubkey_hex or ""
                self._llm_pubkey_error = ""
                self._llm_signing = True
                self._llm_signature_hex = ""
                self._llm_signature_error = ""
                self._llm_signature_timestamp = str(now_ts)
                self._refresh_llm_pubkey_label()

            self._post_ui(switch_to_signing)
            payload, sign_error = sign_unix_time_via_helper(now_ts)

            def finish():
                self._llm_join_in_progress = False
                self._llm_pubkey_loading = False
                self._llm_pubkey_hex = pubkey_hex or ""
                self._llm_pubkey_error = ""
                self._llm_signing = False
                if payload:
                    self._llm_signature_timestamp = str(payload.get("timestamp") or now_ts)
                    self._llm_signature_hex = str(payload.get("signature") or "").strip()
                    self._llm_signature_error = ""
                    self._schedule_llm_info_clear()
                    if self._theme == "matrix":
                        self._llm_start_matrix_rain()
                    else:
                        if self._llm_lobster_photo is None:
                            self._llm_lobster_photo = self._llm_load_lobster_icon()
                        if self._llm_lobster_photo:
                            self._llm_lobster_tick()
                        else:
                            tk.Label(
                                self._llm_content, text="(无 lobster.gif)", font=("", 12),
                                bg=self._c("bg"), fg=self._c("status_off")
                            ).pack(pady=20)
                            try:
                                self._llm_join_btn.config(text=self._t("llm_join"))
                            except tk.TclError:
                                pass
                else:
                    self._stop_llm_animation()
                    self._llm_signature_timestamp = str(now_ts)
                    self._llm_signature_hex = ""
                    self._llm_signature_error = (sign_error or "").strip()
                    try:
                        self._llm_join_btn.config(text=self._t("llm_join"))
                    except tk.TclError:
                        pass
                self._refresh_llm_pubkey_label()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _load_llm_pubkey(self, force=False):
        if self._llm_pubkey_loading:
            return
        self._cancel_llm_clear_job()
        self._llm_info_hidden = False
        if self._llm_pubkey_hex and not force:
            self._refresh_llm_pubkey_label()
            return
        self._llm_pubkey_loading = True
        self._llm_pubkey_error = ""
        self._refresh_llm_pubkey_label()

        def worker():
            pubkey_hex, error = read_slot0_pubkey_via_helper()

            def finish():
                self._llm_pubkey_loading = False
                self._llm_pubkey_hex = pubkey_hex or ""
                self._llm_pubkey_error = (error or "").strip()
                self._refresh_llm_pubkey_label()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _load_llm_signature_for_now(self):
        if self._llm_signing:
            return
        self._cancel_llm_clear_job()
        now_ts = int(time.time())
        self._llm_info_hidden = False
        self._llm_signing = True
        self._llm_signature_error = ""
        self._llm_signature_hex = ""
        self._llm_signature_timestamp = str(now_ts)
        self._refresh_llm_pubkey_label()

        def worker():
            payload, error = sign_unix_time_via_helper(now_ts)

            def finish():
                self._llm_signing = False
                if payload:
                    self._llm_signature_timestamp = str(payload.get("timestamp") or now_ts)
                    self._llm_signature_hex = str(payload.get("signature") or "").strip()
                    self._llm_signature_error = ""
                    self._schedule_llm_info_clear()
                else:
                    self._llm_signature_hex = ""
                    self._llm_signature_error = (error or "").strip()
                self._refresh_llm_pubkey_label()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()


    def _user_messages_signature(self, items):
        return tuple(
            (
                str(item.get("time") or "--:--:--"),
                str(item.get("question") or item.get("text") or ""),
                str(item.get("reply") or ""),
            )
            for item in (items if isinstance(items, list) else [])
        )


    def _render_user_messages(self):
        items = self.user_messages if isinstance(self.user_messages, list) else []
        signature = self._user_messages_signature(items)
        if signature == self._last_user_messages_signature:
            return
        self._last_user_messages_signature = signature
        for child in self._users_messages_body.winfo_children():
            child.destroy()
        if not items:
            tk.Label(
                self._users_messages_body,
                text=self._t("recent_messages_empty"),
                font=("", 11),
                bg=self._c("bg"),
                fg=self._c("fg_dim"),
                anchor="w",
                justify=tk.LEFT,
            ).pack(anchor=tk.W)
            return
        for item in items:
            card_bg = self._safe_color("box_bg", "bg", fallback_key="bg")
            if not (card_bg and str(card_bg).strip()):
                card_bg = self._tk_color("bg")
            card = tk.Frame(self._users_messages_body, bg=card_bg, bd=0, highlightthickness=0)
            card.pack(fill=tk.X, pady=(0, 6))
            meta_parts = [item.get("time") or "--:--:--"]
            tk.Label(
                card,
                text="  ".join(meta_parts),
                font=("", 9),
                bg=card_bg,
                fg=self._safe_color("fg_dim"),
                anchor="w",
                justify=tk.LEFT,
            ).pack(fill=tk.X, padx=8, pady=(6, 0))
            question_text = _single_line_message_preview(
                item.get("question") or item.get("text") or "",
                self._lang,
            )
            reply_text = _single_line_message_preview(item.get("reply") or "", self._lang)
            if question_text:
                tk.Label(
                    card,
                    text="U: " + question_text,
                    font=("", 11),
                    bg=card_bg,
                    fg=self._safe_color("msg_user_fg", "fg"),
                    anchor="w",
                    justify=tk.LEFT,
                ).pack(fill=tk.X, padx=8, pady=(2, 2))
            if reply_text:
                tk.Label(
                    card,
                    text="A: " + self._t("msg_replied_hint"),
                    font=("", 10),
                    bg=card_bg,
                    fg=self._safe_color("msg_agent_fg", "summary_skill", "fg"),
                    anchor="w",
                    justify=tk.LEFT,
                ).pack(fill=tk.X, padx=8, pady=(0, 6))
            elif question_text:
                tk.Label(
                    card,
                    text="A: ...",
                    font=("", 10),
                    bg=card_bg,
                    fg=self._safe_color("fg_dim"),
                    anchor="w",
                    justify=tk.LEFT,
                ).pack(fill=tk.X, padx=8, pady=(0, 6))

    def _render_logs_view(self):
        items = self.log_entries if isinstance(self.log_entries, list) else []
        signature = tuple(self._log_entry_key(item) for item in items)
        if signature == self._last_logs_signature and getattr(self, "_logs_canvas", None):
            return
        self._last_logs_signature = signature
        self._ensure_logs_view_widgets()
        if not items:
            if self._logs_list_wrapper and self._logs_list_wrapper.winfo_exists():
                self._logs_list_wrapper.pack_forget()
            if self._logs_empty_label and self._logs_empty_label.winfo_exists():
                self._logs_empty_label.config(
                    text=self._t("logs_empty"),
                    bg=self._c("bg"),
                    fg=self._c("fg_dim"),
                )
                if self._logs_empty_label.winfo_manager() != "pack":
                    self._logs_empty_label.pack(anchor=tk.W)
            return
        if self._logs_empty_label and self._logs_empty_label.winfo_exists() and self._logs_empty_label.winfo_manager():
            self._logs_empty_label.pack_forget()
        if self._logs_list_wrapper and self._logs_list_wrapper.winfo_exists() and self._logs_list_wrapper.winfo_manager() != "pack":
            self._logs_list_wrapper.pack(fill=tk.BOTH, expand=True)
        color_map = {
            "LLM": self._c("summary_llm"),
            "TASK": self._c("summary_task"),
            "ERROR": self._c("summary_error"),
            "ROUTING": self._c("summary_routing"),
            "TOOL": self._c("summary_tool"),
            "SKILL": self._c("summary_skill"),
            "OTHER": self._c("summary_other"),
        }
        self._ensure_logs_row_pool(len(items))
        for idx, item in enumerate(items):
            time_label = item.get("time") or "--:--:--"
            detail = item.get("detail") or item.get("raw") or ""
            kind = item.get("kind") or "OTHER"
            row = self._logs_rows[idx]
            row["frame"].config(bg=self._c("bg"), height=18)
            row["label"].config(
                text=f"{time_label} {detail}",
                bg=self._c("bg"),
                fg=color_map.get(kind, self._c("fg")),
            )
            if row["frame"].winfo_manager() != "pack":
                row["frame"].pack(fill=tk.X, pady=0)
        for row in self._logs_rows[len(items):]:
            if row["frame"].winfo_exists() and row["frame"].winfo_manager():
                row["frame"].pack_forget()
        self._refresh_logs_scroll_region()
        self._scroll_logs_to_end()

    def _ensure_logs_view_widgets(self):
        body = getattr(self, "_logs_body", None)
        if body is None or not body.winfo_exists():
            return
        if self._logs_empty_label is None or not self._logs_empty_label.winfo_exists():
            self._logs_empty_label = tk.Label(
                body,
                text=self._t("logs_empty"),
                font=("", 11),
                bg=self._c("bg"),
                fg=self._c("fg_dim"),
                anchor="w",
                justify=tk.LEFT,
            )
        if self._logs_list_wrapper is not None and self._logs_list_wrapper.winfo_exists():
            return
        self._logs_list_wrapper = tk.Frame(body, bg=self._c("bg"))
        self._logs_canvas = tk.Canvas(self._logs_list_wrapper, bg=self._c("bg"), highlightthickness=0)
        self._logs_inner = tk.Frame(self._logs_canvas, bg=self._c("bg"))
        self._logs_canvas_window_id = self._logs_canvas.create_window((0, 0), window=self._logs_inner, anchor=tk.NW)

        self._logs_inner.bind("<Configure>", lambda _event: self._refresh_logs_scroll_region())
        self._logs_canvas.bind(
            "<Configure>",
            lambda event: self._logs_canvas.itemconfig(self._logs_canvas_window_id, width=event.width)
            if self._logs_canvas and self._logs_canvas.winfo_exists()
            else None,
        )
        self._logs_canvas.pack(fill=tk.BOTH, expand=True)

        def _scroll(evt):
            if getattr(evt, "num", None) == 5 or getattr(evt, "delta", 0) == -120:
                self._logs_canvas.yview_scroll(4, "units")
            else:
                self._logs_canvas.yview_scroll(-4, "units")

        def _bind_scroll(widget):
            widget.bind("<MouseWheel>", _scroll)
            widget.bind("<Button-4>", lambda e: self._logs_canvas.yview_scroll(-4, "units"))
            widget.bind("<Button-5>", lambda e: self._logs_canvas.yview_scroll(4, "units"))

        self._logs_scroll_handler = _bind_scroll
        _bind_scroll(self._logs_list_wrapper)
        _bind_scroll(self._logs_canvas)
        _bind_scroll(self._logs_inner)
        self._logs_rows = []

    def _ensure_logs_row_pool(self, size):
        self._ensure_logs_view_widgets()
        while len(self._logs_rows) < size:
            row = tk.Frame(self._logs_inner, bg=self._c("bg"), height=18)
            row.pack_propagate(False)
            label = tk.Label(
                row,
                font=("", 9),
                bg=self._c("bg"),
                fg=self._c("fg"),
                anchor="w",
                justify=tk.LEFT,
            )
            label.pack(fill=tk.X, padx=(2, 0))
            bind_scroll = getattr(self, "_logs_scroll_handler", None)
            if bind_scroll is not None:
                bind_scroll(row)
                bind_scroll(label)
            self._logs_rows.append({"frame": row, "label": label})

    def _refresh_logs_scroll_region(self):
        if self._logs_canvas is None or not self._logs_canvas.winfo_exists():
            return
        try:
            self._logs_canvas.configure(scrollregion=self._logs_canvas.bbox("all"))
        except tk.TclError:
            pass

    def _scroll_logs_to_end(self):
        if self._logs_canvas is None or not self._logs_canvas.winfo_exists():
            return
        items = self.log_entries if isinstance(self.log_entries, list) else []
        if not items:
            return
        try:
            self._logs_canvas.update_idletasks()
            self._logs_canvas.yview_moveto(1.0)
        except tk.TclError:
            pass

    def _log_entry_key(self, item):
        if not isinstance(item, dict):
            return str(item)
        raw = (item.get("raw") or "").strip()
        if raw:
            return raw
        return "|".join(
            [
                str(item.get("time") or ""),
                str(item.get("kind") or ""),
                str(item.get("detail") or ""),
            ]
        )

    def _ordered_log_entries(self, logs):
        ordered = []
        seen = set()
        for item in reversed(logs or []):
            if not isinstance(item, dict):
                continue
            normalized = {
                "time": item.get("time") or "--:--:--",
                "kind": item.get("kind") or "OTHER",
                "detail": item.get("detail") or item.get("raw") or "",
                "raw": item.get("raw") or "",
            }
            key = self._log_entry_key(normalized)
            if key in seen:
                continue
            seen.add(key)
            ordered.append(normalized)
        if self._log_entry_limit and len(ordered) > self._log_entry_limit:
            ordered = ordered[-self._log_entry_limit:]
        return ordered

    def _cancel_log_append_job(self):
        job = getattr(self, "_log_append_job", None)
        if job is None:
            return
        try:
            self.root.after_cancel(job)
        except tk.TclError:
            pass
        self._log_append_job = None

    def _append_next_log_entry(self):
        if getattr(self, "_closing", False):
            self._cancel_log_append_job()
            return
        if not self._pending_log_entries:
            self._log_append_job = None
            if self._view_mode == "logs":
                self._render_logs_view()
            return
        self.log_entries.append(self._pending_log_entries.pop(0))
        if self._log_entry_limit and len(self.log_entries) > self._log_entry_limit:
            self.log_entries = self.log_entries[-self._log_entry_limit:]
        if self._view_mode == "logs":
            self._render_logs_view()
        self._log_append_job = self.root.after(140, self._append_next_log_entry)

    def _apply_log_entries(self, logs):
        incoming = self._ordered_log_entries(logs)
        effective_entries = list(self.log_entries) + list(self._pending_log_entries)
        if not effective_entries:
            self._pending_log_entries = []
            self.log_entries = incoming
            if self._view_mode == "logs":
                self._render_logs_view()
            return
        effective_keys = [self._log_entry_key(item) for item in effective_entries]
        incoming_keys = [self._log_entry_key(item) for item in incoming]
        overlap = 0
        max_overlap = min(len(effective_keys), len(incoming_keys))
        for size in range(max_overlap, 0, -1):
            if effective_keys[-size:] == incoming_keys[:size]:
                overlap = size
                break
        if overlap == 0:
            self._pending_log_entries = []
            self._cancel_log_append_job()
            self.log_entries = incoming
            if self._view_mode == "logs":
                self._render_logs_view()
            return
        new_items = incoming[overlap:]
        if not new_items:
            if self._log_entry_limit and len(self.log_entries) > self._log_entry_limit:
                self.log_entries = self.log_entries[-self._log_entry_limit:]
            return
        self._pending_log_entries.extend(new_items)
        if self._log_append_job is None:
            self._append_next_log_entry()

    def _rebuild_ui(self, reopen_settings_category=None):
        """主题切换后重建界面。"""
        self._prepare_for_ui_rebuild()
        for w in self.root.winfo_children():
            w.destroy()
        self._i18n.clear()
        self.gif_frames.clear()
        self.gif_delays.clear()
        self._build_ui()
        self._schedule_refresh()
        self._start_weather_refresh_cycle()
        self._tick_time()
        if self.gif_frames:
            self._animate_gif()
        self._refresh_health_once()
        if reopen_settings_category is not None:
            self._switch_view("settings")
            if reopen_settings_category == "menu":
                self._show_settings_menu()
            else:
                self._show_settings_category(reopen_settings_category)

    def _apply_settings_changes(self, reopen_settings_category=None):
        old_lang = self._lang
        old_theme = self._theme
        old_show_messages = self._show_messages_page
        old_show_logs = self._show_logs_page
        old_show_gallery = self._show_gallery_page
        old_show_skills = self._show_skills_page
        old_show_weather = self._show_weather_page
        old_show_stock = self._show_stock_page
        old_show_us_stock = self._show_us_stock_page
        old_show_crypto = self._show_crypto_page
        self._lang = self._settings_lang_var.get()
        new_theme = self._settings_theme_var.get()
        self._show_messages_page = bool(self._settings_show_messages_var.get())
        self._show_logs_page = bool(self._settings_show_logs_var.get())
        self._show_gallery_page = bool(self._settings_show_gallery_var.get())
        self._show_skills_page = bool(self._settings_show_skills_var.get())
        self._show_weather_page = bool(self._settings_show_weather_var.get())
        self._show_stock_page = bool(self._settings_show_stock_var.get())
        self._show_us_stock_page = bool(self._settings_show_us_stock_var.get())
        self._show_crypto_page = bool(self._settings_show_crypto_var.get())
        save_lang(self._lang)
        save_messages_page_visible(self._show_messages_page)
        save_logs_page_visible(self._show_logs_page)
        save_gallery_page_visible(self._show_gallery_page)
        save_skills_page_visible(self._show_skills_page)
        save_weather_page_visible(self._show_weather_page)
        save_stock_page_visible(self._show_stock_page)
        save_us_stock_page_visible(self._show_us_stock_page)
        save_crypto_page_visible(self._show_crypto_page)
        if new_theme != self._theme:
            self._theme = new_theme
            save_theme(self._theme)
            self._rebuild_ui(
                reopen_settings_category=(
                    reopen_settings_category
                    if reopen_settings_category is not None
                    else getattr(self, "_settings_category", "menu")
                )
            )
            return
        if self._lang != old_lang:
            self._apply_lang()
            self._prepare_settings_view()
            self._fetch_weather(force=True)
        self._refresh_settings_choice_labels()
        if (
            self._show_messages_page != old_show_messages
            or self._show_logs_page != old_show_logs
            or self._show_gallery_page != old_show_gallery
            or self._show_skills_page != old_show_skills
            or self._show_weather_page != old_show_weather
            or self._show_stock_page != old_show_stock
            or self._show_us_stock_page != old_show_us_stock
            or self._show_crypto_page != old_show_crypto
            or self._theme != old_theme
        ):
            self._refresh_health_once()
        if self._view_mode == "settings":
            category = (
                reopen_settings_category
                if reopen_settings_category is not None
                else getattr(self, "_settings_category", "menu")
            )
            if category == "menu":
                self._show_settings_menu()
            else:
                self._show_settings_category(category)

    def _on_settings_restart(self):
        """后台执行 rustclaw -restart release all；15 秒内按钮禁用并显示「重启中.....」。"""
        btn = self._settings_restart_btn
        if btn["state"] == tk.DISABLED:
            return
        btn.config(state=tk.DISABLED, text=self._t("restarting"))
        try:
            subprocess.Popen(
                ["rustclaw", "-restart", "release", "all"],
                stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
                start_new_session=True,
            )
        except FileNotFoundError:
            pass
        except Exception:
            pass

        def reenable():
            if getattr(self, "_closing", False):
                return
            b = getattr(self, "_settings_restart_btn", None)
            if b and b.winfo_exists():
                try:
                    b.config(state=tk.NORMAL, text=self._t("restart"))
                except tk.TclError:
                    pass

        self._cancel_job("_settings_restart_job")
        self._settings_restart_job = self.root.after(15000, reenable)

    def _on_settings_reset_admin_login(self):
        btn = self._settings_reset_admin_btn
        if btn["state"] == tk.DISABLED:
            return
        btn.config(state=tk.DISABLED, text=self._t("resetting_admin_login"))
        self._settings_reset_status_var.set(self._t("resetting_admin_login"))

        def worker():
            ok, err = reset_admin_login_account(
                username="rustclaw",
                password="rustclaw123456",
            )

            def finish():
                b = getattr(self, "_settings_reset_admin_btn", None)
                if b and b.winfo_exists():
                    try:
                        b.config(state=tk.NORMAL, text=self._t("reset_admin_login"))
                    except tk.TclError:
                        pass
                if ok:
                    self._settings_reset_status_var.set(self._t("reset_admin_login_success"))
                    try:
                        from tkinter import messagebox

                        messagebox.showinfo(
                            self._t("reset_admin_login_dialog_title"),
                            self._t("reset_admin_login_dialog_body"),
                        )
                    except Exception:
                        pass
                else:
                    self._settings_reset_status_var.set(
                        self._t("reset_admin_login_failed").format(error=(err or "unknown error"))
                    )

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _toggle_view(self):
        """左滑/下一页：按当前启用的页面顺序循环切换。"""
        if self._view_mode == "wifi":
            self._switch_view("settings")
            self._show_settings_category("system")
            return
        if self._view_mode == "weather" and self._weather_return_view_on_swipe:
            target_mode = self._weather_return_view_on_swipe
            self._weather_return_view_on_swipe = None
            self._switch_view(target_mode)
            return
        if self._overview_return_on_swipe and self._view_mode != "overview":
            self._switch_view("overview")
            return
        modes = self._visible_view_modes()
        try:
            idx = modes.index(self._view_mode)
        except ValueError:
            idx = 0
        self._switch_view(modes[(idx + 1) % len(modes)])

    def _go_prev_view(self):
        """右滑/上一页：按当前启用的页面顺序循环切换。"""
        if self._view_mode == "wifi":
            self._switch_view("settings")
            self._show_settings_category("system")
            return
        if self._view_mode == "weather" and self._weather_return_view_on_swipe:
            target_mode = self._weather_return_view_on_swipe
            self._weather_return_view_on_swipe = None
            self._switch_view(target_mode)
            return
        if self._overview_return_on_swipe and self._view_mode != "overview":
            self._switch_view("overview")
            return
        modes = self._visible_view_modes()
        try:
            idx = modes.index(self._view_mode)
        except ValueError:
            idx = 0
        self._switch_view(modes[(idx - 1) % len(modes)])

    def _show_gallery(self):
        """NNI分布式模型页：Matrix 主题下无标题无按钮、矩阵雨占满屏并自动开始；非 Matrix 为标题+加入/停止+龙虾图。"""
        self._teardown_gallery_view()
        for w in self.gallery_frame.winfo_children():
            w.destroy()
        self._llm_per_line = max(6, (W - 32) // 28)
        self._llm_max_rows = 6
        self._llm_content = tk.Frame(self.gallery_frame, bg=self._c("bg"))
        self._llm_lobster_photo = None
        self._llm_dot_labels = []
        if self._theme == "matrix":
            title_row = tk.Frame(self.gallery_frame, bg=self._c("bg"))
            title_row.pack(fill=tk.X, pady=(0, 6))
            tk.Label(
                title_row, text=self._t("llm_title"), font=("", 14, "bold"),
                bg=self._c("bg"), fg=self._c("fg")
            ).pack(side=tk.LEFT)
            self._llm_join_btn = tk.Button(
                title_row, text=self._t("llm_join"), font=("", 11), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"),
                activebackground=self._c("button_active_bg"), activeforeground=self._c("fg"), cursor="hand2",
                command=self._on_llm_join_click, padx=12, pady=4
            )
            self._llm_join_btn.pack(side=tk.RIGHT)
            llm_info = tk.Frame(self.gallery_frame, bg=self._c("box_bg"), padx=6, pady=4)
            self._llm_info_frame = llm_info
            self._llm_info_pady = (0, 6)
            llm_info.pack(fill=tk.X, pady=self._llm_info_pady)
            self._llm_pubkey_label = tk.Label(
                llm_info,
                text=self._format_llm_pubkey_text(),
                font=("DejaVu Sans Mono", 8),
                bg=self._c("box_bg"),
                fg=self._c("fg"),
                anchor="w",
                justify=tk.LEFT,
                wraplength=W - 32,
            )
            self._llm_pubkey_label.pack(fill=tk.X)
            self._clear_llm_info_display()
            self._llm_content.pack(fill=tk.BOTH, expand=True)
            return
        # 非 Matrix：标题行 + 加入/停止按钮，再内容区
        title_row = tk.Frame(self.gallery_frame, bg=self._c("bg"))
        title_row.pack(fill=tk.X, pady=(0, 8))
        tk.Label(
            title_row, text=self._t("llm_title"), font=("", 14, "bold"),
            bg=self._c("bg"), fg=self._c("fg")
        ).pack(side=tk.LEFT)
        self._llm_join_btn = tk.Button(
            title_row, text=self._t("llm_join"), font=("", 11), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"), activeforeground=self._c("fg"), cursor="hand2",
            command=self._on_llm_join_click, padx=12, pady=4
        )
        self._llm_join_btn.pack(side=tk.RIGHT)
        llm_info = tk.Frame(self.gallery_frame, bg=self._c("box_bg"), padx=6, pady=4)
        self._llm_info_frame = llm_info
        self._llm_info_pady = (0, 8)
        llm_info.pack(fill=tk.X, pady=self._llm_info_pady)
        self._llm_pubkey_label = tk.Label(
            llm_info,
            text=self._format_llm_pubkey_text(),
            font=("DejaVu Sans Mono", 8),
            bg=self._c("box_bg"),
            fg=self._c("fg"),
            anchor="w",
            justify=tk.LEFT,
            wraplength=W - 32,
        )
        self._llm_pubkey_label.pack(fill=tk.X)
        self._clear_llm_info_display()
        self._llm_content.pack(fill=tk.BOTH, expand=True)

    def _llm_start_matrix_rain(self):
        """Matrix 主题：在 _llm_content 内创建多列竖排并启动动画，占满内容区（含右侧）。"""
        # 等宽 12 约 7～8px/字，列数按可用宽度算满（gallery_frame 有 padx 约 16）
        avail_w = W - 24
        num_cols = max(12, avail_w // 8)
        line_height_px = 14
        self._llm_matrix_max_rows = max(10, (H - 52) // line_height_px)
        self._llm_matrix_cols = []
        mono_font = ("DejaVu Sans Mono", 11)
        for _ in range(num_cols):
            lbl = tk.Label(
                self._llm_content, text="", font=mono_font,
                bg=self._c("bg"), fg=self._c("fg"), justify=tk.LEFT
            )
            lbl.pack(side=tk.LEFT, padx=0, pady=2)
            self._llm_matrix_cols.append({
                "chars": [],
                "interval": random.uniform(0.12, 0.55),
                "last_add": 0.0,
                "label": lbl,
            })
        self._llm_matrix_tick()

    def _on_llm_join_click(self):
        """加入/停止：未运行时开始画龙虾点（每 0.5 秒一个），运行时停止并恢复按钮为加入。"""
        if getattr(self, "_closing", False) or self._view_mode != "gallery":
            return
        if self._llm_join_in_progress:
            return
        if self._llm_lobster_job:
            self._stop_llm_animation()
            self._clear_llm_info_display()
            try:
                self._llm_join_btn.config(text=self._t("llm_join"))
            except tk.TclError:
                pass
            return
        for w in self._llm_content.winfo_children():
            w.destroy()
        self._llm_dot_labels.clear()
        self._llm_lobster_count = 0
        self._llm_join_btn.config(text=self._t("llm_stop"))
        self._start_llm_pubkey_and_sign_flow()

    def _llm_load_lobster_icon(self):
        """从 scripts/assets 加载 lobster.png 或 lobster.gif，缩成小图标，透明处叠到深色底（无白底）。"""
        assets = find_assets()
        for name in ("lobster.png", "lobster.gif"):
            path = os.path.join(assets, name)
            if not os.path.isfile(path):
                continue
            try:
                from PIL import Image, ImageTk
                with Image.open(path) as img:
                    icon = img.convert("RGBA")
                    icon = icon.resize((22, 22), Image.Resampling.LANCZOS)
                bg_rgb = self._c("bg_rgb")
                if isinstance(bg_rgb, (list, tuple)) and len(bg_rgb) >= 3:
                    bg_rgb = tuple(int(x) for x in bg_rgb[:3])
                else:
                    bg_rgb = (0x1a, 0x1a, 0x2e)
                out = Image.new("RGB", icon.size, bg_rgb)
                out.paste(icon, mask=icon.split()[3])
                return ImageTk.PhotoImage(out)
            except Exception:
                try:
                    photo = tk.PhotoImage(file=path)
                    photo = photo.subsample(2, 2)
                    return photo
                except Exception:
                    pass
        return None

    def _llm_matrix_tick(self):
        """Matrix 主题：多列竖排随机字符，每列速度不同步；满 6 行清空该列；单次定时驱动所有列。"""
        if getattr(self, "_closing", False) or self._view_mode != "gallery":
            self._llm_lobster_job = None
            btn = getattr(self, "_llm_join_btn", None)
            if btn and btn.winfo_exists():
                try:
                    btn.config(text=self._t("llm_join"))
                except tk.TclError:
                    pass
            return
        now = time.time()
        max_rows = getattr(self, "_llm_matrix_max_rows", 16)
        cols = getattr(self, "_llm_matrix_cols", [])
        if not cols:
            self._llm_lobster_job = None
            return
        for col in cols:
            if now - col["last_add"] < col["interval"]:
                continue
            col["last_add"] = now
            chars = col["chars"]
            if len(chars) >= max_rows:
                chars.clear()
            chars.append(random.choice(MATRIX_CHARS))
            lbl = col.get("label")
            if lbl and lbl.winfo_exists():
                try:
                    lbl.config(text="\n".join(chars))
                except tk.TclError:
                    pass
        # 按「下一列到期」调度，避免固定 80ms 空转（节奏与随机间隔不变）
        end = time.time()
        min_rem = None
        for col in cols:
            rem = col["interval"] - (end - col["last_add"])
            if min_rem is None or rem < min_rem:
                min_rem = rem
        if min_rem is None:
            delay_ms = 80
        elif min_rem <= 0:
            delay_ms = 16
        else:
            delay_ms = max(80, min(250, int(min_rem * 1000)))
        self._llm_lobster_job = self.root.after(delay_ms, self._llm_matrix_tick)

    def _llm_lobster_tick(self):
        """每 0.5 秒多画一个龙虾小图，按行网格排；画满 6 行后清空重新画；切页后也继续画。"""
        if getattr(self, "_closing", False) or self._view_mode != "gallery":
            self._llm_lobster_job = None
            try:
                self._llm_join_btn.config(text=self._t("llm_join"))
            except tk.TclError:
                pass
            return
        per = getattr(self, "_llm_per_line", 10)
        max_count = self._llm_max_rows * per
        if self._llm_lobster_count >= max_count:
            for w in self._llm_content.winfo_children():
                w.destroy()
            self._llm_dot_labels.clear()
            self._llm_lobster_count = 0
        self._llm_lobster_count += 1
        r, c = (self._llm_lobster_count - 1) // per, (self._llm_lobster_count - 1) % per
        photo = getattr(self, "_llm_lobster_photo", None)
        if photo:
            lbl = tk.Label(
                self._llm_content, image=photo, bg=self._c("bg")
            )
            lbl.grid(row=r, column=c, padx=2, pady=2)
            self._llm_dot_labels.append(lbl)
        self._llm_lobster_job = self.root.after(500, self._llm_lobster_tick)

    def _start_weather_refresh_cycle(self):
        self._schedule_weather_refresh()
        if self._weather_data is None:
            self._fetch_weather(force=True)

    def _schedule_weather_refresh(self):
        self._cancel_job("_weather_refresh_job")
        if getattr(self, "_closing", False):
            return
        delay_ms = max(1000, WEATHER_REFRESH_SEC * 1000)
        self._weather_refresh_job = self.root.after(delay_ms, self._weather_refresh_due)

    def _weather_refresh_due(self):
        self._weather_refresh_job = None
        if getattr(self, "_closing", False):
            return
        self._fetch_weather(force=True)
        self._schedule_weather_refresh()

    def _weather_reenable_refresh_btn(self):
        self._weather_manual_refresh_job = None
        if getattr(self, "_closing", False) or self._view_mode != "weather":
            return
        btn = getattr(self, "_weather_refresh_btn", None)
        if btn and btn.winfo_exists():
            try:
                btn.config(state=tk.NORMAL)
            except tk.TclError:
                pass

    def _select_weather_detail(self, offset=0):
        try:
            self._weather_selected_offset = max(0, int(offset))
        except Exception:
            self._weather_selected_offset = 0
        if getattr(self, "_closing", False):
            return
        if self._view_mode == "weather":
            self._show_weather()

    def _fetch_weather(self, force=False):
        if getattr(self, "_closing", False):
            return
        if self._weather_loading:
            return
        self._weather_loading = True
        btn = getattr(self, "_weather_refresh_btn", None)
        if btn and btn.winfo_exists():
            try:
                btn.config(state=tk.DISABLED)
            except tk.TclError:
                pass
        self._cancel_job("_weather_manual_refresh_job")
        self._weather_manual_refresh_job = self.root.after(3000, self._weather_reenable_refresh_btn)

        def worker():
            weather, error = fetch_today_weather(self._lang)

            def finish():
                self._weather_loading = False
                if weather is not None:
                    self._weather_data = weather
                    self._weather_error = ""
                    max_offset = max(0, len(weather.get("details") or []) - 1)
                    self._weather_selected_offset = min(
                        max(0, int(getattr(self, "_weather_selected_offset", 0))),
                        max_offset,
                    )
                else:
                    self._weather_error = (error or "").strip()
                    self._weather_selected_offset = 0
                self._refresh_weather_icon_display()
                self._refresh_dashboard_overview_if_needed()
                if self._view_mode == "weather":
                    self._show_weather()

            self._post_ui(finish)

        threading.Thread(target=worker, daemon=True).start()

    def _show_weather(self):
        for w in self.weather_frame.winfo_children():
            w.destroy()

        def bind_reset_today(widget):
            widget.bind("<Button-1>", lambda _evt: self._select_weather_detail(0))

        def metric_cell(parent, title, value, right_gap=True):
            box_border = self._c("box_border")
            box_bg = self._c("box_bg")
            cell_gap = 6
            cell_w = (W - 16 - 8 - cell_gap) // 2
            box = tk.Frame(parent, bg=box_border, padx=2, pady=2, width=cell_w)
            box.pack(side=tk.LEFT, fill=tk.BOTH, expand=True, padx=(0, cell_gap if right_gap else 0))
            inner = tk.Frame(box, bg=box_bg, padx=5, pady=3)
            inner.pack(fill=tk.BOTH, expand=True)
            tk.Label(
                inner,
                text=f"{title}: {value}",
                font=("", 9, "bold"),
                bg=box_bg,
                fg=self._c("fg"),
                anchor="center",
                justify=tk.CENTER,
                wraplength=160,
            ).pack(fill=tk.BOTH, expand=True)

        self._weather_refresh_btn = tk.Button(
            self.weather_frame,
            text=self._t("refresh"),
            font=("", 9),
            relief=tk.FLAT,
            borderwidth=0,
            highlightthickness=0,
            bg=self._c("button_bg"),
            fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"),
            activeforeground=self._c("fg"),
            disabledforeground=self._c("fg_dim"),
            takefocus=0,
            command=lambda: self._fetch_weather(force=True),
            padx=6,
            pady=1,
        )
        if self._weather_loading:
            self._weather_refresh_btn.config(state=tk.DISABLED)

        weather = self._weather_data or {}
        bind_reset_today(self.weather_frame)

        if not weather:
            empty_text = self._t("weather_loading")
            if self._weather_error:
                empty_text = self._t("weather_error").format(error=self._weather_error)
            tk.Label(
                self.weather_frame,
                text=empty_text,
                font=("", 11),
                bg=self._c("bg"),
                fg=self._c("status_err") if self._weather_error else self._c("fg_dim"),
                anchor="w",
                justify=tk.LEFT,
                wraplength=440,
            ).pack(fill=tk.X, pady=(14, 0))
            return

        details = weather.get("details") or []
        selected_offset = min(
            max(0, int(getattr(self, "_weather_selected_offset", 0))),
            max(0, len(details) - 1),
        )
        selected = details[selected_offset] if details else weather
        location_text = str(selected.get("location") or weather.get("location") or "--").strip() or "--"
        source_text = str(weather.get("source") or "--").strip() or "--"
        hero = tk.Frame(self.weather_frame, bg=self._c("box_border"), padx=2, pady=2)
        hero.pack(fill=tk.X, pady=(0, 4))
        bind_reset_today(hero)
        hero_inner = tk.Frame(hero, bg=self._c("box_bg"), padx=7, pady=5)
        hero_inner.pack(fill=tk.BOTH, expand=True)
        bind_reset_today(hero_inner)
        temp_row = tk.Frame(hero_inner, bg=self._c("box_bg"))
        temp_row.pack(fill=tk.X)
        bind_reset_today(temp_row)
        tk.Label(
            temp_row,
            text=str(selected.get("temperature") or "--"),
            font=("", 20, "bold"),
            bg=self._c("box_bg"),
            fg=self._c("accent"),
            anchor="w",
        ).pack(side=tk.LEFT)
        tk.Label(
            temp_row,
            text=str(selected.get("description") or "--"),
            font=("", 10, "bold"),
            bg=self._c("box_bg"),
            fg=self._c("fg"),
            anchor="e",
            justify=tk.RIGHT,
            wraplength=200,
        ).pack(side=tk.RIGHT)
        meta_row = tk.Frame(hero_inner, bg=self._c("box_bg"))
        meta_row.pack(fill=tk.X, pady=(1, 0))
        bind_reset_today(meta_row)
        tk.Label(
            meta_row,
            text=location_text,
            font=("", 8),
            bg=self._c("box_bg"),
            fg=self._c("fg_dim"),
            anchor="w",
        ).pack(side=tk.LEFT, fill=tk.X, expand=True)
        tk.Label(
            meta_row,
            text=(
                f"{str(selected.get('day') or '--')}   "
                f"{self._t('weather_updated')}: {str(weather.get('updated_at') or '--')}   "
                f"{self._t('weather_source')}: {source_text}"
            ),
            font=("", 8),
            bg=self._c("box_bg"),
            fg=self._c("fg_dim"),
            anchor="e",
        ).pack(side=tk.RIGHT)

        row1 = tk.Frame(self.weather_frame, bg=self._c("bg"))
        row1.pack(fill=tk.X, pady=(0, 3))
        bind_reset_today(row1)
        metric_cell(row1, self._t("weather_feels_like"), str(selected.get("feels_like") or "--"), right_gap=True)
        metric_cell(row1, self._t("weather_high_low"), str(selected.get("high_low") or "--"), right_gap=False)

        row2 = tk.Frame(self.weather_frame, bg=self._c("bg"))
        row2.pack(fill=tk.X, pady=(0, 3))
        bind_reset_today(row2)
        metric_cell(row2, self._t("weather_humidity"), str(selected.get("humidity") or "--"), right_gap=True)
        metric_cell(row2, self._t("weather_wind"), str(selected.get("wind") or "--"), right_gap=False)

        forecast_items = weather.get("forecast") or []
        if forecast_items:
            forecast_row = tk.Frame(self.weather_frame, bg=self._c("bg"))
            forecast_row.pack(fill=tk.X, pady=(0, 3))
            bind_reset_today(forecast_row)
            for idx, item in enumerate(forecast_items[:3]):
                item_offset = int(item.get("offset") or (idx + 1))
                is_selected = item_offset == selected_offset
                card = tk.Frame(
                    forecast_row,
                    bg=self._c("accent") if is_selected else self._c("box_border"),
                    padx=2,
                    pady=2,
                )
                card.pack(
                    side=tk.LEFT,
                    fill=tk.BOTH,
                    expand=True,
                    padx=(0, 4 if idx < min(2, len(forecast_items[:3]) - 1) else 0),
                )
                inner = tk.Frame(card, bg=self._c("box_bg"), padx=3, pady=2)
                inner.pack(fill=tk.BOTH, expand=True)
                day_label = tk.Label(
                    inner,
                    text=str(item.get("day") or "--"),
                    font=("", 8),
                    bg=self._c("box_bg"),
                    fg=self._c("accent") if is_selected else self._c("fg_dim"),
                    anchor="center",
                )
                day_label.pack()
                forecast_mid = tk.Frame(inner, bg=self._c("box_bg"))
                forecast_mid.pack(pady=(1, 0))
                icon_label = tk.Label(
                    forecast_mid,
                    text=str(item.get("icon") or "◌"),
                    font=("", 14),
                    bg=self._c("box_bg"),
                    fg=self._c("accent"),
                )
                icon_label.pack(side=tk.LEFT, padx=(0, 4))
                temp_label = tk.Label(
                    forecast_mid,
                    text=str(item.get("high_low") or "--"),
                    font=("", 8, "bold"),
                    bg=self._c("box_bg"),
                    fg=self._c("fg"),
                )
                temp_label.pack(side=tk.LEFT)
                for widget in (card, inner, day_label, forecast_mid, icon_label, temp_label):
                    widget.bind("<Button-1>", lambda _evt, off=item_offset: self._select_weather_detail(off))

        info_row = tk.Frame(self.weather_frame, bg=self._c("bg"))
        info_row.pack(fill=tk.X, pady=(0, 2))
        bind_reset_today(info_row)
        tk.Label(
            info_row,
            text=(
                f"{self._t('weather_rain')}: {str(selected.get('rain') or '--')}   "
                f"{self._t('weather_sunrise')}: {str(selected.get('sunrise') or '--')}   "
                f"{self._t('weather_sunset')}: {str(selected.get('sunset') or '--')}"
            ),
            font=("", 8),
            bg=self._c("bg"),
            fg=self._c("fg"),
            anchor="w",
            justify=tk.LEFT,
            wraplength=440,
        ).pack(fill=tk.X)

        if self._weather_error:
            tk.Label(
                self.weather_frame,
                text=self._t("weather_error").format(error=self._weather_error),
                font=("", 8),
                bg=self._c("bg"),
                fg=self._c("status_err"),
                anchor="w",
                justify=tk.LEFT,
                wraplength=440,
            ).pack(fill=tk.X, pady=(3, 0))

        action_row = tk.Frame(self.weather_frame, bg=self._c("bg"))
        action_row.pack(fill=tk.X, pady=(2, 0))
        bind_reset_today(action_row)
        self._weather_refresh_btn.pack(in_=action_row, side=tk.RIGHT)

    def _show_crypto(self):
        for w in self.crypto_frame.winfo_children():
            w.destroy()
        self._crypto_items, self._crypto_refresh_sec = _load_small_screen_crypto_config()
        title_row = tk.Frame(self.crypto_frame, bg=self._c("bg"))
        title_row.pack(fill=tk.X, pady=(0, 6))
        tk.Label(
            title_row, text="CRYPTO", font=("DejaVu Sans", 14, "bold"),
            bg=self._c("bg"), fg=self._c("fg")
        ).pack(side=tk.LEFT)
        self._crypto_refresh_btn = tk.Button(
            title_row, text=self._t("refresh"), font=("", 10), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"), activeforeground=self._c("fg"), cursor="hand2",
            command=self._crypto_manual_refresh, padx=8, pady=2
        )
        self._crypto_refresh_btn.pack(side=tk.RIGHT, padx=(6, 0))
        tk.Label(
            title_row, text=self._t("crypto_refresh_hint").format(sec=self._crypto_refresh_sec), font=("", 10),
            bg=self._c("bg"), fg=self._c("foot_fg")
        ).pack(side=tk.RIGHT)
        box_bg = self._c("box_bg")
        box_border = self._c("box_border")
        cell_gap = 6
        cell_h = 28
        cell_w = (W - 16 - 8 - cell_gap) // 2
        self._crypto_vars = {}
        if not self._crypto_items:
            tk.Label(
                self.crypto_frame, text=self._t("crypto_empty"), font=("", 12),
                bg=self._c("bg"), fg=self._c("status_off")
            ).pack(anchor=tk.W, pady=(12, 0))
            return
        for i in range(0, len(self._crypto_items), 2):
            row = tk.Frame(self.crypto_frame, bg=self._c("bg"), height=cell_h)
            row.pack(fill=tk.X, pady=2)
            row.pack_propagate(False)
            for j in range(2):
                if i + j >= len(self._crypto_items):
                    break
                name = self._crypto_items[i + j]["name"]
                var = tk.StringVar(value="--")
                self._crypto_vars[name] = var
                f = tk.Frame(row, bg=box_border, padx=3, pady=2, width=cell_w, height=cell_h - 4)
                f.pack_propagate(False)
                f.pack(side=tk.LEFT, padx=(0, cell_gap if j == 0 else 0))
                inner = tk.Frame(f, bg=box_bg, padx=6, pady=2)
                inner.pack(fill=tk.BOTH, expand=True)
                tk.Label(inner, text=name + " ", font=("", 11), bg=box_bg, fg=self._c("fg_dim")).pack(side=tk.LEFT)
                tk.Label(inner, textvariable=var, font=("", 12, "bold"), bg=box_bg, fg=self._c("fg")).pack(side=tk.RIGHT)
        def _fetch_and_update():
            prices = fetch_crypto_prices(self._crypto_items)
            self._post_ui(lambda: self._update_crypto_prices(prices))
        threading.Thread(target=_fetch_and_update, daemon=True).start()
        self._crypto_job = self.root.after(self._crypto_refresh_sec * 1000, self._crypto_refresh_loop)

    def _update_crypto_prices(self, prices):
        if getattr(self, "_closing", False) or prices is None or self._view_mode != "crypto":
            return
        for name, var in self._crypto_vars.items():
            var.set(prices.get(name, "--"))

    def _crypto_manual_refresh(self):
        if getattr(self, "_closing", False) or self._view_mode != "crypto":
            return
        btn = getattr(self, "_crypto_refresh_btn", None)
        if btn and btn.winfo_exists():
            btn.config(state=tk.DISABLED)
            self.root.after(3000, self._crypto_reenable_refresh_btn)

        def _fetch():
            if getattr(self, "_closing", False):
                return
            prices = fetch_crypto_prices(getattr(self, "_crypto_items", None))
            try:
                self._post_ui(lambda: self._update_crypto_prices(prices))
            except Exception:
                pass

        threading.Thread(target=_fetch, daemon=True).start()

    def _crypto_reenable_refresh_btn(self):
        """3 秒后恢复刷新键可点。"""
        if getattr(self, "_closing", False) or self._view_mode != "crypto":
            return
        btn = getattr(self, "_crypto_refresh_btn", None)
        if btn and btn.winfo_exists():
            try:
                btn.config(state=tk.NORMAL)
            except tk.TclError:
                pass

    def _crypto_refresh_loop(self):
        if getattr(self, "_closing", False) or self._view_mode != "crypto":
            self._crypto_job = None
            return
        def _fetch():
            if getattr(self, "_closing", False):
                return
            prices = fetch_crypto_prices(getattr(self, "_crypto_items", None))
            self._post_ui(lambda: self._update_crypto_prices(prices))
        threading.Thread(target=_fetch, daemon=True).start()
        self._crypto_job = self.root.after(self._crypto_refresh_sec * 1000, self._crypto_refresh_loop)

    def _show_stock(self):
        for w in self.stock_frame.winfo_children():
            w.destroy()
        self._stock_items, self._stock_refresh_sec = _load_small_screen_stock_config()
        title_row = tk.Frame(self.stock_frame, bg=self._c("bg"))
        title_row.pack(fill=tk.X, pady=(0, 6))
        tk.Label(
            title_row, text="A-SHARES", font=("DejaVu Sans", 14, "bold"),
            bg=self._c("bg"), fg=self._c("fg")
        ).pack(side=tk.LEFT)
        self._stock_refresh_btn = tk.Button(
            title_row, text=self._t("refresh"), font=("", 10), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"), activeforeground=self._c("fg"), cursor="hand2",
            command=self._stock_manual_refresh, padx=8, pady=2
        )
        self._stock_refresh_btn.pack(side=tk.RIGHT, padx=(6, 0))
        tk.Label(
            title_row, text=self._t("stock_refresh_hint").format(sec=self._stock_refresh_sec), font=("", 10),
            bg=self._c("bg"), fg=self._c("foot_fg")
        ).pack(side=tk.RIGHT)

        items = [
            {"title": symbol, "price": "--", "pct": "--", "meta1": "...", "meta2": ""}
            for symbol in [item.get("name") or item.get("code") or "--" for item in self._stock_items]
        ]
        self._stock_cards = []
        list_wrapper = tk.Frame(self.stock_frame, bg=self._c("bg"))
        list_wrapper.pack(fill=tk.BOTH, expand=True)
        canvas = tk.Canvas(list_wrapper, bg=self._c("bg"), highlightthickness=0)
        scrollbar = tk.Scrollbar(list_wrapper)
        inner = tk.Frame(canvas, bg=self._c("bg"))
        win_id = canvas.create_window((0, 0), window=inner, anchor=tk.NW)

        def _on_inner_configure(_):
            canvas.configure(scrollregion=canvas.bbox("all"))

        def _on_canvas_configure(evt):
            canvas.itemconfig(win_id, width=evt.width)

        inner.bind("<Configure>", _on_inner_configure)
        canvas.bind("<Configure>", _on_canvas_configure)
        scrollbar.pack(side=tk.RIGHT, fill=tk.Y)
        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        self._style_scrollbar(scrollbar)

        def _after_scroll():
            canvas.update_idletasks()

        def _scrollbar_cmd(*args):
            canvas.yview(*args)
            _after_scroll()

        canvas.configure(yscrollcommand=scrollbar.set)
        scrollbar.configure(command=_scrollbar_cmd)

        if not items:
            tk.Label(
                inner, text=self._t("stock_empty"), font=("", 12),
                bg=self._c("bg"), fg=self._c("status_off")
            ).pack(anchor=tk.W, pady=(12, 0))
            return

        box_bg = self._c("box_bg")
        box_border = self._c("box_border")
        for item in items:
            card = tk.Frame(inner, bg=box_border, padx=2, pady=2)
            card.pack(fill=tk.X, pady=2)
            inner_card = tk.Frame(card, bg=box_bg, padx=8, pady=4)
            inner_card.pack(fill=tk.BOTH, expand=True)
            title_var = tk.StringVar(value=item.get("title") or "--")
            price_var = tk.StringVar(value=item.get("price") or "--")
            pct_var = tk.StringVar(value=item.get("pct") or "--")
            detail_var = tk.StringVar(value=item.get("meta1") or "")
            top_row = tk.Frame(inner_card, bg=box_bg)
            top_row.pack(fill=tk.X)
            tk.Label(top_row, textvariable=title_var, font=("", 10), bg=box_bg, fg=self._c("fg_dim"), anchor=tk.W).pack(side=tk.LEFT, fill=tk.X, expand=True)
            price_label = tk.Label(top_row, textvariable=price_var, font=("", 12, "bold"), bg=box_bg, fg=self._c("fg"))
            price_label.pack(side=tk.RIGHT)
            pct_label = tk.Label(top_row, textvariable=pct_var, font=("", 10, "bold"), bg=box_bg, fg=self._c("fg"))
            pct_label.pack(side=tk.RIGHT, padx=(0, 8))
            tk.Label(inner_card, textvariable=detail_var, font=("", 9), bg=box_bg, fg=self._c("fg"), anchor=tk.W, justify=tk.LEFT).pack(anchor=tk.W, pady=(1, 0))
            self._stock_cards.append({
                "title": title_var,
                "price": price_var,
                "pct": pct_var,
                "detail": detail_var,
                "pct_label": pct_label,
                "price_label": price_label,
            })

        _scroll_units = 3

        def _scroll(evt):
            if getattr(evt, "num", None) == 5 or getattr(evt, "delta", 0) == -120:
                canvas.yview_scroll(_scroll_units, "units")
            else:
                canvas.yview_scroll(-_scroll_units, "units")
            _after_scroll()

        _drag_y_root = [None]

        def _on_drag_start(evt):
            _drag_y_root[0] = evt.y_root

        def _on_drag_motion(evt):
            if _drag_y_root[0] is not None:
                dy = evt.y_root - _drag_y_root[0]
                step = max(-12, min(12, int(dy)))
                if step != 0:
                    canvas.yview_scroll(step, "units")
                    _after_scroll()
                _drag_y_root[0] = evt.y_root

        def _on_drag_end(_evt):
            _drag_y_root[0] = None

        def _bind_scroll(widget):
            widget.bind("<MouseWheel>", _scroll)
            widget.bind("<Button-4>", lambda e: (canvas.yview_scroll(-_scroll_units, "units"), _after_scroll()))
            widget.bind("<Button-5>", lambda e: (canvas.yview_scroll(_scroll_units, "units"), _after_scroll()))
            widget.bind("<Button-1>", _on_drag_start)
            widget.bind("<B1-Motion>", _on_drag_motion)
            widget.bind("<ButtonRelease-1>", _on_drag_end)

        _bind_scroll(list_wrapper)
        _bind_scroll(canvas)
        _bind_scroll(inner)
        _bind_scroll(scrollbar)
        for row in inner.winfo_children():
            _bind_scroll(row)
            for child in row.winfo_children():
                _bind_scroll(child)
                for grand in child.winfo_children():
                    _bind_scroll(grand)

        def _fetch_and_update():
            stock_data = fetch_a_share_quotes(getattr(self, "_stock_items", None))
            try:
                self._post_ui(lambda: self._update_stock_quotes(stock_data))
            except Exception:
                pass

        threading.Thread(target=_fetch_and_update, daemon=True).start()
        self._stock_job = self.root.after(self._stock_refresh_sec * 1000, self._stock_refresh_loop)

    def _update_stock_quotes(self, stock_data):
        if getattr(self, "_closing", False) or self._view_mode != "stock" or not isinstance(stock_data, dict):
            return
        items = stock_data.get("items") or []
        for idx, card in enumerate(getattr(self, "_stock_cards", [])):
            item = items[idx] if idx < len(items) else {}
            card["title"].set(item.get("title") or "--")
            card["price"].set(item.get("price") or "--")
            pct_text = item.get("pct") or "--"
            card["pct"].set(pct_text)
            detail_text = "   ".join(part for part in [item.get("meta1") or "", item.get("meta2") or ""] if part).strip()
            card["detail"].set(detail_text)
            pct_fg = self._c("fg")
            price_fg = self._c("fg")
            if pct_text.startswith("+"):
                pct_fg = self._c("status_ok")
            elif pct_text.startswith("-"):
                pct_fg = self._c("status_err")
            if item.get("price") == "--":
                price_fg = self._c("fg_dim")
            try:
                card["pct_label"].config(fg=pct_fg)
                card["price_label"].config(fg=price_fg)
            except tk.TclError:
                pass

    def _stock_manual_refresh(self):
        if getattr(self, "_closing", False) or self._view_mode != "stock":
            return
        btn = getattr(self, "_stock_refresh_btn", None)
        if btn and btn.winfo_exists():
            btn.config(state=tk.DISABLED)
            self.root.after(3000, self._stock_reenable_refresh_btn)

        def _fetch():
            if getattr(self, "_closing", False):
                return
            stock_data = fetch_a_share_quotes(getattr(self, "_stock_items", None))
            try:
                self._post_ui(lambda: self._update_stock_quotes(stock_data))
            except Exception:
                pass

        threading.Thread(target=_fetch, daemon=True).start()

    def _stock_reenable_refresh_btn(self):
        if getattr(self, "_closing", False) or self._view_mode != "stock":
            return
        btn = getattr(self, "_stock_refresh_btn", None)
        if btn and btn.winfo_exists():
            try:
                btn.config(state=tk.NORMAL)
            except tk.TclError:
                pass

    def _stock_refresh_loop(self):
        if getattr(self, "_closing", False) or self._view_mode != "stock":
            self._stock_job = None
            return

        def _fetch():
            if getattr(self, "_closing", False):
                return
            stock_data = fetch_a_share_quotes(getattr(self, "_stock_items", None))
            self._post_ui(lambda: self._update_stock_quotes(stock_data))

        threading.Thread(target=_fetch, daemon=True).start()
        self._stock_job = self.root.after(self._stock_refresh_sec * 1000, self._stock_refresh_loop)

    def _show_us_stock(self):
        for w in self.us_stock_frame.winfo_children():
            w.destroy()
        self._us_stock_items, self._us_stock_refresh_sec = _load_small_screen_us_stock_config()
        title_row = tk.Frame(self.us_stock_frame, bg=self._c("bg"))
        title_row.pack(fill=tk.X, pady=(0, 6))
        tk.Label(
            title_row, text="US STOCKS", font=("DejaVu Sans", 14, "bold"),
            bg=self._c("bg"), fg=self._c("fg")
        ).pack(side=tk.LEFT)
        self._us_stock_refresh_btn = tk.Button(
            title_row, text=self._t("refresh"), font=("", 10), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"),
            activebackground=self._c("button_active_bg"), activeforeground=self._c("fg"), cursor="hand2",
            command=self._us_stock_manual_refresh, padx=8, pady=2
        )
        self._us_stock_refresh_btn.pack(side=tk.RIGHT, padx=(6, 0))
        tk.Label(
            title_row, text=self._t("us_stock_refresh_hint").format(sec=self._us_stock_refresh_sec), font=("", 10),
            bg=self._c("bg"), fg=self._c("foot_fg")
        ).pack(side=tk.RIGHT)

        items = [
            {"title": symbol, "price": "--", "pct": "--", "meta1": "...", "meta2": ""}
            for symbol in [item.get("name") or item.get("symbol") or "--" for item in self._us_stock_items]
        ]
        self._us_stock_cards = []
        list_wrapper = tk.Frame(self.us_stock_frame, bg=self._c("bg"))
        list_wrapper.pack(fill=tk.BOTH, expand=True)
        canvas = tk.Canvas(list_wrapper, bg=self._c("bg"), highlightthickness=0)
        scrollbar = tk.Scrollbar(list_wrapper)
        inner = tk.Frame(canvas, bg=self._c("bg"))
        win_id = canvas.create_window((0, 0), window=inner, anchor=tk.NW)

        def _on_inner_configure(_):
            canvas.configure(scrollregion=canvas.bbox("all"))

        def _on_canvas_configure(evt):
            canvas.itemconfig(win_id, width=evt.width)

        inner.bind("<Configure>", _on_inner_configure)
        canvas.bind("<Configure>", _on_canvas_configure)
        scrollbar.pack(side=tk.RIGHT, fill=tk.Y)
        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        self._style_scrollbar(scrollbar)

        def _after_scroll():
            canvas.update_idletasks()

        def _scrollbar_cmd(*args):
            canvas.yview(*args)
            _after_scroll()

        canvas.configure(yscrollcommand=scrollbar.set)
        scrollbar.configure(command=_scrollbar_cmd)

        if not items:
            tk.Label(
                inner, text=self._t("us_stock_empty"), font=("", 12),
                bg=self._c("bg"), fg=self._c("status_off")
            ).pack(anchor=tk.W, pady=(12, 0))
            return

        box_bg = self._c("box_bg")
        box_border = self._c("box_border")
        for item in items:
            card = tk.Frame(inner, bg=box_border, padx=2, pady=2)
            card.pack(fill=tk.X, pady=2)
            inner_card = tk.Frame(card, bg=box_bg, padx=8, pady=4)
            inner_card.pack(fill=tk.BOTH, expand=True)
            title_var = tk.StringVar(value=item.get("title") or "--")
            price_var = tk.StringVar(value=item.get("price") or "--")
            pct_var = tk.StringVar(value=item.get("pct") or "--")
            detail_var = tk.StringVar(value=item.get("meta1") or "")
            top_row = tk.Frame(inner_card, bg=box_bg)
            top_row.pack(fill=tk.X)
            tk.Label(top_row, textvariable=title_var, font=("", 10), bg=box_bg, fg=self._c("fg_dim"), anchor=tk.W).pack(side=tk.LEFT, fill=tk.X, expand=True)
            price_label = tk.Label(top_row, textvariable=price_var, font=("", 12, "bold"), bg=box_bg, fg=self._c("fg"))
            price_label.pack(side=tk.RIGHT)
            pct_label = tk.Label(top_row, textvariable=pct_var, font=("", 10, "bold"), bg=box_bg, fg=self._c("fg"))
            pct_label.pack(side=tk.RIGHT, padx=(0, 8))
            tk.Label(inner_card, textvariable=detail_var, font=("", 9), bg=box_bg, fg=self._c("fg"), anchor=tk.W, justify=tk.LEFT).pack(anchor=tk.W, pady=(1, 0))
            self._us_stock_cards.append({
                "title": title_var,
                "price": price_var,
                "pct": pct_var,
                "detail": detail_var,
                "pct_label": pct_label,
                "price_label": price_label,
            })

        _scroll_units = 3

        def _scroll(evt):
            if getattr(evt, "num", None) == 5 or getattr(evt, "delta", 0) == -120:
                canvas.yview_scroll(_scroll_units, "units")
            else:
                canvas.yview_scroll(-_scroll_units, "units")
            _after_scroll()

        _drag_y_root = [None]

        def _on_drag_start(evt):
            _drag_y_root[0] = evt.y_root

        def _on_drag_motion(evt):
            if _drag_y_root[0] is not None:
                dy = evt.y_root - _drag_y_root[0]
                step = max(-12, min(12, int(dy)))
                if step != 0:
                    canvas.yview_scroll(step, "units")
                    _after_scroll()
                _drag_y_root[0] = evt.y_root

        def _on_drag_end(_evt):
            _drag_y_root[0] = None

        def _bind_scroll(widget):
            widget.bind("<MouseWheel>", _scroll)
            widget.bind("<Button-4>", lambda e: (canvas.yview_scroll(-_scroll_units, "units"), _after_scroll()))
            widget.bind("<Button-5>", lambda e: (canvas.yview_scroll(_scroll_units, "units"), _after_scroll()))
            widget.bind("<Button-1>", _on_drag_start)
            widget.bind("<B1-Motion>", _on_drag_motion)
            widget.bind("<ButtonRelease-1>", _on_drag_end)

        _bind_scroll(list_wrapper)
        _bind_scroll(canvas)
        _bind_scroll(inner)
        _bind_scroll(scrollbar)
        for row in inner.winfo_children():
            _bind_scroll(row)
            for child in row.winfo_children():
                _bind_scroll(child)
                for grand in child.winfo_children():
                    _bind_scroll(grand)

        def _fetch_and_update():
            us_stock_data = fetch_us_stock_quotes(getattr(self, "_us_stock_items", None))
            try:
                self._post_ui(lambda: self._update_us_stock_quotes(us_stock_data))
            except Exception:
                pass

        threading.Thread(target=_fetch_and_update, daemon=True).start()
        self._us_stock_job = self.root.after(self._us_stock_refresh_sec * 1000, self._us_stock_refresh_loop)

    def _update_us_stock_quotes(self, us_stock_data):
        if getattr(self, "_closing", False) or self._view_mode != "us_stock" or not isinstance(us_stock_data, dict):
            return
        items = us_stock_data.get("items") or []
        for idx, card in enumerate(getattr(self, "_us_stock_cards", [])):
            item = items[idx] if idx < len(items) else {}
            card["title"].set(item.get("title") or "--")
            card["price"].set(item.get("price") or "--")
            pct_text = item.get("pct") or "--"
            card["pct"].set(pct_text)
            detail_text = "   ".join(part for part in [item.get("meta1") or "", item.get("meta2") or ""] if part).strip()
            card["detail"].set(detail_text)
            pct_fg = self._c("fg")
            price_fg = self._c("fg")
            if pct_text.startswith("+"):
                pct_fg = self._c("status_ok")
            elif pct_text.startswith("-"):
                pct_fg = self._c("status_err")
            if item.get("price") == "--":
                price_fg = self._c("fg_dim")
            try:
                card["pct_label"].config(fg=pct_fg)
                card["price_label"].config(fg=price_fg)
            except tk.TclError:
                pass

    def _us_stock_manual_refresh(self):
        if getattr(self, "_closing", False) or self._view_mode != "us_stock":
            return
        btn = getattr(self, "_us_stock_refresh_btn", None)
        if btn and btn.winfo_exists():
            btn.config(state=tk.DISABLED)
            self.root.after(3000, self._us_stock_reenable_refresh_btn)

        def _fetch():
            if getattr(self, "_closing", False):
                return
            us_stock_data = fetch_us_stock_quotes(getattr(self, "_us_stock_items", None))
            try:
                self._post_ui(lambda: self._update_us_stock_quotes(us_stock_data))
            except Exception:
                pass

        threading.Thread(target=_fetch, daemon=True).start()

    def _us_stock_reenable_refresh_btn(self):
        if getattr(self, "_closing", False) or self._view_mode != "us_stock":
            return
        btn = getattr(self, "_us_stock_refresh_btn", None)
        if btn and btn.winfo_exists():
            try:
                btn.config(state=tk.NORMAL)
            except tk.TclError:
                pass

    def _us_stock_refresh_loop(self):
        if getattr(self, "_closing", False) or self._view_mode != "us_stock":
            self._us_stock_job = None
            return

        def _fetch():
            if getattr(self, "_closing", False):
                return
            us_stock_data = fetch_us_stock_quotes(getattr(self, "_us_stock_items", None))
            self._post_ui(lambda: self._update_us_stock_quotes(us_stock_data))

        threading.Thread(target=_fetch, daemon=True).start()
        self._us_stock_job = self.root.after(self._us_stock_refresh_sec * 1000, self._us_stock_refresh_loop)

    def _refresh_skills_view(self):
        for w in self.skills_frame.winfo_children():
            w.destroy()
        self._skills_loading_label = tk.Label(
            self.skills_frame, text="Loading...", font=("", 12), bg=self._c("bg"), fg=self._c("status_off")
        )
        self._skills_loading_label.pack(pady=12)
        def _fetch():
            result = fetch_skills_config(self._auth_key)
            if getattr(self, "_closing", False):
                return
            self._post_ui(lambda: self._fill_skills_view(result))
        threading.Thread(target=_fetch, daemon=True).start()

    def _refresh_health_once(self):
        def _fetch():
            data, err = fetch_health(self._auth_key)
            logs, user_messages, _summary_err = fetch_clawd_activity(self._auth_key, self._lang)
            if getattr(self, "_closing", False):
                return
            self._post_ui(lambda d=data, e=err, logs=logs, user_messages=user_messages: self._update(d, e, logs=logs, user_messages=user_messages))
        threading.Thread(target=_fetch, daemon=True).start()

    def _fill_skills_view(self, result):
        if getattr(self, "_closing", False) or self._view_mode != "skills":
            return
        all_skills, enabled_set = result if result else (None, None)
        for w in self.skills_frame.winfo_children():
            w.destroy()
        if all_skills is None:
            tk.Label(self.skills_frame, text=self._t("skills_load_fail"), font=("", 12), bg=self._c("bg"), fg=self._c("status_off")).pack(anchor=tk.W)
            return
        # 技能列表全屏上下滑动，保留右侧滚动条；拖拽/滚轮/滚动条均可
        list_wrapper = tk.Frame(self.skills_frame, bg=self._c("bg"))
        list_wrapper.pack(fill=tk.BOTH, expand=True, pady=(0, 4))
        canvas = tk.Canvas(list_wrapper, bg=self._c("bg"), highlightthickness=0)
        scrollbar = tk.Scrollbar(list_wrapper)
        inner = tk.Frame(canvas, bg=self._c("bg"))
        win_id = canvas.create_window((0, 0), window=inner, anchor=tk.NW)

        def _on_inner_configure(_):
            canvas.configure(scrollregion=canvas.bbox("all"))

        def _on_canvas_configure(evt):
            canvas.itemconfig(win_id, width=evt.width)

        inner.bind("<Configure>", _on_inner_configure)
        canvas.bind("<Configure>", _on_canvas_configure)
        row_h = 22
        canvas.configure(yscrollcommand=scrollbar.set, yscrollincrement=row_h)
        scrollbar.pack(side=tk.RIGHT, fill=tk.Y)
        canvas.pack(side=tk.LEFT, fill=tk.BOTH, expand=True)
        self._style_scrollbar(scrollbar)

        def _after_scroll():
            canvas.update_idletasks()

        def _scrollbar_cmd(*args):
            canvas.yview(*args)
            _after_scroll()

        scrollbar.configure(command=_scrollbar_cmd)

        for name in all_skills:
            row = tk.Frame(inner, bg=self._c("bg"), height=row_h)
            row.pack(fill=tk.X, pady=0)
            row.pack_propagate(False)
            dot_canvas = tk.Canvas(row, width=14, height=14, bg=self._c("bg"), highlightthickness=0)
            dot_canvas.pack(side=tk.LEFT, padx=(0, 8), pady=3)
            fill = self._c("status_ok") if name in enabled_set else self._c("status_err")
            dot_canvas.create_oval(2, 2, 12, 12, outline=self._c("status_outline"), fill=fill)
            tk.Label(row, text=name[:36], font=("", 12), bg=self._c("bg"), fg=self._c("fg")).pack(side=tk.LEFT, fill=tk.X, expand=True)
        inner.update_idletasks()
        canvas.configure(scrollregion=canvas.bbox("all"))

        # 滚轮：每次翻一点（约 3 单位）；拖拽：手指方向即列表移动方向，按像素比例滚动
        _scroll_units = 3

        def _scroll(evt):
            # 滚轮向下(delta<0/num=5) -> 视图向下 -> 看下面内容
            if getattr(evt, "num", None) == 5 or getattr(evt, "delta", 0) == -120:
                canvas.yview_scroll(_scroll_units, "units")
            else:
                canvas.yview_scroll(-_scroll_units, "units")
            _after_scroll()
        _drag_y_root = [None]

        def _on_drag_start(evt):
            _drag_y_root[0] = evt.y_root

        def _on_drag_motion(evt):
            if _drag_y_root[0] is not None:
                dy = evt.y_root - _drag_y_root[0]
                # 手指向下(dy>0) -> 视图向下(yview 正) -> 内容上移、看到下面；每次按位移滚动一点
                step = max(-15, min(15, int(dy)))
                if step != 0:
                    canvas.yview_scroll(step, "units")
                    _after_scroll()
                _drag_y_root[0] = evt.y_root

        def _on_drag_end(_evt):
            _drag_y_root[0] = None

        def _bind_scroll(widget):
            widget.bind("<MouseWheel>", _scroll)
            widget.bind("<Button-4>", lambda e: (canvas.yview_scroll(-_scroll_units, "units"), _after_scroll()))
            widget.bind("<Button-5>", lambda e: (canvas.yview_scroll(_scroll_units, "units"), _after_scroll()))
            widget.bind("<Button-1>", _on_drag_start)
            widget.bind("<B1-Motion>", _on_drag_motion)
            widget.bind("<ButtonRelease-1>", _on_drag_end)

        _bind_scroll(list_wrapper)
        _bind_scroll(canvas)
        _bind_scroll(inner)
        _bind_scroll(scrollbar)
        for row in inner.winfo_children():
            _bind_scroll(row)
            for child in row.winfo_children():
                _bind_scroll(child)

    def _schedule_refresh(self):
        existing = getattr(self, "_refresh_thread", None)
        if existing and existing.is_alive():
            return
        def loop():
            health_data = self.health
            health_err = self.error
            next_health_at = 0.0
            while not getattr(self, "_closing", False):
                now_ts = time.time()
                if now_ts >= next_health_at:
                    health_data, health_err = fetch_health(self._auth_key)
                    next_health_at = now_ts + HEALTH_REFRESH_SEC
                profile = self._activity_fetch_profile()
                activity_started_at = time.perf_counter()
                logs, user_messages, summary_err = fetch_clawd_activity(
                    self._auth_key,
                    self._lang,
                    lines=profile["lines"],
                    log_limit=profile["log_limit"],
                    message_limit=profile["message_limit"],
                )
                activity_elapsed_ms = int((time.perf_counter() - activity_started_at) * 1000)
                if summary_err:
                    logger.warning("Activity refresh failed: %s", summary_err)
                else:
                    logger.debug(
                        "Activity refresh completed in %sms (lines=%s, logs=%s, messages=%s)",
                        activity_elapsed_ms,
                        profile["lines"],
                        len(logs or []),
                        len(user_messages or []),
                    )
                if getattr(self, "_closing", False):
                    break
                self._post_ui(
                    lambda d=health_data, e=health_err, logs=logs, user_messages=user_messages: self._update(
                        d, e, logs=logs, user_messages=user_messages
                    )
                )
                time.sleep(LOGS_REFRESH_SEC)
            self._refresh_thread = None
        t = threading.Thread(target=loop, daemon=True)
        self._refresh_thread = t
        t.start()

    def _blink_step(self):
        if getattr(self, "_closing", False) or not self._online:
            return
        try:
            current = self.status_canvas.itemcget(self.status_oval, "fill")
            next_fill = self._c("bg") if current == self._c("status_ok") else self._c("status_ok")
            self.status_canvas.itemconfig(self.status_oval, fill=next_fill)
        except tk.TclError:
            return
        self._blink_job = self.root.after(500, self._blink_step)

    def _update(self, data, err, summary=None, logs=None, user_messages=None):
        if getattr(self, "_closing", False):
            return
        if logs is not None:
            self._apply_log_entries(logs)
            self.log_summary = logs[:8]
        elif summary is not None:
            self.log_summary = summary
        if user_messages is not None:
            self.user_messages = user_messages
            self._refresh_topbar()
        if err:
            self.health = None
            self._online = False
            self._pending_log_entries = []
            self._cancel_log_append_job()
            if self._blink_job:
                self.root.after_cancel(self._blink_job)
                self._blink_job = None
            try:
                self.status_canvas.itemconfig(self.status_oval, fill=self._c("status_err"))
            except tk.TclError:
                pass
            self.ver_var.set("--")
            self.uptime_var.set("--")
            self.queue_var.set("--")
            self.running_var.set("--")
            self.worker_var.set(self._t("worker_offline"))
            self.rss_var.set("--")
            self.adapters_var.set(self._t("worker_offline"))
            self.adapters_rss_var.set("--")
            self.users_count_var.set("--")
            self.bound_channels_var.set("--")
            self.user_messages = []
            self._refresh_topbar()
            if self._view_mode == "users":
                self._render_user_messages()
            if self._view_mode == "logs":
                self._render_logs_view()
            self.foot_var.set(err[:60])
            self._refresh_dashboard_overview_if_needed()
            return
        self.health = data
        self._online = True
        try:
            self.status_canvas.itemconfig(self.status_oval, fill=self._c("status_ok"))
        except tk.TclError:
            pass
        if not self._blink_job:
            self._blink_job = self.root.after(500, self._blink_step)
        self.ver_var.set((data.get("version") or "--")[:20])
        self.uptime_var.set(fmt_duration(data.get("uptime_seconds")))
        self.queue_var.set(str(data.get("queue_length") if data.get("queue_length") is not None else "--"))
        self.running_var.set(str(data.get("running_length") if data.get("running_length") is not None else "--"))
        self.worker_var.set((data.get("worker_state") or "--")[:16])
        self.rss_var.set(fmt_bytes(data.get("memory_rss_bytes")))
        # 通信端：TG 后显示 TG 占用内存，WA / WA-Web / WEBD / FS(Feishu) / Lark / WX(wechatd)
        parts = []
        if data.get("webd_healthy"):
            webd_rss = data.get("webd_memory_rss_bytes")
            parts.append("WEBD " + fmt_bytes(webd_rss) if webd_rss is not None else "WEBD")
        if data.get("telegramd_healthy") or data.get("telegram_bot_healthy"):
            tg_rss = data.get("telegramd_memory_rss_bytes") or data.get("telegram_bot_memory_rss_bytes")
            parts.append("TG " + fmt_bytes(tg_rss) if tg_rss is not None else "TG")
        if data.get("whatsappd_healthy") or data.get("whatsapp_cloud_healthy"):
            parts.append("WA")
        if data.get("whatsapp_web_healthy"):
            parts.append("WA-Web")
        if data.get("feishud_healthy"):
            fs_rss = data.get("feishud_memory_rss_bytes")
            parts.append("FS " + fmt_bytes(fs_rss) if fs_rss is not None else "FS")
        if data.get("larkd_healthy"):
            lk_rss = data.get("larkd_memory_rss_bytes")
            parts.append("Lark " + fmt_bytes(lk_rss) if lk_rss is not None else "Lark")
        if data.get("wechatd_healthy"):
            wx_rss = data.get("wechatd_memory_rss_bytes")
            parts.append("WX " + fmt_bytes(wx_rss) if wx_rss is not None else "WX")
        self.adapters_var.set(", ".join(parts) if parts else "--")
        # 通信端占的内存（TG + WA + WA-Web + WEBD + Feishu + Lark + wechatd 进程 RSS 之和）
        def _n(v):
            return v if isinstance(v, (int, float)) and v is not None else 0
        total = (
            _n(data.get("telegramd_memory_rss_bytes"))
            + _n(data.get("whatsappd_memory_rss_bytes"))
            + _n(data.get("whatsapp_web_memory_rss_bytes"))
            + _n(data.get("webd_memory_rss_bytes"))
            + _n(data.get("feishud_memory_rss_bytes"))
            + _n(data.get("larkd_memory_rss_bytes"))
            + _n(data.get("wechatd_memory_rss_bytes"))
        )
        self.adapters_rss_var.set(fmt_bytes(int(total)) if total else "--")
        self._update_user_summary_view()
        self._refresh_topbar()
        self.foot_var.set(self._t("update_fmt").format(time=datetime.now().strftime("%H:%M:%S"), sec=LOGS_REFRESH_SEC))
        self._refresh_dashboard_overview_if_needed()

    def _on_close(self):
        self._closing = True
        self._close_wifi_keyboard()
        self._prepare_for_ui_rebuild()
        self._cancel_job("_ui_pump_job")
        lock_fd = getattr(self, "_lock_fd", None)
        if lock_fd is not None:
            try:
                os.close(lock_fd)
            except OSError:
                pass
            self._lock_fd = None
        try:
            self.root.quit()
        except tk.TclError:
            pass
        try:
            self.root.destroy()
        except tk.TclError:
            pass

    def _raise_window(self):
        """自启动时把窗口提到最前并获取焦点，避免被其它窗口挡住。"""
        self._raise_window_job = None
        try:
            self.root.lift()
            self.root.attributes("-topmost", True)
            self._clear_topmost_job = self.root.after(400, self._clear_topmost)
            self.root.focus_force()
        except tk.TclError:
            pass

    def _clear_topmost(self):
        self._clear_topmost_job = None
        try:
            if not getattr(self, "_closing", False):
                self.root.attributes("-topmost", False)
        except tk.TclError:
            pass

    def _start_fullscreen(self):
        self.root.attributes("-fullscreen", True)
        try:
            self.root.config(cursor="none")
        except tk.TclError:
            pass
        self.root.bind("<F11>", lambda e: self._toggle_fullscreen())
        self.root.bind("<Escape>", lambda e: self._on_close())
        # bind_all 使设置页等子控件上滑动也能触发翻页
        self.root.bind_all("<Left>", lambda e: self._on_swipe_next())
        self.root.bind_all("<Right>", lambda e: self._on_swipe_prev())
        self.root.bind_all("<ButtonPress-1>", self._on_swipe_start)
        self.root.bind_all("<ButtonRelease-1>", self._on_swipe_end)
        # 自启动时窗口容易被挡，启动后置前一次
        self._cancel_job("_raise_window_job")
        self._raise_window_job = self.root.after(300, self._raise_window)

    def _on_swipe_start(self, event):
        self._swipe_start_x = getattr(self, "_swipe_start_x", 0)
        self._swipe_start_y = getattr(self, "_swipe_start_y", 0)
        self._swipe_start_x = event.x
        self._swipe_start_y = event.y

    def _is_nested_menu_active(self):
        if getattr(self, "_view_mode", None) == "wifi":
            return True
        if getattr(self, "_view_mode", None) == "settings" and getattr(self, "_settings_category", "menu") != "menu":
            return True
        return False

    def _on_swipe_end(self, event):
        if getattr(self, "_closing", False):
            return
        dx = event.x - getattr(self, "_swipe_start_x", event.x)
        dy = event.y - getattr(self, "_swipe_start_y", event.y)
        if abs(dx) < 50 or abs(dx) <= abs(dy):
            return
        if dx < 0:
            self._on_swipe_next()
        else:
            self._on_swipe_prev()

    def _on_swipe_next(self, _event=None):
        if getattr(self, "_closing", False):
            return
        if getattr(self, "_view_mode", None) is None:
            return
        if self._is_nested_menu_active():
            return
        self._toggle_view()

    def _on_swipe_prev(self, _event=None):
        if getattr(self, "_closing", False):
            return
        if getattr(self, "_view_mode", None) is None:
            return
        if self._is_nested_menu_active():
            return
        self._go_prev_view()

    def _toggle_fullscreen(self):
        self.root.attributes("-fullscreen", not self.root.attributes("-fullscreen"))

    def _tick_time(self):
        if getattr(self, "_closing", False):
            self._time_job = None
            return
        self.time_var.set(datetime.now().strftime("%H:%M:%S"))
        self._time_job = self.root.after(1000, self._tick_time)

    def _animate_gif(self):
        if getattr(self, "_closing", False) or not self.gif_frames or not self.gif_delays:
            self._gif_job = None
            return
        self.lobster_label.configure(image=self.gif_frames[self.gif_frame_idx])
        delay = self.gif_delays[self.gif_frame_idx]
        self.gif_frame_idx = (self.gif_frame_idx + 1) % len(self.gif_frames)
        self._gif_job = self.root.after(delay, self._animate_gif)

    def run(self):
        self.root.mainloop()


if __name__ == "__main__":
    app = SmallScreenApp()
    app.run()
