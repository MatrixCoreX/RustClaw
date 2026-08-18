import json
import logging
import os
import secrets
import sqlite3
import sys
from contextlib import closing

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None


logger = logging.getLogger(__name__)


def _pi_app_dir():
    if getattr(sys, "frozen", False):
        return sys._MEIPASS
    return os.path.dirname(os.path.abspath(__file__))


def _writable_pi_app_dir():
    if getattr(sys, "frozen", False):
        return os.path.dirname(os.path.abspath(sys.executable))
    return os.path.dirname(os.path.abspath(__file__))


def _settings_file():
    root = _writable_pi_app_dir()
    return os.path.join(root, ".agent_small_screen_config.json")


def _default_settings():
    return {
        "lang": _default_lang_from_system(),
        "theme": "default",
        "show_stock": True,
        "show_us_stock": True,
        "show_messages": True,
        "show_logs": True,
        "show_gallery": True,
        "show_weather": True,
        "show_crypto": True,
        "show_bancor": True,
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


def _product_identity_path():
    selected = os.environ.get("APP_PRODUCT_IDENTITY_CONFIG", "").strip()
    return selected or os.path.join(_root_dir(), "configs", "product_identity.toml")


def _load_product_identity():
    if tomllib is None:
        raise RuntimeError("Python 3.11 or newer is required to read product identity")
    path = _product_identity_path()
    try:
        with open(path, "rb") as source:
            config = tomllib.load(source)
    except (OSError, ValueError) as exc:
        raise RuntimeError(f"Unable to read product identity config: {path}") from exc
    if config.get("schema_version") != 1:
        raise RuntimeError(f"Unsupported product identity schema: {path}")
    return config, path


def load_product_display_name():
    config, path = _load_product_identity()
    display_name = str(config.get("display_name") or "").strip()
    if not display_name:
        raise RuntimeError(f"Product identity display_name is missing: {path}")
    return display_name


def load_product_splash_image():
    config, path = _load_product_identity()
    filename = str(config.get("small_screen_splash_image") or "").strip()
    if not filename or filename in (".", "..") or "/" in filename or "\\" in filename:
        raise RuntimeError(f"Product identity small_screen_splash_image is invalid: {path}")
    return filename


def _load_settings_dict():
    try:
        path = _settings_file()
        os.chmod(path, 0o600)
        with open(path, "r", encoding="utf-8") as f:
            data = json.load(f)
        return data if isinstance(data, dict) else {}
    except FileNotFoundError:
        return {}
    except Exception as exc:
        logger.warning("Unable to load small-screen settings: %s", exc)
        return {}


def _save_settings_dict(settings):
    try:
        path = _settings_file()
        flags = os.O_WRONLY | os.O_CREAT | os.O_TRUNC
        if hasattr(os, "O_NOFOLLOW"):
            flags |= os.O_NOFOLLOW
        fd = os.open(path, flags, 0o600)
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(settings, f, ensure_ascii=True, indent=2, sort_keys=True)
    except Exception as exc:
        logger.warning("Unable to save small-screen settings: %s", exc)


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
        "show_weather": bool(settings.get("show_weather", defaults["show_weather"])),
        "show_crypto": bool(settings.get("show_crypto", defaults["show_crypto"])),
        "show_bancor": bool(settings.get("show_bancor", defaults["show_bancor"])),
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


def load_bancor_page_visible():
    settings = _load_settings_dict()
    if "show_bancor" in settings:
        return bool(settings.get("show_bancor"))
    return True


def save_bancor_page_visible(visible):
    _save_setting_value("show_bancor", bool(visible))


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
        raise RuntimeError("Python 3.11 or newer is required to read runtime configuration")
    try:
        with open(_config_path(), "rb") as f:
            cfg = tomllib.load(f)
        db_rel = str(((cfg or {}).get("database") or {}).get("sqlite_path") or "").strip()
        if not db_rel:
            raise RuntimeError("database.sqlite_path is missing")
        return os.path.join(_root_dir(), db_rel)
    except (OSError, ValueError, TypeError) as exc:
        raise RuntimeError(f"Unable to read runtime database path: {_config_path()}") from exc


def _generate_user_key():
    return "rk-" + secrets.token_urlsafe(18)


def ensure_small_screen_auth_key():
    user_key = load_auth_key().strip()
    db_path = _load_sqlite_path_from_config()
    try:
        os.makedirs(os.path.dirname(db_path), exist_ok=True)
        with closing(sqlite3.connect(db_path)) as conn:
            with conn:
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
    except Exception as exc:
        logger.warning("Unable to register the small-screen fallback auth key: %s", exc)
        return user_key


def load_enabled_admin_user_key():
    db_path = _load_sqlite_path_from_config()
    try:
        with closing(sqlite3.connect(db_path)) as conn:
            row = conn.execute(
                """
                SELECT user_key
                FROM auth_keys
                WHERE role = 'admin' AND enabled = 1
                ORDER BY rowid ASC
                LIMIT 1
                """
            ).fetchone()
        if row and row[0]:
            return str(row[0]).strip()
    except Exception as exc:
        logger.warning("Unable to load the enabled admin key for the small-screen app: %s", exc)
    return ""


def load_preferred_runtime_auth_key():
    """Use the runtime's enabled admin identity without persisting it in app settings."""
    admin_key = load_enabled_admin_user_key()
    if admin_key:
        return admin_key
    return ensure_small_screen_auth_key()


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
