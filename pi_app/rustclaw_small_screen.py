#!/usr/bin/env python3
# RustClaw 小屏监控：480×320 全屏，健康状态慢刷新、日志温和刷新，左侧龙虾动图 + RustClaw 标题。
# 需先启动 clawd（8787）。按 F11 或 Escape 退出全屏/关闭。

import errno
import json
import os
import random
import re
import secrets
import sqlite3
import subprocess
import sys
import tkinter as tk
import tkinter.font as tkfont
import urllib.parse
import urllib.request
import threading
import time
from datetime import datetime

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None

API_BASE = "http://127.0.0.1:8787"
HEALTH_REFRESH_SEC = 15
LOGS_REFRESH_SEC = 5
W, H = 480, 320
ASSETS_DIR = None

# Matrix 主题下竖排随机字符（数字、拉丁、片假名等）
MATRIX_CHARS = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyzｱｲｳｴｵｶｷｸｹｺｻｼｽｾｿﾀﾁﾂﾃﾄﾅﾆﾇﾈﾉﾊﾋﾎﾏﾐﾑﾒﾓﾔﾕﾖﾗﾘﾙﾚﾛﾜﾝｦｧｨｩｪｫｬｭｮｯｰ"

# 多语言文案（CN / EN）
STRINGS = {
    "CN": {
        "app_title": "RustClaw 小屏",
        "switch": "切换",
        "settings": "设置",
        "version": "版本",
        "uptime": "运行时长",
        "queue": "队列",
        "running": "执行中",
        "worker": "Worker",
        "worker_offline": "未运行",
        "memory_rss": "内存 RSS",
        "adapters": "通信端",
        "adapters_memory": "通信端内存",
        "adapters_memory_space": "  通信端内存 ",
        "foot_prefix": "同源请求 /v1/health",
        "update_fmt": "更新: {time} (每{sec}s)",
        "skills_title": "SKILLS",
        "skills_load_fail": "无法加载技能配置",
        "users_title": "用户",
        "users_count": "启用用户",
        "bound_channels": "已绑定通信端",
        "clawd_summary": "Logs",
        "clawd_summary_empty": "暂无摘要",
        "logs_title": "Logs",
        "recent_messages_title": "最近消息",
        "recent_messages_empty": "暂无用户消息",
        "recent_message_more_hint": "在通信端查看",
        "msg_replied_label": "已回复",
        "msg_replied_hint": "已回复，在通信端查看",
        "logs_empty": "暂无日志",
        "settings_title": "设置",
        "language": "语言",
        "lang_en": "EN",
        "lang_cn": "CN",
        "ok": "确定",
        "cancel": "取消",
        "crypto_refresh_hint": "每{sec}秒自动刷新",
        "crypto_empty": "请在 small_screen_markets.toml 配置展示币种",
        "stock_refresh_hint": "每{sec}秒自动刷新",
        "stock_empty": "请在 small_screen_markets.toml 配置展示股票",
        "refresh": "刷新",
        "llm_title": "NNI分布式模型 (test)",
        "llm_join": "加入",
        "llm_stop": "停止",
        "theme": "界面",
        "theme_default": "默认",
        "theme_matrix": "Matrix",
        "restart": "重启RustClaw核心",
        "restarting": "重启中.....",
    },
    "EN": {
        "app_title": "RustClaw Small Screen",
        "switch": "Switch",
        "settings": "Settings",
        "version": "Version",
        "uptime": "Uptime",
        "queue": "Queue",
        "running": "Running",
        "worker": "Worker",
        "worker_offline": "Not running",
        "memory_rss": "Memory RSS",
        "adapters": "Adapters",
        "adapters_memory": "Adapters memory",
        "adapters_memory_space": "  Adapters memory ",
        "foot_prefix": "Same-origin /v1/health",
        "update_fmt": "Update: {time} (every {sec}s)",
        "skills_title": "SKILLS",
        "skills_load_fail": "Failed to load skills config",
        "users_title": "Users",
        "users_count": "Enabled users",
        "bound_channels": "Bound channels",
        "clawd_summary": "Logs",
        "clawd_summary_empty": "No summary",
        "logs_title": "Logs",
        "recent_messages_title": "Recent Messages",
        "recent_messages_empty": "No user messages",
        "recent_message_more_hint": "See in Adapters",
        "msg_replied_label": "Replied",
        "msg_replied_hint": "Replied, see in Adapters",
        "logs_empty": "No logs",
        "settings_title": "Settings",
        "language": "Language",
        "lang_en": "EN",
        "lang_cn": "CN",
        "ok": "OK",
        "cancel": "Cancel",
        "crypto_refresh_hint": "Auto refresh every {sec}s",
        "crypto_empty": "Configure crypto items in small_screen_markets.toml",
        "stock_refresh_hint": "Auto refresh every {sec}s",
        "stock_empty": "Configure stock items in small_screen_markets.toml",
        "refresh": "Refresh",
        "llm_title": "Network Native Intelligence (test)",
        "llm_join": "Join",
        "llm_stop": "Stop",
        "theme": "Theme",
        "theme_default": "Default",
        "theme_matrix": "Matrix",
        "restart": "Restart RustClaw Core",
        "restarting": "Restarting.....",
    },
}

