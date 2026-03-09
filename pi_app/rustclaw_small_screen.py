#!/usr/bin/env python3
# RustClaw 小屏监控：480×320 全屏，请求 /v1/health 每 15 秒刷新，左侧龙虾动图 + RustClaw 标题。
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
REFRESH_SEC = 15
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
        "settings_title": "设置",
        "language": "语言",
        "lang_en": "EN",
        "lang_cn": "CN",
        "ok": "确定",
        "cancel": "取消",
        "crypto_refresh_hint": "每15秒自动刷新",
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
        "settings_title": "Settings",
        "language": "Language",
        "lang_en": "EN",
        "lang_cn": "CN",
        "ok": "OK",
        "cancel": "Cancel",
        "crypto_refresh_hint": "Auto refresh every 15s",
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


def _extract_log_detail(line):
    line = line or ""
    routed = re.search(
        r"route_request_mode llm .* mode=(ChatAct|AskClarify|Chat|Act)\s+confidence=([0-9.]+)\s+reason=(.+?)\s+evidence_refs=",
        line,
    )
    if routed:
        mode = routed.group(1)
        confidence = routed.group(2)
        reason = " ".join((routed.group(3) or "").split())
        reason = re.split(r"[;。]|(?:\s+-\s+)", reason, maxsplit=1)[0].strip()
        if mode in ("ChatAct", "AskClarify") and reason:
            if len(reason) > 28:
                reason = reason[:28].rstrip() + "..."
            return f"{mode} reason={reason}"
        return f"{mode} {confidence}"
    mode = re.search(r"routed_mode=(ChatAct|AskClarify|Chat|Act)\b", line)
    if mode:
        return mode.group(1)
    prompt = re.search(r"prompt_name=([A-Za-z0-9_./-]+)", line)
    if prompt:
        return prompt.group(1)
    tool = re.search(r"type=call_tool tool=([A-Za-z0-9_./:-]+)", line)
    if tool:
        return tool.group(1)
    skill = re.search(r"type=call_skill(?:\(rerouted\))? skill=([A-Za-z0-9_./:-]+)", line)
    if skill:
        return skill.group(1)
    task_status = re.search(r"task_call_end .* status=([A-Za-z0-9_-]+)", line)
    if task_status:
        return task_status.group(1)
    return ""


def _summarize_clawd_log_text(raw_text, lang="CN"):
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
            items.append({"time": time_label, "kind": item, "detail": _extract_log_detail(line)})
        if len(items) >= 8:
            break
    return items


def fetch_clawd_log_summary(user_key="", lang="CN"):
    try:
        query = urllib.parse.urlencode({"file": "clawd.log", "lines": 80})
        req = _build_api_request("/v1/logs/latest?" + query, user_key)
        with urllib.request.urlopen(req, timeout=5) as r:
            body = json.loads(r.read().decode())
        data = body.get("data") or body or {}
        return _summarize_clawd_log_text(data.get("text") or "", lang=lang), None
    except Exception as e:
        return None, str(e)


BINANCE_TICKER_URL = "https://api.binance.com/api/v3/ticker/price"
CRYPTO_PAIRS = [
    ("BTC", "BTCUSDT"), ("ETH", "ETHUSDT"), ("BCH", "BCHUSDT"), ("LTC", "LTCUSDT"),
    ("SOL", "SOLUSDT"), ("BNB", "BNBUSDT"), ("XRP", "XRPUSDT"), ("DOGE", "DOGEUSDT"),
    ("PEPE", "PEPEUSDT"), ("SHIB", "SHIBUSDT"),
]


def _strip_trailing_zeros(price_str):
    """去掉价格字符串小数点后尾部的 0，若小数部分全为 0 则去掉小数点。"""
    s = str(price_str).strip()
    if "." not in s:
        return s
    int_part, _, frac = s.partition(".")
    frac = frac.rstrip("0")
    return int_part if not frac else f"{int_part}.{frac}"


