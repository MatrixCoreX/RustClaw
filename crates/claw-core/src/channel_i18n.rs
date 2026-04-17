use std::collections::HashMap;
use std::fs;
use std::sync::{Mutex, OnceLock};

use toml::Value as TomlValue;

static I18N_DICT_CACHE: OnceLock<Mutex<HashMap<String, HashMap<String, String>>>> = OnceLock::new();

fn load_i18n_dict(i18n_path: &str) -> HashMap<String, String> {
    let Ok(raw) = fs::read_to_string(i18n_path) else {
        return HashMap::new();
    };
    let Ok(value) = toml::from_str::<TomlValue>(&raw) else {
        return HashMap::new();
    };
    let Some(dict) = value.get("dict").and_then(|v| v.as_table()) else {
        return HashMap::new();
    };
    let mut out = HashMap::new();
    for (k, v) in dict {
        if let Some(text) = v.as_str() {
            out.insert(k.to_string(), text.to_string());
        }
    }
    out
}

pub fn text_from_path(i18n_path: &str, key: &str, fallback: &str) -> String {
    if i18n_path.trim().is_empty() {
        return fallback.to_string();
    }
    let cache = I18N_DICT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = match cache.lock() {
        Ok(g) => g,
        Err(_) => return fallback.to_string(),
    };
    let dict = guard
        .entry(i18n_path.to_string())
        .or_insert_with(|| load_i18n_dict(i18n_path));
    dict.get(key)
        .cloned()
        .unwrap_or_else(|| fallback.to_string())
}

pub fn text_with_vars_from_path(
    i18n_path: &str,
    key: &str,
    vars: &[(&str, &str)],
    fallback: &str,
) -> String {
    let mut out = text_from_path(i18n_path, key, fallback);
    for (name, value) in vars {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}