# 界面主题：default 深蓝 | matrix 黑客帝国绿
THEMES = {
    "default": {
        "bg": "#1a1a2e",
        "fg": "#e8e6e3",
        "fg_dim": "#8a8580",
        "accent": "#ff6b4a",
        "button_bg": "#2a2a3a",
        "button_fg": "#e8e6e3",
        "button_active_bg": "#3a3a4a",
        "box_bg": "#12121a",
        "box_border": "#2a2a3a",
        "adapters_fg": "#5bc0be",
        "adapters_value_fg": "#98e6e4",
        "foot_fg": "#666",
        "status_outline": "#444",
        "status_off": "#888",
        "status_ok": "#5cdb5c",
        "status_err": "#ff6b6b",
        "summary_llm": "#ffd166",
        "summary_task": "#5bc0eb",
        "summary_error": "#ff6b6b",
        "summary_routing": "#c77dff",
        "summary_tool": "#7bd389",
        "summary_skill": "#39d2c0",
        "summary_other": "#bfc7d5",
        "msg_user_fg": "#e8e6e3",
        "msg_agent_fg": "#ffd166",
        "selectcolor": "#2a2a3a",
        "bg_rgb": (0x1a, 0x1a, 0x2e),
    },
    "matrix": {
        "bg": "#000000",
        "fg": "#00ff41",
        "fg_dim": "#008f11",
        "accent": "#00ff41",
        "button_bg": "#0a1a0a",
        "button_fg": "#00ff41",
        "button_active_bg": "#0d2a0d",
        "box_bg": "#001100",
        "box_border": "#003300",
        "adapters_fg": "#00ff41",
        "adapters_value_fg": "#39ff14",
        "foot_fg": "#004400",
        "status_outline": "#003300",
        "status_off": "#005500",
        "status_ok": "#00ff41",
        "status_err": "#ff0040",
        "summary_llm": "#ffe600",
        "summary_task": "#00d5ff",
        "summary_error": "#ff0040",
        "summary_routing": "#ff66ff",
        "summary_tool": "#00ff9c",
        "summary_skill": "#39ff14",
        "summary_other": "#7dff7d",
        "msg_user_fg": "#7dff7d",
        "msg_agent_fg": "#00d5ff",
        "selectcolor": "#0a2a0a",
        "bg_rgb": (0, 0, 0),
    },
}


def find_assets():
    import os
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(script_dir, "assets")


def find_splash_image():
    """启动图：脚本目录下 RustClaw480X320.png，若存在则用于全屏启动界面。"""
    script_dir = os.path.dirname(os.path.abspath(__file__))
    path = os.path.join(script_dir, "RustClaw480X320.png")
    return path if os.path.isfile(path) else None


def find_image_dir():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(script_dir, "image")


def list_gallery_images():
    """返回 scripts/image 下图片路径列表，按文件名排序。"""
    ext = (".png", ".jpg", ".jpeg", ".gif", ".bmp", ".webp")
    path = find_image_dir()
    if not os.path.isdir(path):
        return []
    out = []
    for name in sorted(os.listdir(path)):
        if name.lower().endswith(ext):
            out.append(os.path.join(path, name))
    return out


def _lang_file():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(script_dir, ".rustclaw_small_screen_lang")


def _theme_file():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(script_dir, ".rustclaw_small_screen_theme")


def _key_file():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(script_dir, ".rustclaw_small_screen_key")


def _root_dir():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.dirname(script_dir)


def _config_path():
    return os.path.join(_root_dir(), "configs", "config.toml")


def load_theme():
    try:
        with open(_theme_file(), "r", encoding="utf-8") as f:
            t = f.read().strip().lower()
            if t in ("default", "matrix"):
                return t
    except Exception:
        pass
    return "default"


def save_theme(theme):
    try:
        with open(_theme_file(), "w", encoding="utf-8") as f:
            f.write(theme)
    except Exception:
        pass


def load_auth_key():
    try:
        with open(_key_file(), "r", encoding="utf-8") as f:
            return f.read().strip()
    except Exception:
        return ""


def save_auth_key(user_key):
    try:
        with open(_key_file(), "w", encoding="utf-8") as f:
            f.write((user_key or "").strip())
    except Exception:
        pass


def _load_sqlite_path_from_config():
    if tomllib is None:
        return os.path.join(_root_dir(), "data", "rustclaw.db")
    try:
        with open(_config_path(), "rb") as f:
            cfg = tomllib.load(f)
        db_rel = (((cfg or {}).get("database") or {}).get("sqlite_path")) or "data/rustclaw.db"
        return os.path.join(_root_dir(), db_rel)
    except Exception:
        return os.path.join(_root_dir(), "data", "rustclaw.db")


def _generate_user_key():
    return "rk-" + secrets.token_urlsafe(18)


def ensure_small_screen_auth_key():
    user_key = load_auth_key().strip()
    db_path = _load_sqlite_path_from_config()
    try:
        os.makedirs(os.path.dirname(db_path), exist_ok=True)
        conn = sqlite3.connect(db_path)
        conn.execute(
            """
            CREATE TABLE IF NOT EXISTS auth_keys (
                user_key     TEXT PRIMARY KEY,
                role         TEXT NOT NULL CHECK (role IN ('admin', 'user')),
                enabled      INTEGER NOT NULL DEFAULT 1,
                created_at   TEXT NOT NULL,
                last_used_at TEXT
            )
            """
        )
        if not user_key:
            user_key = _generate_user_key()
            save_auth_key(user_key)
        conn.execute(
            """
            INSERT INTO auth_keys (user_key, role, enabled, created_at, last_used_at)
            VALUES (?, 'user', 1, strftime('%s','now'), NULL)
            ON CONFLICT(user_key) DO UPDATE SET enabled=1
            """,
            (user_key,),
        )
        conn.commit()
        conn.close()
        return user_key
    except Exception:
        return user_key


