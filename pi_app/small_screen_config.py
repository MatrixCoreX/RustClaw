import json
import os
import secrets
import sqlite3
import sys

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None


def _pi_app_dir():
    if getattr(sys, "frozen", False):
        return sys._MEIPASS
    return os.path.dirname(os.path.abspath(__file__))


def _writable_pi_app_dir():
    if getattr(sys, "frozen", False):
        return os.path.dirname(os.path.abspath(sys.executable))
    return os.path.dirname(os.path.abspath(__file__))


def _settings_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_config.json")


def _default_settings():
    return {
        "lang": _default_lang_from_system(),
        "theme": "default",
        "show_stock": True,
        "show_us_stock": True,
        "show_messages": True,
        "show_logs": True,
        "show_gallery": True,
        "show_skills": True,
        "show_weather": True,
        "show_crypto": True,
        "user_key": "",
    }


def _root_dir():
    if getattr(sys, "frozen", False):
        exe = os.path.abspath(sys.executable)
        cur = os.path.dirname(exe)
        for _ in range(6):
            trial = os.path.join(cur, "configs", "config.toml")
            if os.path.isfile(trial):
                return cur
            parent = os.path.dirname(cur)
            if parent == cur:
                break
            cur = parent
        exe_dir = os.path.dirname(exe)
        return os.path.normpath(os.path.join(exe_dir, "..", "..", ".."))
    script_dir = os.path.dirname(os.path.abspath(__file__))
    return os.path.dirname(script_dir)


def _config_path():
    return os.path.join(_root_dir(), "configs", "config.toml")


def _load_settings_dict():
    try:
        with open(_settings_file(), "r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except Exception:
        return {}


def _save_settings_dict(settings):
    try:
        with open(_settings_file(), "w", encoding="utf-8") as f:
            json.dump(settings, f, ensure_ascii=True, indent=2, sort_keys=True)
    except Exception:
        pass


def _save_setting_value(name, value):
    settings = _load_settings_dict()
    settings[name] = value
    _save_settings_dict(settings)


def migrate_small_screen_settings(remove_legacy=False):
    _ = remove_legacy
    settings = _load_settings_dict()
    defaults = _default_settings()
    merged = {
        "lang": str(settings.get("lang") or defaults["lang"]).upper(),
        "theme": str(settings.get("theme") or defaults["theme"]).lower(),
        "show_stock": bool(settings.get("show_stock", defaults["show_stock"])),
        "show_us_stock": bool(settings.get("show_us_stock", defaults["show_us_stock"])),
        "show_messages": bool(settings.get("show_messages", defaults["show_messages"])),
        "show_logs": bool(settings.get("show_logs", defaults["show_logs"])),
        "show_gallery": bool(settings.get("show_gallery", defaults["show_gallery"])),
        "show_skills": bool(settings.get("show_skills", defaults["show_skills"])),
        "show_weather": bool(settings.get("show_weather", defaults["show_weather"])),
        "show_crypto": bool(settings.get("show_crypto", defaults["show_crypto"])),
        "user_key": str(settings.get("user_key") or defaults["user_key"]).strip(),
    }
    if merged["lang"] not in ("EN", "CN"):
        merged["lang"] = _default_lang_from_system()
    if merged["theme"] not in ("default", "matrix"):
        merged["theme"] = "default"
    _save_settings_dict(merged)
    return merged


def load_theme():
    settings = _load_settings_dict()
    theme = str(settings.get("theme") or "").strip().lower()
    if theme in ("default", "matrix"):
        return theme
    return "default"


def save_theme(theme):
    _save_setting_value("theme", str(theme).strip().lower())


def load_stock_page_visible():
    settings = _load_settings_dict()
    if "show_stock" in settings:
        return bool(settings.get("show_stock"))
    return True


def save_stock_page_visible(visible):
    _save_setting_value("show_stock", bool(visible))


def load_us_stock_page_visible():
    settings = _load_settings_dict()
    if "show_us_stock" in settings:
        return bool(settings.get("show_us_stock"))
    return True


def save_us_stock_page_visible(visible):
    _save_setting_value("show_us_stock", bool(visible))


def load_messages_page_visible():
    settings = _load_settings_dict()
    if "show_messages" in settings:
        return bool(settings.get("show_messages"))
    return True


def save_messages_page_visible(visible):
    _save_setting_value("show_messages", bool(visible))


def load_logs_page_visible():
    settings = _load_settings_dict()
    if "show_logs" in settings:
        return bool(settings.get("show_logs"))
    return True


def save_logs_page_visible(visible):
    _save_setting_value("show_logs", bool(visible))


def load_gallery_page_visible():
    settings = _load_settings_dict()
    if "show_gallery" in settings:
        return bool(settings.get("show_gallery"))
    return True


def save_gallery_page_visible(visible):
    _save_setting_value("show_gallery", bool(visible))


def load_skills_page_visible():
    settings = _load_settings_dict()
    if "show_skills" in settings:
        return bool(settings.get("show_skills"))
    return True


def save_skills_page_visible(visible):
    _save_setting_value("show_skills", bool(visible))


def load_weather_page_visible():
    settings = _load_settings_dict()
    if "show_weather" in settings:
        return bool(settings.get("show_weather"))
    return True


def save_weather_page_visible(visible):
    _save_setting_value("show_weather", bool(visible))


def load_crypto_page_visible():
    settings = _load_settings_dict()
    if "show_crypto" in settings:
        return bool(settings.get("show_crypto"))
    return True


def save_crypto_page_visible(visible):
    _save_setting_value("show_crypto", bool(visible))


def load_auth_key():
    settings = _load_settings_dict()
    if "user_key" in settings:
        return str(settings.get("user_key") or "").strip()
    return ""


def save_auth_key(user_key):
    _save_setting_value("user_key", (user_key or "").strip())


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
        with sqlite3.connect(db_path) as conn:
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
        return user_key
    except Exception:
        return user_key


def load_enabled_admin_user_key():
    db_path = _load_sqlite_path_from_config()
    try:
        conn = sqlite3.connect(db_path)
        row = conn.execute(
            """
            SELECT user_key
            FROM auth_keys
            WHERE role = 'admin' AND enabled = 1
            ORDER BY rowid ASC
            LIMIT 1
            """
        ).fetchone()
        conn.close()
        if row and row[0]:
            return str(row[0]).strip()
    except Exception:
        pass
    return ""


def _default_lang_from_system():
    try:
        import locale
        loc, _ = locale.getdefaultlocale()
        if loc and loc.lower().startswith("zh"):
            return "CN"
    except Exception:
        pass
    for key in ("LANG", "LC_ALL", "LANGUAGE"):
        val = os.environ.get(key, "")
        if isinstance(val, str) and val.lower().startswith("zh"):
            return "CN"
    return "EN"


def load_lang():
    settings = _load_settings_dict()
    lang = str(settings.get("lang") or "").strip().upper()
    if lang in ("EN", "CN"):
        return lang
    return _default_lang_from_system()


def save_lang(lang):
    _save_setting_value("lang", str(lang).strip().upper())
