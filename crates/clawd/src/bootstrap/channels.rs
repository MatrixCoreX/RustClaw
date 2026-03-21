use std::path::{Path, PathBuf};

use toml::Value as TomlValue;

use crate::channel_send;

pub(crate) fn resolve_ui_dist_dir(workspace_root: &Path) -> PathBuf {
    if let Ok(raw) = std::env::var("RUSTCLAW_UI_DIST") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            let candidate = PathBuf::from(trimmed);
            if candidate.is_absolute() {
                return candidate;
            }
            return workspace_root.join(candidate);
        }
    }
    workspace_root.join("UI").join("dist")
}

pub(crate) fn load_feishu_send_config(
    workspace_root: &Path,
) -> Option<channel_send::FeishuSendConfig> {
    let path = workspace_root.join("configs/channels/feishu.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let table: TomlValue = toml::from_str(&content).ok()?;
    let feishu = table.get("feishu")?.as_table()?;
    let app_id = feishu.get("app_id")?.as_str()?.trim().to_string();
    let app_secret = feishu.get("app_secret")?.as_str()?.trim().to_string();
    if app_id.is_empty() || app_secret.is_empty() {
        return None;
    }
    let api_base_url = feishu
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://open.feishu.cn".to_string());
    Some(channel_send::FeishuSendConfig {
        app_id,
        app_secret,
        api_base_url,
    })
}

pub(crate) fn load_lark_send_config(workspace_root: &Path) -> Option<channel_send::LarkSendConfig> {
    let path = workspace_root.join("configs/channels/lark.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let table: TomlValue = toml::from_str(&content).ok()?;
    let lark = table.get("lark")?.as_table()?;
    let app_id = lark.get("app_id")?.as_str()?.trim().to_string();
    let app_secret = lark.get("app_secret")?.as_str()?.trim().to_string();
    if app_id.is_empty() || app_secret.is_empty() {
        return None;
    }
    let api_base_url = lark
        .get("api_base_url")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "https://open.larksuite.com".to_string());
    Some(channel_send::LarkSendConfig {
        app_id,
        app_secret,
        api_base_url,
    })
}
