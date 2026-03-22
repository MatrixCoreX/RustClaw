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

pub(crate) fn load_wechat_send_config(
    workspace_root: &Path,
) -> Option<channel_send::WechatSendConfig> {
    let path = workspace_root.join("configs/channels/wechat.toml");
    let content = std::fs::read_to_string(&path).ok()?;
    let table: TomlValue = toml::from_str(&content).ok()?;
    let wechat = table.get("wechat")?.as_table()?;
    let enabled = wechat
        .get("enabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !enabled {
        return None;
    }
    let api_base_url = wechat.get("api_base_url")?.as_str()?.trim().to_string();
    let bot_token = wechat
        .get("bot_token")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    if api_base_url.is_empty() || bot_token.is_empty() {
        return None;
    }
    let wechat_uin_base64 = wechat
        .get("wechat_uin_base64")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let text_chunk_chars = wechat
        .get("text_chunk_chars")
        .and_then(|v| v.as_integer())
        .map(|v| v.max(1) as usize)
        .unwrap_or(1200);
    let sk_route_tag = wechat
        .get("sk_route_tag")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    Some(channel_send::WechatSendConfig {
        api_base_url,
        bot_token,
        wechat_uin_base64,
        text_chunk_chars,
        sk_route_tag,
    })
}