def _default_lang_from_system():
    """根据系统语言返回默认语言，兜底为英语 EN。"""
    try:
        import locale
        loc, _ = locale.getdefaultlocale()
        if loc:
            if loc.lower().startswith("zh"):
                return "CN"
    except Exception:
        pass
    for key in ("LANG", "LC_ALL", "LANGUAGE"):
        val = os.environ.get(key, "")
        if isinstance(val, str) and val.lower().startswith("zh"):
            return "CN"
    return "EN"


def load_lang():
    try:
        with open(_lang_file(), "r", encoding="utf-8") as f:
            lang = f.read().strip().upper()
            if lang in ("EN", "CN"):
                return lang
    except Exception:
        pass
    return _default_lang_from_system()


def save_lang(lang):
    try:
        with open(_lang_file(), "w", encoding="utf-8") as f:
            f.write(lang)
    except Exception:
        pass


def _build_api_request(path, user_key=""):
    req = urllib.request.Request(API_BASE.rstrip("/") + path)
    user_key = (user_key or "").strip()
    if user_key:
        req.add_header("X-RustClaw-Key", user_key)
    return req


def fetch_health(user_key=""):
    try:
        req = _build_api_request("/v1/health", user_key)
        with urllib.request.urlopen(req, timeout=5) as r:
            body = json.loads(r.read().decode())
        data = body.get("data") or body
        return data, None
    except Exception as e:
        return None, str(e)


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


def _line_clamp_text(text, font, wraplength, max_lines=3, ellipsis="..."):
    content = str(text or "")
    if max_lines <= 0:
        return content
    measure_font = tkfont.Font(font=font)
    wrapped_lines = []
    for raw_line in content.split("\n"):
        if raw_line == "":
            wrapped_lines.append("")
            continue
        current = ""
        for ch in raw_line:
            candidate = current + ch
            if current and measure_font.measure(candidate) > wraplength:
                wrapped_lines.append(current)
                current = ch
            else:
                current = candidate
        wrapped_lines.append(current)
    if len(wrapped_lines) > max_lines:
        wrapped_lines = wrapped_lines[:max_lines]
        tail = wrapped_lines[-1].rstrip()
        while tail and measure_font.measure(tail + ellipsis) > wraplength:
            tail = tail[:-1]
        wrapped_lines[-1] = (tail + ellipsis) if tail else ellipsis
    if len(wrapped_lines) < max_lines:
        wrapped_lines.extend([""] * (max_lines - len(wrapped_lines)))
    return "\n".join(wrapped_lines)


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
    routed = re.search(r"route_request_mode llm .* mode=(ChatAct|AskClarify|Chat|Act)", line)
    if routed:
        return f"route {routed.group(1)} {status}"
    mode = re.search(r"routed_mode=(ChatAct|AskClarify|Chat|Act)\b", line)
    if mode:
        return f"route {mode.group(1)} {status}"
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
        elif any(key in lower for key in ("routed_mode", "resolve_user_request", "context_resolver", "[routing]")):
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
        req = _build_api_request("/v1/logs/latest?" + query, user_key)
        with urllib.request.urlopen(req, timeout=5) as r:
            body = json.loads(r.read().decode())
        data = body.get("data") or body or {}
        return _collect_clawd_log_items(data.get("text") or "", lang=lang, limit=limit), None
    except Exception as e:
        return None, str(e)


def fetch_clawd_activity(user_key="", lang="CN", lines=300, log_limit=24, message_limit=5):
    try:
        query = urllib.parse.urlencode({"file": "clawd.log", "lines": lines})
        req = _build_api_request("/v1/logs/latest?" + query, user_key)
        with urllib.request.urlopen(req, timeout=5) as r:
            body = json.loads(r.read().decode())
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


BINANCE_TICKER_URL = "https://api.binance.com/api/v3/ticker/price"
SINA_HQ_URL = "http://hq.sinajs.cn/list="
SINA_REFERER = "https://finance.sina.com.cn"
DEFAULT_A_SHARE_REFRESH_SEC = 15
DEFAULT_CRYPTO_REFRESH_SEC = 15
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


def _strip_trailing_zeros(price_str):
    """去掉价格字符串小数点后尾部的 0，若小数部分全为 0 则去掉小数点。"""
    s = str(price_str).strip()
    if "." not in s:
        return s
    int_part, _, frac = s.partition(".")
    frac = frac.rstrip("0")
    return int_part if not frac else f"{int_part}.{frac}"


def _small_screen_market_config_path():
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.join(script_dir, "small_screen_markets.toml")


def _load_small_screen_market_config():
    if tomllib is None:
        return {}
    try:
        with open(_small_screen_market_config_path(), "rb") as f:
            cfg = tomllib.load(f)
        return cfg if isinstance(cfg, dict) else {}
    except Exception:
        return {}


def _parse_refresh_seconds(value, default_value):
    if isinstance(value, (int, float)):
        return max(5, min(int(value), 3600))
    return default_value


def _load_small_screen_crypto_config():
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


def fetch_crypto_prices(crypto_items=None):
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


def _normalize_stock_code(input_text):
    s = str(input_text or "").strip().lower()
    digits = "".join(ch for ch in s if ch.isdigit())
    if s.startswith(("sh", "sz")) and len(digits) == 6:
        return s[:2] + digits
    if len(digits) == 6:
        return ("sh" if digits.startswith("6") else "sz") + digits
    return ""


def _load_small_screen_stock_config():
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


