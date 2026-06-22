use std::path::{Path, PathBuf};

const CONFIG_REL: &str = "configs/config.toml";
const DEFAULT_SQLITE_PATH: &str = "data/rustclaw.db";

fn find_workspace_root() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("RUSTCLAW_WORKSPACE") {
        let p = Path::new(s.trim());
        if !p.as_os_str().is_empty() && p.join(CONFIG_REL).exists() {
            return Some(p.to_path_buf());
        }
    }
    let mut dir = std::env::current_dir().ok()?;
    loop {
        if dir.join(CONFIG_REL).exists() {
            return Some(dir);
        }
        dir = dir.parent()?.to_path_buf();
        if dir.as_os_str().is_empty() {
            return None;
        }
    }
}

fn sqlite_path_from_config() -> Option<PathBuf> {
    let root = find_workspace_root()?;
    let config_path = root.join(CONFIG_REL);
    let raw = std::fs::read_to_string(&config_path).ok()?;
    let value: toml::Value = toml::from_str(&raw).ok()?;
    let path_str = value.get("database")?.get("sqlite_path")?.as_str()?.trim();
    if path_str.is_empty() {
        return Some(root.join(DEFAULT_SQLITE_PATH));
    }
    let p = Path::new(path_str);
    if p.is_absolute() {
        Some(p.to_path_buf())
    } else {
        Some(root.join(p))
    }
}

fn admin_key_from_db() -> Option<String> {
    let db_path = sqlite_path_from_config()
        .or_else(|| find_workspace_root().map(|root| root.join(DEFAULT_SQLITE_PATH)))?;
    let db = rusqlite::Connection::open(&db_path).ok()?;
    let mut stmt = db
        .prepare("SELECT user_key FROM auth_keys WHERE role = 'admin' AND enabled = 1 LIMIT 1")
        .ok()?;
    let mut rows = stmt.query([]).ok()?;
    let row = rows.next().ok()??;
    let user_key: String = row.get(0).ok()?;
    if user_key.trim().is_empty() {
        return None;
    }
    Some(user_key)
}

pub(crate) fn default_admin_key() -> Option<String> {
    if let Ok(s) = std::env::var("RUSTCLAW_ADMIN_KEY") {
        let t = s.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    admin_key_from_db()
}

pub(crate) fn key_required_error() -> anyhow::Error {
    let hint = if find_workspace_root().is_none() {
        "workspace_not_found: current directory and parents do not contain configs/config.toml; set RUSTCLAW_WORKSPACE or run from the project root"
    } else {
        "admin_key_not_found: auth_keys has no enabled admin key; start clawd first or pass -k/--key"
    };
    anyhow::anyhow!("key_required: pass -k/--key or set RUSTCLAW_ADMIN_KEY; {hint}")
}
