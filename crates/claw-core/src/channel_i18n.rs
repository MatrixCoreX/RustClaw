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
        collect_i18n_dict_entries(k, v, &mut out);
    }
    out
}

fn collect_i18n_dict_entries(prefix: &str, value: &TomlValue, out: &mut HashMap<String, String>) {
    if let Some(text) = value.as_str() {
        out.insert(prefix.to_string(), text.to_string());
        return;
    }
    let Some(table) = value.as_table() else {
        return;
    };
    for (key, child) in table {
        let child_key = format!("{prefix}.{key}");
        collect_i18n_dict_entries(&child_key, child, out);
    }
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

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn temp_i18n_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("rustclaw_channel_i18n_{name}_{unique}.toml"))
    }

    #[test]
    fn text_from_path_flattens_dotted_dict_keys() {
        let path = temp_i18n_path("dotted");
        std::fs::write(
            &path,
            "[dict]\ncrypto.err.account_access_failed = \"ACCOUNT_ACCESS\"\n\"flat.key\" = \"FLAT\"\n",
        )
        .expect("write i18n");
        let path_text = path.to_string_lossy();

        assert_eq!(
            text_from_path(
                path_text.as_ref(),
                "crypto.err.account_access_failed",
                "fallback"
            ),
            "ACCOUNT_ACCESS"
        );
        assert_eq!(
            text_from_path(path_text.as_ref(), "flat.key", "fallback"),
            "FLAT"
        );

        let _ = std::fs::remove_file(path);
    }
}