def fetch_crypto_prices():
    """从币安 API 拉取 USDT 价格，返回 { "BTC": "43210.5", ... }，失败返回 None。去掉小数点后尾部的 0。"""
    try:
        req = urllib.request.Request(BINANCE_TICKER_URL)
        with urllib.request.urlopen(req, timeout=8) as r:
            data = json.loads(r.read().decode())
        if not isinstance(data, list):
            return None
        by_symbol = {item.get("symbol"): item.get("price") for item in data if isinstance(item, dict) and item.get("symbol") and item.get("price")}
        out = {}
        for name, symbol in CRYPTO_PAIRS:
            p = by_symbol.get(symbol)
            if p is not None:
                out[name] = _strip_trailing_zeros(p)
            else:
                out[name] = "--"
        return out
    except Exception:
        return None


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
            if "display" in str(e).lower() or "DISPLAY" in str(e):
                print("无法连接图形显示。请在有桌面的环境运行，或先设置：", file=sys.stderr)
                print("  export DISPLAY=:0", file=sys.stderr)
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
        tk.Label(
            top, text="RustClaw", font=("", 20, "bold"),
            bg=self._c("bg"), fg=self._c("accent")
        ).pack(side=tk.LEFT, padx=(0, 12))
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
        self.users_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=20, pady=18)
        self.settings_frame = tk.Frame(self.switch_container, bg=self._c("bg"), padx=24, pady=20)
        # 顺序（左滑下一页）：首页 → 用户 → 技能 → 加密货币 → 挖矿 → 设置 → 首页；右滑=上一页
        self._view_mode = "dashboard"  # dashboard | users | skills | crypto | gallery | settings
        self._crypto_job = None
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
        self.foot_var = tk.StringVar(value=_t("foot_prefix"))
        tk.Label(content, textvariable=self.foot_var, font=("", 11), bg=self._c("bg"), fg=self._c("foot_fg")).pack(anchor=tk.W)
        self.users_count_var = tk.StringVar(value="--")
        self.bound_channels_var = tk.StringVar(value="--")
        self.clawd_summary_var = tk.StringVar(value=_t("clawd_summary_empty"))
        self._users_body = tk.Frame(self.users_frame, bg=self._c("bg"))
        self._users_body.pack(fill=tk.BOTH, expand=True)
        self._users_left = tk.Frame(self._users_body, bg=self._c("bg"), width=132)
        self._users_left.pack(side=tk.LEFT, fill=tk.Y, anchor=tk.N)
        self._users_left.pack_propagate(False)
        self._users_right = tk.Frame(self._users_body, bg=self._c("bg"))
        self._users_right.pack(side=tk.RIGHT, fill=tk.BOTH, expand=True, padx=(12, 0))
        self._users_title_label = tk.Label(self._users_left, text=_t("users_title"), font=("", 16, "bold"), bg=self._c("bg"), fg=self._c("fg"))
        self._users_title_label.pack(anchor=tk.W, pady=(0, 16))
        self._users_count_label = tk.Label(self._users_left, text=_t("users_count"), font=("", 12), bg=self._c("bg"), fg=self._c("fg_dim"))
        self._users_count_label.pack(anchor=tk.W)
        self._users_count_value = tk.Label(self._users_left, textvariable=self.users_count_var, font=("", 26, "bold"), bg=self._c("bg"), fg=self._c("fg"))
        self._users_count_value.pack(anchor=tk.W, pady=(4, 14))
        self._bound_channels_label = tk.Label(self._users_left, text=_t("bound_channels"), font=("", 12), bg=self._c("bg"), fg=self._c("fg_dim"))
        self._bound_channels_label.pack(anchor=tk.W)
        self._bound_channels_value = tk.Label(self._users_left, textvariable=self.bound_channels_var, font=("", 22, "bold"), bg=self._c("bg"), fg=self._c("adapters_value_fg"))
        self._bound_channels_value.pack(anchor=tk.W, pady=(4, 0))
        self._clawd_summary_label = tk.Label(self._users_right, text=_t("clawd_summary"), font=("", 12, "bold"), bg=self._c("bg"), fg=self._c("fg"))
        self._clawd_summary_label.pack(anchor=tk.W, pady=(2, 8))
        self._clawd_summary_box = tk.Frame(self._users_right, bg=self._c("box_border"), padx=2, pady=2)
        self._clawd_summary_box.pack(fill=tk.BOTH, expand=True)
        self._clawd_summary_inner = tk.Frame(self._clawd_summary_box, bg=self._c("box_bg"), padx=8, pady=8)
        self._clawd_summary_inner.pack(fill=tk.BOTH, expand=True)
        self._clawd_summary_items = tk.Frame(self._clawd_summary_inner, bg=self._c("box_bg"))
        self._clawd_summary_items.pack(fill=tk.BOTH, expand=True, anchor=tk.NW)
        self._clawd_summary_value = tk.Label(
            self._clawd_summary_inner,
            textvariable=self.clawd_summary_var,
            justify=tk.LEFT,
            anchor="nw",
            font=("", 10),
            bg=self._c("box_bg"),
            fg=self._c("fg_dim"),
        )
        self._clawd_summary_value.pack(anchor=tk.NW)
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

    def _t(self, key):
        return STRINGS.get(self._lang, STRINGS["CN"]).get(key, key)

    def _c(self, key):
        return THEMES.get(self._theme, THEMES["default"]).get(key, "")

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
        self.foot_var.set(self._t("foot_prefix"))

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

    def _prepare_users_view(self):
        self._users_title_label.config(text=self._t("users_title"), bg=self._c("bg"), fg=self._c("fg"))
        self._users_count_label.config(text=self._t("users_count"), bg=self._c("bg"), fg=self._c("fg_dim"))
        self._bound_channels_label.config(text=self._t("bound_channels"), bg=self._c("bg"), fg=self._c("fg_dim"))
        self._clawd_summary_label.config(text=self._t("clawd_summary"), bg=self._c("bg"), fg=self._c("fg"))
        self._users_count_value.config(bg=self._c("bg"), fg=self._c("fg"))
        self._bound_channels_value.config(bg=self._c("bg"), fg=self._c("adapters_value_fg"))
        self._clawd_summary_box.config(bg=self._c("box_border"))
        self._clawd_summary_inner.config(bg=self._c("box_bg"))
        self._clawd_summary_items.config(bg=self._c("box_bg"))
        self._clawd_summary_value.config(bg=self._c("box_bg"), fg=self._c("fg_dim"))
        self._update_user_summary_view()

    def _update_user_summary_view(self):
        data = self.health if isinstance(self.health, dict) else {}
        user_count = data.get("user_count")
        bound_channel_count = data.get("bound_channel_count")
        self.users_count_var.set(str(user_count) if user_count is not None else "--")
        self.bound_channels_var.set(str(bound_channel_count) if bound_channel_count is not None else "--")
        self._render_clawd_summary_items()

    def _render_clawd_summary_items(self):
        color_map = {
            "LLM": self._c("summary_llm"),
            "TASK": self._c("summary_task"),
            "ERROR": self._c("summary_error"),
            "ROUTING": self._c("summary_routing"),
            "TOOL": self._c("summary_tool"),
            "SKILL": self._c("summary_skill"),
            "OTHER": self._c("summary_other"),
        }
        for child in self._clawd_summary_items.winfo_children():
            child.destroy()
        items = self.log_summary if isinstance(self.log_summary, list) else []
        if not items:
            self.clawd_summary_var.set(self._t("clawd_summary_empty"))
            self._clawd_summary_value.pack(anchor=tk.NW)
            return
        self._clawd_summary_value.pack_forget()
        for item in items:
            if isinstance(item, dict):
                time_label = item.get("time") or "--:--:--"
                kind = item.get("kind") or "OTHER"
                detail = item.get("detail") or ""
            else:
                time_label = "--:--:--"
                kind = str(item or "OTHER")
                detail = ""
            row = tk.Frame(self._clawd_summary_items, bg=self._c("box_bg"))
            row.pack(anchor=tk.W, fill=tk.X, pady=(0, 4))
            tk.Label(
                row,
                text=time_label,
                anchor="w",
                justify=tk.LEFT,
                width=8,
                font=("", 10),
                bg=self._c("box_bg"),
                fg=self._c("fg_dim"),
            ).pack(side=tk.LEFT, padx=(2, 8))
            tk.Label(
                row,
                text=kind,
                anchor="w",
                justify=tk.LEFT,
                padx=6,
                pady=3,
                font=("", 10, "bold"),
                bg=self._c("box_bg"),
                fg=color_map.get(kind, self._c("summary_other")),
            ).pack(side=tk.LEFT, fill=tk.X)
            if detail:
                tk.Label(
                    row,
                    text=detail,
                    anchor="w",
                    justify=tk.LEFT,
                    font=("", 9),
                    bg=self._c("box_bg"),
                    fg=self._c("fg"),
                ).pack(side=tk.LEFT, padx=(8, 0), fill=tk.X)

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
        """左滑/下一页：dashboard -> users -> skills -> crypto -> gallery -> settings -> dashboard"""
        if self._gallery_job:
            self.root.after_cancel(self._gallery_job)
            self._gallery_job = None
        if self._view_mode == "dashboard":
            self._view_mode = "users"
            self.dashboard_frame.pack_forget()
            self._prepare_users_view()
            self.users_frame.pack(fill=tk.BOTH, expand=True)
        elif self._view_mode == "users":
            self._view_mode = "skills"
            self.users_frame.pack_forget()
            self.skills_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._refresh_skills_view()
        elif self._view_mode == "skills":
            self._view_mode = "crypto"
            self.skills_frame.pack_forget()
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

    def _go_prev_view(self):
        """右滑/上一页：dashboard -> settings -> gallery -> crypto -> skills -> users -> dashboard（循环）。"""
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
            self._view_mode = "skills"
            if self._crypto_job:
                self.root.after_cancel(self._crypto_job)
                self._crypto_job = None
            self.crypto_frame.pack_forget()
            self.skills_frame.pack(fill=tk.BOTH, expand=True, padx=8, pady=4)
            self._refresh_skills_view()
        elif self._view_mode == "skills":
            self._view_mode = "users"
            self.skills_frame.pack_forget()
            self._prepare_users_view()
            self.users_frame.pack(fill=tk.BOTH, expand=True)
        else:
            self._view_mode = "dashboard"
            self.users_frame.pack_forget()
            self.dashboard_frame.pack(fill=tk.BOTH, expand=True)

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
            title_row, text=self._t("crypto_refresh_hint"), font=("", 10),
            bg=self._c("bg"), fg=self._c("foot_fg")
        ).pack(side=tk.RIGHT)
        box_bg = self._c("box_bg")
        box_border = self._c("box_border")
        cell_gap = 6
        cell_h = 28
        cell_w = (W - 16 - 8 - cell_gap) // 2
        self._crypto_vars = {}
        for i in range(0, len(CRYPTO_PAIRS), 2):
            row = tk.Frame(self.crypto_frame, bg=self._c("bg"), height=cell_h)
            row.pack(fill=tk.X, pady=2)
            row.pack_propagate(False)
            for j in range(2):
                if i + j >= len(CRYPTO_PAIRS):
                    break
                name, _ = CRYPTO_PAIRS[i + j]
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
            prices = fetch_crypto_prices()
            self.root.after(0, lambda: self._update_crypto_prices(prices))
        threading.Thread(target=_fetch_and_update, daemon=True).start()
        self._crypto_job = self.root.after(15000, self._crypto_refresh_loop)

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
            prices = fetch_crypto_prices()
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
            prices = fetch_crypto_prices()
            self.root.after(0, lambda: self._update_crypto_prices(prices))
        threading.Thread(target=_fetch, daemon=True).start()
        self._crypto_job = self.root.after(15000, self._crypto_refresh_loop)

    def _refresh_skills_view(self):
        for w in self.skills_frame.winfo_children():
            w.destroy()
        tk.Label(
            self.skills_frame, text=self._t("skills_title"), font=("DejaVu Sans", 14, "bold"),
            bg=self._c("bg"), fg=self._c("fg")
        ).pack(pady=(0, 6))
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
            summary, _summary_err = fetch_clawd_log_summary(self._auth_key, self._lang)
            if getattr(self, "_closing", False):
                return
            try:
                self.root.after(0, lambda d=data, e=err, s=summary: self._update(d, e, s))
            except tk.TclError:
                pass
        threading.Thread(target=_fetch, daemon=True).start()

    def _fill_skills_view(self, result):
        if getattr(self, "_closing", False) or self._view_mode != "skills":
            return
        all_skills, enabled_set = result if result else (None, None)
        for w in self.skills_frame.winfo_children():
            w.destroy()
        tk.Label(
            self.skills_frame, text=self._t("skills_title"), font=("DejaVu Sans", 14, "bold"),
            bg=self._c("bg"), fg=self._c("fg")
        ).pack(pady=(0, 6))
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

        _bind_scroll(canvas)
        _bind_scroll(inner)
        for row in inner.winfo_children():
            _bind_scroll(row)
            for child in row.winfo_children():
                _bind_scroll(child)

    def _schedule_refresh(self):
        def loop():
            while not getattr(self, "_closing", False):
                data, err = fetch_health(self._auth_key)
                summary, _summary_err = fetch_clawd_log_summary(self._auth_key, self._lang)
                if getattr(self, "_closing", False):
                    break
                try:
                    self.root.after(0, lambda d=data, e=err, s=summary: self._update(d, e, s))
                except tk.TclError:
                    break
                time.sleep(REFRESH_SEC)
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

    def _update(self, data, err, summary=None):
        if getattr(self, "_closing", False):
            return
        if summary is not None:
            self.log_summary = summary
        if err:
            self.health = None
            self._online = False
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
            self._render_clawd_summary_items()
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
        # 通信端：TG 后显示 TG 占用内存，WA / WA-Web
        parts = []
        if data.get("telegramd_healthy") or data.get("telegram_bot_healthy"):
            tg_rss = data.get("telegramd_memory_rss_bytes") or data.get("telegram_bot_memory_rss_bytes")
            parts.append("TG " + fmt_bytes(tg_rss) if tg_rss is not None else "TG")
        if data.get("whatsappd_healthy") or data.get("whatsapp_cloud_healthy"):
            parts.append("WA")
        if data.get("whatsapp_web_healthy"):
            parts.append("WA-Web")
        self.adapters_var.set(", ".join(parts) if parts else "--")
        # 通信端占的内存（TG + WA + WA-Web 进程 RSS 之和，接口用 telegramd/whatsappd/whatsapp_web）
        def _n(v):
            return v if isinstance(v, (int, float)) and v is not None else 0
        total = (
            _n(data.get("telegramd_memory_rss_bytes"))
            + _n(data.get("whatsappd_memory_rss_bytes"))
            + _n(data.get("whatsapp_web_memory_rss_bytes"))
        )
        self.adapters_rss_var.set(fmt_bytes(int(total)) if total else "--")
        self._update_user_summary_view()
        from datetime import datetime
        self.foot_var.set(self._t("update_fmt").format(time=datetime.now().strftime("%H:%M:%S"), sec=REFRESH_SEC))

    def _on_close(self):
        self._closing = True
        for attr in ("_blink_job", "_gallery_job", "_crypto_job", "_llm_lobster_job"):
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
