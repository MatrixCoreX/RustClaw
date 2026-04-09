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


def _lang_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_lang")


def _theme_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_theme")


def _stock_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_stock")


def _us_stock_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_us_stock")


def _messages_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_messages")


def _logs_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_logs")


def _gallery_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_gallery")


def _skills_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_skills")


def _weather_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_weather")


def _crypto_page_visible_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_show_crypto")


def _key_file():
    return os.path.join(_writable_pi_app_dir(), ".rustclaw_small_screen_key")


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


def load_theme():
    try:
        with open(_theme_file(), "r", encoding="utf-8") as f:
            theme = f.read().strip().lower()
            if theme in ("default", "matrix"):
                return theme
    except Exception:
        pass
    return "default"


def save_theme(theme):
    try:
        with open(_theme_file(), "w", encoding="utf-8") as f:
            f.write(theme)
    except Exception:
        pass


def _load_bool_setting(path, default=True):
    try:
        with open(path, "r", encoding="utf-8") as f:
            value = f.read().strip().lower()
    except Exception:
        return default
    if value in ("1", "true", "yes", "on"):
        return True
    if value in ("0", "false", "no", "off"):
        return False
    return default


def _save_bool_setting(path, value):
    try:
        with open(path, "w", encoding="utf-8") as f:
            f.write("1" if value else "0")
    except Exception:
        pass


def load_stock_page_visible():
    return _load_bool_setting(_stock_page_visible_file(), default=True)


def save_stock_page_visible(visible):
    _save_bool_setting(_stock_page_visible_file(), visible)


def load_us_stock_page_visible():
    return _load_bool_setting(_us_stock_page_visible_file(), default=True)


def save_us_stock_page_visible(visible):
    _save_bool_setting(_us_stock_page_visible_file(), visible)


def load_messages_page_visible():
    return _load_bool_setting(_messages_page_visible_file(), default=True)


def save_messages_page_visible(visible):
    _save_bool_setting(_messages_page_visible_file(), visible)


def load_logs_page_visible():
    return _load_bool_setting(_logs_page_visible_file(), default=True)


def save_logs_page_visible(visible):
    _save_bool_setting(_logs_page_visible_file(), visible)


def load_gallery_page_visible():
    return _load_bool_setting(_gallery_page_visible_file(), default=True)


def save_gallery_page_visible(visible):
    _save_bool_setting(_gallery_page_visible_file(), visible)


def load_skills_page_visible():
    return _load_bool_setting(_skills_page_visible_file(), default=True)


def save_skills_page_visible(visible):
    _save_bool_setting(_skills_page_visible_file(), visible)


def load_weather_page_visible():
    return _load_bool_setting(_weather_page_visible_file(), default=True)


def save_weather_page_visible(visible):
    _save_bool_setting(_weather_page_visible_file(), visible)


def load_crypto_page_visible():
    return _load_bool_setting(_crypto_page_visible_file(), default=True)


def save_crypto_page_visible(visible):
    _save_bool_setting(_crypto_page_visible_file(), visible)


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
