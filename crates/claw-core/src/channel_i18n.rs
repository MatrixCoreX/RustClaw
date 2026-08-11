use std::collections::HashMap;
use std::fs;
use std::sync::{Mutex, OnceLock};

use toml::Value as TomlValue;

use crate::channel_notice::{ChannelNotice, CHANNEL_NOTICE_SAFE_GENERIC_MESSAGE_KEY};
use crate::product_identity::product_identity;

static I18N_DICT_CACHE: OnceLock<Mutex<HashMap<String, HashMap<String, String>>>> = OnceLock::new();
static COMMON_I18N_DICTS: OnceLock<HashMap<&'static str, HashMap<String, String>>> =
    OnceLock::new();

const COMMON_I18N_EN_US: &str = include_str!("../../../configs/i18n/channel-common.en-US.toml");
const COMMON_I18N_ZH_CN: &str = include_str!("../../../configs/i18n/channel-common.zh-CN.toml");
const COMMON_I18N_JA: &str = include_str!("../../../configs/i18n/channel-common.ja.toml");
const COMMON_I18N_KO: &str = include_str!("../../../configs/i18n/channel-common.ko.toml");

fn load_i18n_dict(i18n_path: &str) -> HashMap<String, String> {
    let Ok(raw) = fs::read_to_string(i18n_path) else {
        return HashMap::new();
    };
    parse_i18n_dict(&raw)
}

fn parse_i18n_dict(raw: &str) -> HashMap<String, String> {
    let Ok(value) = toml::from_str::<TomlValue>(raw) else {
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

fn render_product_name(text: String) -> String {
    text.replace("{product_name}", product_identity().display_name())
}

fn lookup_text_from_path(i18n_path: &str, key: &str) -> Option<String> {
    if i18n_path.trim().is_empty() {
        return None;
    }
    let cache = I18N_DICT_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut guard = cache.lock().ok()?;
    let dict = guard
        .entry(i18n_path.to_string())
        .or_insert_with(|| load_i18n_dict(i18n_path));
    dict.get(key).cloned().map(render_product_name)
}

fn common_i18n_dicts() -> &'static HashMap<&'static str, HashMap<String, String>> {
    COMMON_I18N_DICTS.get_or_init(|| {
        HashMap::from([
            ("en-US", parse_i18n_dict(COMMON_I18N_EN_US)),
            ("zh-CN", parse_i18n_dict(COMMON_I18N_ZH_CN)),
            ("ja", parse_i18n_dict(COMMON_I18N_JA)),
            ("ko", parse_i18n_dict(COMMON_I18N_KO)),
        ])
    })
}

fn normalized_common_locale(locale: &str) -> &'static str {
    let locale = locale.trim().to_ascii_lowercase();
    if locale.starts_with("zh") {
        "zh-CN"
    } else if locale.starts_with("ja") {
        "ja"
    } else if locale.starts_with("ko") {
        "ko"
    } else {
        "en-US"
    }
}

fn locale_hint_from_path(i18n_path: &str) -> &'static str {
    let path = i18n_path.to_ascii_lowercase();
    if path.contains("zh-cn") || path.contains("zh_cn") {
        "zh-CN"
    } else if path.contains(".ja.") || path.ends_with(".ja.toml") {
        "ja"
    } else if path.contains(".ko.") || path.ends_with(".ko.toml") {
        "ko"
    } else {
        "en-US"
    }
}

pub fn safe_generic_text_for_locale(locale: &str) -> String {
    let locale = normalized_common_locale(locale);
    let dictionaries = common_i18n_dicts();
    dictionaries
        .get(locale)
        .and_then(|dict| dict.get(CHANNEL_NOTICE_SAFE_GENERIC_MESSAGE_KEY))
        .or_else(|| {
            dictionaries
                .get("en-US")
                .and_then(|dict| dict.get(CHANNEL_NOTICE_SAFE_GENERIC_MESSAGE_KEY))
        })
        .cloned()
        .map(render_product_name)
        .expect("bundled channel common i18n must define safe generic error")
}

pub fn common_text_for_locale(locale: &str, key: &str) -> String {
    let locale = normalized_common_locale(locale);
    let dictionaries = common_i18n_dicts();
    dictionaries
        .get(locale)
        .and_then(|dict| dict.get(key))
        .or_else(|| dictionaries.get("en-US").and_then(|dict| dict.get(key)))
        .filter(|text| !looks_like_machine_text(text, key))
        .cloned()
        .map(render_product_name)
        .unwrap_or_else(|| safe_generic_text_for_locale(locale))
}

pub fn common_text_with_vars_for_locale(locale: &str, key: &str, vars: &[(&str, &str)]) -> String {
    let mut text = common_text_for_locale(locale, key);
    for (name, value) in vars {
        text = text.replace(&format!("{{{name}}}"), value);
    }
    text
}

pub fn safe_generic_text_for_path(i18n_path: &str) -> String {
    safe_generic_text_for_locale(locale_hint_from_path(i18n_path))
}

fn looks_like_machine_text(text: &str, key: &str) -> bool {
    let text = text.trim();
    text == key || text.starts_with("message_key=")
}

pub fn text_from_path(i18n_path: &str, key: &str, fallback: &str) -> String {
    if let Some(text) = lookup_text_from_path(i18n_path, key) {
        if !looks_like_machine_text(&text, key) {
            return text;
        }
    }
    if looks_like_machine_text(fallback, key) {
        safe_generic_text_for_path(i18n_path)
    } else {
        render_product_name(fallback.to_string())
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelNoticeLocalizationSource {
    RequestedMessageKey,
    SafeGenericFallback,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalizedChannelNotice {
    pub text: String,
    pub resolved_message_key: String,
    pub source: ChannelNoticeLocalizationSource,
}

pub fn localize_channel_notice_from_path(
    i18n_path: &str,
    locale: &str,
    notice: &ChannelNotice,
) -> LocalizedChannelNotice {
    if notice.validate().is_ok() {
        if let Some(mut text) = lookup_text_from_path(i18n_path, &notice.message_key) {
            for (name, value) in &notice.params {
                text = text.replace(&format!("{{{name}}}"), value);
            }
            if !looks_like_machine_text(&text, &notice.message_key) {
                return LocalizedChannelNotice {
                    text,
                    resolved_message_key: notice.message_key.clone(),
                    source: ChannelNoticeLocalizationSource::RequestedMessageKey,
                };
            }
        }
    }

    LocalizedChannelNotice {
        text: safe_generic_text_for_locale(locale),
        resolved_message_key: CHANNEL_NOTICE_SAFE_GENERIC_MESSAGE_KEY.to_string(),
        source: ChannelNoticeLocalizationSource::SafeGenericFallback,
    }
}

#[cfg(test)]
#[path = "channel_i18n_tests.rs"]
mod tests;
