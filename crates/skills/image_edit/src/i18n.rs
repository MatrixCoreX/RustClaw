use std::collections::HashMap;

use super::*;

#[derive(Debug, Clone)]
pub(super) struct TextCatalog {
    current: HashMap<String, String>,
}

impl TextCatalog {
    pub(super) fn for_lang(workspace_root: &Path, cfg: &ImageSkillConfig, lang: &str) -> Self {
        let mut current = default_i18n_dict(lang);
        let lang_tag = normalize_lang_tag(lang);
        let default_path = workspace_root.join(format!("configs/i18n/image_edit.{lang_tag}.toml"));
        if let Some(external) = load_external_i18n(&default_path) {
            current.extend(external);
        }
        if let Some(custom) = cfg
            .i18n_path
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
        {
            let custom_path = if Path::new(custom).is_absolute() {
                PathBuf::from(custom)
            } else {
                workspace_root.join(custom)
            };
            if let Some(external) = load_external_i18n(&custom_path) {
                current.extend(external);
            }
        }
        Self { current }
    }

    pub(super) fn render(&self, key: &str, vars: &[(&str, String)], default: &str) -> String {
        let mut out = self
            .current
            .get(key)
            .cloned()
            .unwrap_or_else(|| default.to_string());
        for (k, v) in vars {
            out = out.replace(&format!("{{{k}}}"), v);
        }
        out
    }
}

pub(super) fn resolve_output_language(
    cfg: &RootConfig,
    obj: &serde_json::Map<String, Value>,
) -> String {
    obj.get("response_language")
        .or_else(|| obj.get("language"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(normalize_lang_tag)
        .or_else(|| {
            obj.get("_memory")
                .and_then(|m| m.get("lang_hint"))
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(normalize_lang_tag)
        })
        .or_else(|| {
            cfg.image_edit
                .language
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(normalize_lang_tag)
        })
        .or_else(|| {
            cfg.command_intent
                .default_locale
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(normalize_lang_tag)
        })
        .unwrap_or_else(|| "en-US".to_string())
}

pub(super) fn normalize_lang_tag(raw: &str) -> String {
    let lowered = raw.trim().to_ascii_lowercase();
    if lowered.starts_with("zh") || lowered.contains("cn") || lowered.contains("hans") {
        "zh-CN".to_string()
    } else {
        "en-US".to_string()
    }
}

pub(super) fn default_i18n_dict(lang: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if normalize_lang_tag(lang) == "zh-CN" {
        out.insert(
            "image_edit.msg.saved".to_string(),
            "图片编辑成功并已保存：{path}".to_string(),
        );
    } else {
        out.insert(
            "image_edit.msg.saved".to_string(),
            "Edited successfully and saved: {path}".to_string(),
        );
    }
    out
}

pub(super) fn load_external_i18n(path: &Path) -> Option<HashMap<String, String>> {
    let raw = std::fs::read_to_string(path).ok()?;
    let value = toml::from_str::<toml::Value>(&raw).ok()?;
    let dict = value.get("dict")?.as_table()?;
    let mut out = HashMap::new();
    for (k, v) in dict {
        if let Some(s) = v.as_str() {
            out.insert(k.to_string(), s.to_string());
        }
    }
    Some(out)
}
