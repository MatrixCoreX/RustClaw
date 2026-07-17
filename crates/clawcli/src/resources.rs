use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const EN_CATALOG: &str = include_str!("../resources/en.toml");
const ZH_CN_CATALOG: &str = include_str!("../resources/zh-CN.toml");

#[derive(Debug, Deserialize)]
struct CliCatalog {
    #[serde(default)]
    messages: BTreeMap<String, String>,
    #[serde(default)]
    lists: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Copy)]
enum CatalogKind {
    En,
    ZhCn,
}

static EN: OnceLock<CliCatalog> = OnceLock::new();
static ZH_CN: OnceLock<CliCatalog> = OnceLock::new();

fn catalog(kind: CatalogKind) -> &'static CliCatalog {
    match kind {
        CatalogKind::En => EN.get_or_init(|| parse_catalog(EN_CATALOG, "en")),
        CatalogKind::ZhCn => ZH_CN.get_or_init(|| parse_catalog(ZH_CN_CATALOG, "zh-CN")),
    }
}

fn parse_catalog(raw: &str, locale: &str) -> CliCatalog {
    toml::from_str(raw).unwrap_or_else(|error| panic!("cli_catalog_invalid:{locale}:{error}"))
}

fn active_locale() -> Option<String> {
    ["RUSTCLAW_CLI_LOCALE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .find_map(|name| {
            std::env::var(name)
                .ok()
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
        })
}

fn catalog_kind(locale: Option<&str>) -> CatalogKind {
    let language = locale
        .unwrap_or_default()
        .split(['.', '@'])
        .next()
        .unwrap_or_default()
        .replace('_', "-")
        .to_ascii_lowercase()
        .split('-')
        .next()
        .unwrap_or_default()
        .to_string();
    if language == "zh" {
        CatalogKind::ZhCn
    } else {
        CatalogKind::En
    }
}

fn message_for_locale(key: &'static str, locale: Option<&str>) -> &'static str {
    let selected = catalog(catalog_kind(locale));
    selected
        .messages
        .get(key)
        .or_else(|| catalog(CatalogKind::En).messages.get(key))
        .map(String::as_str)
        .unwrap_or(key)
}

fn list_for_locale(key: &'static str, locale: Option<&str>) -> &'static [String] {
    let selected = catalog(catalog_kind(locale));
    selected
        .lists
        .get(key)
        .or_else(|| catalog(CatalogKind::En).lists.get(key))
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub(crate) fn text(key: &'static str) -> &'static str {
    let locale = active_locale();
    message_for_locale(key, locale.as_deref())
}

pub(crate) fn optional_text(key: &str) -> Option<&'static str> {
    let locale = active_locale();
    let selected = catalog(catalog_kind(locale.as_deref()));
    selected
        .messages
        .get(key)
        .or_else(|| catalog(CatalogKind::En).messages.get(key))
        .map(String::as_str)
}

pub(crate) fn lines(key: &'static str) -> &'static [String] {
    let locale = active_locale();
    list_for_locale(key, locale.as_deref())
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