def _decode_sina_body(raw):
    try:
        text = raw.decode("utf-8")
        if "var hq_str_" in text:
            return text
    except UnicodeDecodeError:
        pass
    return raw.decode("gbk", errors="ignore")


def _safe_float(value):
    try:
        return float(str(value).strip())
    except Exception:
        return None


def _fmt_signed_pct(current, prev_close):
    current_num = _safe_float(current)
    prev_num = _safe_float(prev_close)
    if current_num is None or prev_num is None or prev_num <= 0:
        return "--"
    pct = (current_num - prev_num) / prev_num * 100.0
    sign = "+" if pct >= 0 else ""
    return f"{sign}{pct:.2f}%"


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


def fetch_a_share_quotes(stock_items=None):
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


def fetch_skills_config(user_key=""):
    """GET /v1/skills/config，返回 (all_skills, enabled_set) 或 (None, None) 表示失败。"""
    try:
        req = _build_api_request("/v1/skills/config", user_key)
        with urllib.request.urlopen(req, timeout=5) as r:
            body = json.loads(r.read().decode())
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


def fmt_duration(sec):
    if sec is None or sec < 0:
        return "--"
    h = int(sec // 3600)
    m = int((sec % 3600) // 60)
    s = int(sec % 60)
    if h > 0:
        return f"{h}h{m}m"
    if m > 0:
        return f"{m}m{s}s"
    return f"{s}s"


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


def fmt_bytes(n):
    if n is None or n < 0:
        return "--"
    if n < 1024:
        return f"{n} B"
    if n < 1024 * 1024:
        return f"{n/1024:.1f} KB"
    return f"{n/(1024*1024):.1f} MB"


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
        self._auth_key = ensure_small_screen_auth_key()
        self._i18n = []  # [(widget, key), ...] 用于切换语言时更新
        self.root.title(STRINGS.get(self._lang, STRINGS["CN"])["app_title"])
        self.root.geometry(f"{W}x{H}")
        self.root.resizable(False, False)
        self.root.configure(bg=self._c("bg"))
        self.health = None
        self.log_summary = []
        self.log_entries = []
        self.user_messages = []
        self._last_user_messages_signature = None
        self._log_entry_limit = 24
        self._pending_log_entries = []
        self._log_append_job = None
        self.error = None
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
            self.root.after(2000, self._after_splash)
        else:
            self._build_ui()
            self._schedule_refresh()
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
            img = Image.open(image_path).convert("RGB")
            # 全屏：缩放到窗口大小 W×H 填满
            img = img.resize((W, H), Image.Resampling.LANCZOS)
            self._splash_photo = ImageTk.PhotoImage(img)
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

    def _after_splash(self):
        """等待界面结束后构建主界面。"""
        if getattr(self, "_closing", False):
            return
        if hasattr(self, "_splash_frame") and self._splash_frame.winfo_exists():
            self._splash_frame.destroy()
        self._build_ui()
        self._schedule_refresh()
        self._tick_time()
        if self.gif_frames:
            self._animate_gif()
        self.root.after(200, self._raise_window)

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
                    img = Image.open(gif_path)
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
        top_text = tk.Frame(top, bg=self._c("bg"))
        top_text.pack(side=tk.LEFT, fill=tk.X, expand=True, padx=(0, 8))
        tk.Label(
            top_text, text="RustClaw", font=("", 20, "bold"),
            bg=self._c("bg"), fg=self._c("accent"), anchor="w"
        ).pack(anchor=tk.W)
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
        # 右侧：当前时间（左） + 状态在线/离线（右）
        self.time_var = tk.StringVar(value="--:--:--")
        right_frame = tk.Frame(top, bg=self._c("bg"))
        right_frame.pack(side=tk.RIGHT)
        tk.Label(
            right_frame, textvariable=self.time_var, font=("", 14),
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
        self.skills_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.gallery_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.crypto_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.stock_frame = tk.Frame(self.switch_container, bg=self._c("bg"))
        self.users_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=20, pady=18)
        self.logs_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=10, pady=8)
        self.settings_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=24, pady=20)
        # 顺序（左滑下一页）：首页 → 用户 → 日志 → 技能 → A股 → 加密货币 → 挖矿 → 设置 → 首页；右滑=上一页
        self._view_mode = "dashboard"  # dashboard | users | logs | skills | stock | crypto | gallery | settings
        self._crypto_job = None
        self._stock_job = None
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
        tk.Label(adapters_row, textvariable=self.adapters_var, font=adapters_font, bg=self._c("bg"), fg=self._c("adapters_value_fg")).pack(side=tk.LEFT)
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
        self._users_body = tk.Frame(self.users_frame, bg=self._c("bg"))
        self._users_body.pack(fill=tk.BOTH, expand=True)
        self._users_messages_body = tk.Frame(self._users_body, bg=self._c("bg"))
        self._users_messages_body.pack(fill=tk.BOTH, expand=True)
        self._logs_body = tk.Frame(self.logs_frame, bg=self._c("bg"))
        self._logs_body.pack(fill=tk.BOTH, expand=True)
        # 翻页：左右滑屏可到仪表盘 / 技能 / 加密货币 / 图库 / 用户 / 设置
        # 设置页（内嵌在主窗口，左滑可进入）
        self._settings_title_label = tk.Label(self.settings_frame, text=_t("settings_title"), font=("", 16, "bold"), bg=self._c("bg"), fg=self._c("fg"))
        self._settings_title_label.pack(anchor=tk.W, pady=(0, 12))
        self._settings_lang_label = tk.Label(self.settings_frame, text=_t("language") + ":", font=("", 12), bg=self._c("bg"), fg=self._c("fg"))
        self._settings_lang_label.pack(anchor=tk.W)
        self._settings_lang_var = tk.StringVar(value=self._lang)
        rf = tk.Frame(self.settings_frame, bg=self._c("bg"))
        rf.pack(fill=tk.X, pady=6)
        tk.Radiobutton(rf, text="EN", variable=self._settings_lang_var, value="EN", font=("", 11), bg=self._c("bg"), fg=self._c("fg"), selectcolor=self._c("selectcolor"), activebackground=self._c("bg"), activeforeground=self._c("fg")).pack(side=tk.LEFT, padx=(0, 16))
        tk.Radiobutton(rf, text="CN", variable=self._settings_lang_var, value="CN", font=("", 11), bg=self._c("bg"), fg=self._c("fg"), selectcolor=self._c("selectcolor"), activebackground=self._c("bg"), activeforeground=self._c("fg")).pack(side=tk.LEFT)
        self._settings_theme_label = tk.Label(self.settings_frame, text=_t("theme") + ":", font=("", 12), bg=self._c("bg"), fg=self._c("fg"))
        self._settings_theme_label.pack(anchor=tk.W, pady=(12, 4))
        self._settings_theme_var = tk.StringVar(value=self._theme)
        rf2 = tk.Frame(self.settings_frame, bg=self._c("bg"))
        rf2.pack(fill=tk.X, pady=2)
        tk.Radiobutton(rf2, text=_t("theme_default"), variable=self._settings_theme_var, value="default", font=("", 11), bg=self._c("bg"), fg=self._c("fg"), selectcolor=self._c("selectcolor"), activebackground=self._c("bg"), activeforeground=self._c("fg")).pack(side=tk.LEFT, padx=(0, 16))
        tk.Radiobutton(rf2, text=_t("theme_matrix"), variable=self._settings_theme_var, value="matrix", font=("", 11), bg=self._c("bg"), fg=self._c("fg"), selectcolor=self._c("selectcolor"), activebackground=self._c("bg"), activeforeground=self._c("fg")).pack(side=tk.LEFT)
        bf = tk.Frame(self.settings_frame, bg=self._c("bg"))
        bf.pack(fill=tk.X, pady=(12, 0))
        self._settings_ok_btn = tk.Button(bf, text=_t("ok"), font=("", 11), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"), command=self._on_settings_ok)
        self._settings_ok_btn.pack(side=tk.LEFT, padx=(0, 8))
        self._settings_cancel_btn = tk.Button(bf, text=_t("cancel"), font=("", 11), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"), command=self._on_settings_cancel)
        self._settings_cancel_btn.pack(side=tk.LEFT, padx=(0, 8))
        self._settings_restart_btn = tk.Button(bf, text=_t("restart"), font=("", 11), relief=tk.FLAT, bg=self._c("button_bg"), fg=self._c("button_fg"), command=self._on_settings_restart)
        self._settings_restart_btn.pack(side=tk.LEFT)
        self._refresh_topbar()

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

    def _refresh_topbar(self):
        if self._view_mode == "users":
            self._top_recent_message_var.set(self._t("recent_messages_title"))
        elif self._view_mode == "logs":
            self._top_recent_message_var.set("logs")
        elif self._view_mode == "skills":
            self._top_recent_message_var.set("skills")
        else:
            if self._top_recent_message_label.winfo_manager():
                self._top_recent_message_label.pack_forget()
            return
        self._top_recent_message_label.config(bg=self._c("bg"), fg=self._c("fg_dim"))
        if self._top_recent_message_label.winfo_manager() != "pack":
            self._top_recent_message_label.pack(fill=tk.X, anchor=tk.W)

    def _prepare_settings_view(self):
        """进入设置页时刷新标题和按钮文案。"""
        self._settings_title_label.config(text=self._t("settings_title"), bg=self._c("bg"), fg=self._c("fg"))
        self._settings_lang_label.config(text=self._t("language") + ":", bg=self._c("bg"), fg=self._c("fg"))
        self._settings_theme_label.config(text=self._t("theme") + ":", bg=self._c("bg"), fg=self._c("fg"))
        self._settings_ok_btn.config(text=self._t("ok"), bg=self._c("button_bg"), fg=self._c("button_fg"))
        self._settings_cancel_btn.config(text=self._t("cancel"), bg=self._c("button_bg"), fg=self._c("button_fg"))
        self._settings_restart_btn.config(bg=self._c("button_bg"), fg=self._c("button_fg"))
        try:
            self._settings_restart_btn.config(text=self._t("restart") if self._settings_restart_btn["state"] != tk.DISABLED else self._t("restarting"))
        except tk.TclError:
            pass
        self._settings_lang_var.set(self._lang)
        self._settings_theme_var.set(self._theme)

    def _prepare_logs_view(self):
        self._render_logs_view()

    def _prepare_users_view(self):
        self._dashboard_summary_row.config(bg=self._c("bg"))
        self._dashboard_users_label.config(text=self._t("users_count") + ": ", bg=self._c("bg"), fg=self._c("fg_dim"))
        self._dashboard_users_value.config(bg=self._c("bg"), fg=self._c("fg"))
        self._dashboard_channels_label.config(text="    " + self._t("bound_channels") + ": ", bg=self._c("bg"), fg=self._c("fg_dim"))
        self._dashboard_channels_value.config(bg=self._c("bg"), fg=self._c("adapters_value_fg"))
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
        for child in self._logs_body.winfo_children():
            child.destroy()
        items = self.log_entries if isinstance(self.log_entries, list) else []
        if not items:
            tk.Label(
                self._logs_body,
                text=self._t("logs_empty"),
                font=("", 11),
                bg=self._c("bg"),
                fg=self._c("fg_dim"),
                anchor="w",
                justify=tk.LEFT,
            ).pack(anchor=tk.W)
            return
        list_wrapper = tk.Frame(self._logs_body, bg=self._c("bg"))
        list_wrapper.pack(fill=tk.BOTH, expand=True)
        canvas = tk.Canvas(list_wrapper, bg=self._c("bg"), highlightthickness=0)
        inner = tk.Frame(canvas, bg=self._c("bg"))
        win_id = canvas.create_window((0, 0), window=inner, anchor=tk.NW)

        def _on_inner_configure(_event):
            canvas.configure(scrollregion=canvas.bbox("all"))

        def _on_canvas_configure(event):
            canvas.itemconfig(win_id, width=event.width)

        inner.bind("<Configure>", _on_inner_configure)
        canvas.bind("<Configure>", _on_canvas_configure)
        canvas.pack(fill=tk.BOTH, expand=True)

        color_map = {
            "LLM": self._c("summary_llm"),
            "TASK": self._c("summary_task"),
            "ERROR": self._c("summary_error"),
            "ROUTING": self._c("summary_routing"),
            "TOOL": self._c("summary_tool"),
            "SKILL": self._c("summary_skill"),
            "OTHER": self._c("summary_other"),
        }
        for item in items:
            time_label = item.get("time") or "--:--:--"
            detail = item.get("detail") or item.get("raw") or ""
            kind = item.get("kind") or "OTHER"
            row = tk.Frame(inner, bg=self._c("bg"), height=18)
            row.pack(fill=tk.X, pady=0)
            row.pack_propagate(False)
            tk.Label(
                row,
                text=f"{time_label} {detail}",
                font=("", 9),
                bg=self._c("bg"),
                fg=color_map.get(kind, self._c("fg")),
                anchor="w",
                justify=tk.LEFT,
            ).pack(fill=tk.X, padx=(2, 0))

        def _scroll(evt):
            if getattr(evt, "num", None) == 5 or getattr(evt, "delta", 0) == -120:
                canvas.yview_scroll(4, "units")
            else:
                canvas.yview_scroll(-4, "units")

        def _bind_scroll(widget):
            widget.bind("<MouseWheel>", _scroll)
            widget.bind("<Button-4>", lambda e: canvas.yview_scroll(-4, "units"))
            widget.bind("<Button-5>", lambda e: canvas.yview_scroll(4, "units"))

        _bind_scroll(canvas)
        _bind_scroll(inner)
        for row in inner.winfo_children():
            _bind_scroll(row)
            for child in row.winfo_children():
                _bind_scroll(child)
        try:
            canvas.update_idletasks()
            canvas.yview_moveto(1.0)
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

    def _rebuild_ui(self):
        """主题切换后重建界面。"""
        for w in self.root.winfo_children():
            w.destroy()
        self._i18n.clear()
        self.gif_frames.clear()
        self.gif_delays.clear()
        self._build_ui()
        self._schedule_refresh()
        self._tick_time()
        if self.gif_frames:
            self._animate_gif()

    def _on_settings_ok(self):
        self._lang = self._settings_lang_var.get()
        new_theme = self._settings_theme_var.get()
        save_lang(self._lang)
        if new_theme != self._theme:
            self._theme = new_theme
            save_theme(self._theme)
            self._rebuild_ui()
            return
        self._apply_lang()
        self.settings_frame.pack_forget()
        self.dashboard_frame.pack(fill=tk.BOTH, expand=True)
        self._view_mode = "dashboard"
        self._refresh_health_once()

    def _on_settings_cancel(self):
        self.settings_frame.pack_forget()
        self.dashboard_frame.pack(fill=tk.BOTH, expand=True)
        self._view_mode = "dashboard"

    def _on_settings_restart(self):
        """后台执行 rustclaw -restart release all --quick --skip-setup；15 秒内按钮禁用并显示「重启中.....」。"""
        btn = self._settings_restart_btn
        if btn["state"] == tk.DISABLED:
            return
        btn.config(state=tk.DISABLED, text=self._t("restarting"))
        try:
            subprocess.Popen(
                ["rustclaw", "-restart", "release", "all", "--quick", "--skip-setup"],
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

        self.root.after(15000, reenable)

    def _toggle_view(self):
        """左滑/下一页：dashboard -> users -> logs -> skills -> stock -> crypto -> gallery -> settings -> dashboard"""
        if self._gallery_job:
            self.root.after_cancel(self._gallery_job)
            self._gallery_job = None
        if self._view_mode == "dashboard":
            self._view_mode = "users"
            self.dashboard_frame.pack_forget()
            self._prepare_users_view()
            self.users_frame.pack(fill=tk.BOTH, expand=True)
        elif self._view_mode == "users":
            self._view_mode = "logs"
            self.users_frame.pack_forget()
            self._prepare_logs_view()
            self.logs_frame.pack(fill=tk.BOTH, expand=True)
        elif self._view_mode == "logs":
            self._view_mode = "skills"
            self.logs_frame.pack_forget()
            self.skills_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._refresh_skills_view()
        elif self._view_mode == "skills":
            self._view_mode = "stock"
            self.skills_frame.pack_forget()
            self.stock_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_stock()
        elif self._view_mode == "stock":
            if self._stock_job:
                self.root.after_cancel(self._stock_job)
                self._stock_job = None
            self.stock_frame.pack_forget()
            self._view_mode = "crypto"
            self.crypto_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_crypto()
        elif self._view_mode == "crypto":
            self._view_mode = "gallery"
            if self._crypto_job:
                self.root.after_cancel(self._crypto_job)
                self._crypto_job = None
            self.crypto_frame.pack_forget()
            self.gallery_frame.pack(fill=tk.BOTH, expand=True, padx=(2, 14), pady=4)
            self._show_gallery()
        elif self._view_mode == "gallery":
            self._view_mode = "settings"
            self.gallery_frame.pack_forget()
            self._prepare_settings_view()
            self.settings_frame.pack(fill=tk.BOTH, expand=True)
        else:
            self._view_mode = "dashboard"
            self.settings_frame.pack_forget()
            self.dashboard_frame.pack(fill=tk.BOTH, expand=True)
        self._refresh_topbar()

    def _go_prev_view(self):
        """右滑/上一页：dashboard -> settings -> gallery -> crypto -> stock -> skills -> logs -> users -> dashboard（循环）。"""
        if self._gallery_job:
            self.root.after_cancel(self._gallery_job)
            self._gallery_job = None
        if self._view_mode == "dashboard":
            self._view_mode = "settings"
            self.dashboard_frame.pack_forget()
            self._prepare_settings_view()
            self.settings_frame.pack(fill=tk.BOTH, expand=True)
        elif self._view_mode == "settings":
            self._view_mode = "gallery"
            self.settings_frame.pack_forget()
            self.gallery_frame.pack(fill=tk.BOTH, expand=True, padx=(2, 14), pady=4)
            self._show_gallery()
        elif self._view_mode == "gallery":
            self._view_mode = "crypto"
            self.gallery_frame.pack_forget()
            self.crypto_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_crypto()
        elif self._view_mode == "crypto":
            self._view_mode = "stock"
            if self._crypto_job:
                self.root.after_cancel(self._crypto_job)
                self._crypto_job = None
            self.crypto_frame.pack_forget()
            self.stock_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._show_stock()
        elif self._view_mode == "stock":
            self._view_mode = "skills"
            if self._stock_job:
                self.root.after_cancel(self._stock_job)
                self._stock_job = None
            self.stock_frame.pack_forget()
            self.skills_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._refresh_skills_view()
        elif self._view_mode == "skills":
            self._view_mode = "logs"
            self.skills_frame.pack_forget()
            self._prepare_logs_view()
            self.logs_frame.pack(fill=tk.BOTH, expand=True)
        elif self._view_mode == "logs":
            self._view_mode = "users"
            self.logs_frame.pack_forget()
            self._prepare_users_view()
            self.users_frame.pack(fill=tk.BOTH, expand=True)
        else:
            self._view_mode = "dashboard"
            self.users_frame.pack_forget()
            self.dashboard_frame.pack(fill=tk.BOTH, expand=True)
        self._refresh_topbar()

    def _show_gallery(self):
        """NNI分布式模型页：Matrix 主题下无标题无按钮、矩阵雨占满屏并自动开始；非 Matrix 为标题+加入/停止+龙虾图。"""
        if self._llm_lobster_job:
            return
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
        if self._llm_lobster_job:
            try:
                self.root.after_cancel(self._llm_lobster_job)
            except tk.TclError:
                pass
            self._llm_lobster_job = None
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
        if self._theme == "matrix":
            self._llm_start_matrix_rain()
            return
        if self._llm_lobster_photo is None:
            self._llm_lobster_photo = self._llm_load_lobster_icon()
        if self._llm_lobster_photo:
            self._llm_lobster_tick()
        else:
            tk.Label(
                self._llm_content, text="(无 lobster.gif)", font=("", 12),
                bg=self._c("bg"), fg=self._c("status_off")
            ).pack(pady=20)
            self._llm_join_btn.config(text=self._t("llm_join"))

    def _llm_load_lobster_icon(self):
        """从 scripts/assets 加载 lobster.png 或 lobster.gif，缩成小图标，透明处叠到深色底（无白底）。"""
        assets = find_assets()
        for name in ("lobster.png", "lobster.gif"):
            path = os.path.join(assets, name)
            if not os.path.isfile(path):
                continue
            try:
                from PIL import Image, ImageTk
                img = Image.open(path)
                img = img.convert("RGBA")
                img = img.resize((22, 22), Image.Resampling.LANCZOS)
                bg_rgb = self._c("bg_rgb")
                if isinstance(bg_rgb, (list, tuple)) and len(bg_rgb) >= 3:
                    bg_rgb = tuple(int(x) for x in bg_rgb[:3])
                else:
                    bg_rgb = (0x1a, 0x1a, 0x2e)
                out = Image.new("RGB", img.size, bg_rgb)
                out.paste(img, mask=img.split()[3])
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
        if getattr(self, "_closing", False):
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
        self._llm_lobster_job = self.root.after(80, self._llm_matrix_tick)

    def _llm_lobster_tick(self):
        """每 0.5 秒多画一个龙虾小图，按行网格排；画满 6 行后清空重新画；切页后也继续画。"""
        if getattr(self, "_closing", False):
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
            self.root.after(0, lambda: self._update_crypto_prices(prices))
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
                self.root.after(0, lambda: self._update_crypto_prices(prices))
            except tk.TclError:
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
            self.root.after(0, lambda: self._update_crypto_prices(prices))
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
                self.root.after(0, lambda: self._update_stock_quotes(stock_data))
            except tk.TclError:
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
                self.root.after(0, lambda: self._update_stock_quotes(stock_data))
            except tk.TclError:
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
            self.root.after(0, lambda: self._update_stock_quotes(stock_data))

        threading.Thread(target=_fetch, daemon=True).start()
        self._stock_job = self.root.after(self._stock_refresh_sec * 1000, self._stock_refresh_loop)

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
            self.root.after(0, lambda: self._fill_skills_view(result))
        threading.Thread(target=_fetch, daemon=True).start()

    def _refresh_health_once(self):
        def _fetch():
            data, err = fetch_health(self._auth_key)
            logs, user_messages, _summary_err = fetch_clawd_activity(self._auth_key, self._lang)
            if getattr(self, "_closing", False):
                return
            try:
                self.root.after(0, lambda d=data, e=err, logs=logs, user_messages=user_messages: self._update(d, e, logs=logs, user_messages=user_messages))
            except tk.TclError:
                pass
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
        def loop():
            health_data = self.health
            health_err = self.error
            next_health_at = 0.0
            while not getattr(self, "_closing", False):
                now_ts = time.time()
                if now_ts >= next_health_at:
                    health_data, health_err = fetch_health(self._auth_key)
                    next_health_at = now_ts + HEALTH_REFRESH_SEC
                logs, user_messages, _summary_err = fetch_clawd_activity(self._auth_key, self._lang)
                if getattr(self, "_closing", False):
                    break
                try:
                    self.root.after(
                        0,
                        lambda d=health_data, e=health_err, logs=logs, user_messages=user_messages: self._update(
                            d, e, logs=logs, user_messages=user_messages
                        ),
                    )
                except tk.TclError:
                    break
                time.sleep(LOGS_REFRESH_SEC)
        t = threading.Thread(target=loop, daemon=True)
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
        # 通信端：TG 后显示 TG 占用内存，WA / WA-Web / FS(Feishu)
        parts = []
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
        self.adapters_var.set(", ".join(parts) if parts else "--")
        # 通信端占的内存（TG + WA + WA-Web + Feishu + Lark 进程 RSS 之和）
        def _n(v):
            return v if isinstance(v, (int, float)) and v is not None else 0
        total = (
            _n(data.get("telegramd_memory_rss_bytes"))
            + _n(data.get("whatsappd_memory_rss_bytes"))
            + _n(data.get("whatsapp_web_memory_rss_bytes"))
            + _n(data.get("feishud_memory_rss_bytes"))
            + _n(data.get("larkd_memory_rss_bytes"))
        )
        self.adapters_rss_var.set(fmt_bytes(int(total)) if total else "--")
        self._update_user_summary_view()
        self._refresh_topbar()
        from datetime import datetime
        self.foot_var.set(self._t("update_fmt").format(time=datetime.now().strftime("%H:%M:%S"), sec=LOGS_REFRESH_SEC))

    def _on_close(self):
        self._closing = True
        for attr in ("_blink_job", "_gallery_job", "_crypto_job", "_stock_job", "_llm_lobster_job"):
            job = getattr(self, attr, None)
            if job is not None:
                try:
                    self.root.after_cancel(job)
                except tk.TclError:
                    pass
                setattr(self, attr, None)
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
        try:
            self.root.lift()
            self.root.attributes("-topmost", True)
            self.root.after(400, self._clear_topmost)
            self.root.focus_force()
        except tk.TclError:
            pass

    def _clear_topmost(self):
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
        self.root.after(300, self._raise_window)

    def _on_swipe_start(self, event):
        self._swipe_start_x = getattr(self, "_swipe_start_x", 0)
        self._swipe_start_y = getattr(self, "_swipe_start_y", 0)
        self._swipe_start_x = event.x
        self._swipe_start_y = event.y

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
        # 设置页左滑（下一页）= 按顺序到首页
        if self._view_mode == "settings":
            self._settings_frame_to_dashboard()
            return
        self._toggle_view()

    def _on_swipe_prev(self, _event=None):
        if getattr(self, "_closing", False):
            return
        if getattr(self, "_view_mode", None) is None:
            return
        # 设置页右滑（上一页）= 按顺序到用户页
        if self._view_mode == "settings":
            self._settings_frame_to_users()
            return
        self._go_prev_view()

    def _settings_frame_to_dashboard(self):
        """设置 → 首页（左滑下一页）。"""
        self.settings_frame.pack_forget()
        self.dashboard_frame.pack(fill=tk.BOTH, expand=True)
        self._view_mode = "dashboard"

    def _settings_frame_to_users(self):
        """设置 → 用户页（右滑上一页）。"""
        self.settings_frame.pack_forget()
        self._prepare_users_view()
        self.users_frame.pack(fill=tk.BOTH, expand=True)
        self._view_mode = "users"

    def _toggle_fullscreen(self):
        self.root.attributes("-fullscreen", not self.root.attributes("-fullscreen"))

    def _tick_time(self):
        if getattr(self, "_closing", False):
            return
        from datetime import datetime
        self.time_var.set(datetime.now().strftime("%H:%M:%S"))
        self.root.after(1000, self._tick_time)

    def _animate_gif(self):
        if getattr(self, "_closing", False) or not self.gif_frames or not self.gif_delays:
            return
        self.lobster_label.configure(image=self.gif_frames[self.gif_frame_idx])
        delay = self.gif_delays[self.gif_frame_idx]
        self.gif_frame_idx = (self.gif_frame_idx + 1) % len(self.gif_frames)
        self.root.after(delay, self._animate_gif)

    def run(self):
        self.root.mainloop()


if __name__ == "__main__":
    app = SmallScreenApp()
    app.run()
