//! Typed WeChat iLink message and conversation-scope contracts.
//!
//! The wire shapes mirror Tencent's maintained `openclaw-weixin` protocol
//! types. Provider capabilities and local upload safety limits remain separate:
//! this module describes the provider wire contract and does not claim a
//! provider-enforced byte limit.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::Rng;
use serde::Serialize;

use crate::http::BaseInfo;

pub const WECHAT_ILINK_CONTRACT_SOURCE: &str =
    "https://github.com/Tencent/openclaw-weixin/blob/main/src/api/types.ts";
pub const WECHAT_ILINK_CONTRACT_VERIFIED_AT: &str = "2026-07-31";
pub const WECHAT_ILINK_ADAPTER: &str = "wechat_ilink";

pub const UPLOAD_MEDIA_TYPE_IMAGE: i64 = 1;
pub const UPLOAD_MEDIA_TYPE_VIDEO: i64 = 2;
pub const UPLOAD_MEDIA_TYPE_FILE: i64 = 3;
pub const UPLOAD_MEDIA_TYPE_VOICE: i64 = 4;

pub const MESSAGE_TYPE_BOT: i64 = 2;
pub const MESSAGE_STATE_NEW: i64 = 0;
pub const MESSAGE_STATE_GENERATING: i64 = 1;
pub const MESSAGE_STATE_FINISH: i64 = 2;

pub const MESSAGE_ITEM_TEXT: i64 = 1;
pub const MESSAGE_ITEM_IMAGE: i64 = 2;
pub const MESSAGE_ITEM_VOICE: i64 = 3;
pub const MESSAGE_ITEM_FILE: i64 = 4;
pub const MESSAGE_ITEM_VIDEO: i64 = 5;

pub const TYPING_STATUS_TYPING: i64 = 1;
pub const TYPING_STATUS_CANCEL: i64 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WechatConversationScope {
    account_id: String,
    channel: String,
    peer_id: String,
}

impl WechatConversationScope {
    pub fn new(account_id: &str, channel: &str, peer_id: &str) -> Result<Self, String> {
        Ok(Self {
            account_id: required_scope_part("account_id", account_id)?,
            channel: required_scope_part("channel", channel)?,
            peer_id: required_scope_part("peer_id", peer_id)?,
        })
    }

    pub fn wechat_ilink(account_id: &str, peer_id: &str) -> Result<Self, String> {
        Self::new(account_id, WECHAT_ILINK_ADAPTER, peer_id)
    }

    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    pub fn channel(&self) -> &str {
        &self.channel
    }

    pub fn peer_id(&self) -> &str {
        &self.peer_id
    }

    /// Collision-resistant, non-secret key for caches and conversation IDs.
    pub fn storage_key(&self) -> String {
        let payload = serde_json::json!({
            "schema_version": 1,
            "account_id": self.account_id,
            "channel": self.channel,
            "peer_id": self.peer_id,
        });
        format!(
            "wechat-scope-v1:{}",
            URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes())
        )
    }
}

fn required_scope_part(name: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("wechat_scope_{name}_missing"));
    }
    if value.len() > 512 {
        return Err(format!("wechat_scope_{name}_too_long"));
    }
    Ok(value.to_string())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatCdnMedia {
    pub encrypt_query_param: String,
    pub aes_key: String,
    pub encrypt_type: i64,
}

impl WechatCdnMedia {
    pub fn encrypted(encrypt_query_param: String, aes_key: String) -> Result<Self, String> {
        if encrypt_query_param.trim().is_empty() {
            return Err("wechat_cdn_encrypt_query_param_missing".to_string());
        }
        if aes_key.trim().is_empty() {
            return Err("wechat_cdn_aes_key_missing".to_string());
        }
        Ok(Self {
            encrypt_query_param,
            aes_key,
            encrypt_type: 1,
        })
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatTextItem {
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatImageItem {
    pub media: WechatCdnMedia,
    pub mid_size: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatVoiceItem {
    pub media: WechatCdnMedia,
    pub encode_type: i64,
    pub sample_rate: i64,
    pub playtime: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatFileItem {
    pub media: WechatCdnMedia,
    pub file_name: String,
    pub len: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatVideoItem {
    pub media: WechatCdnMedia,
    pub video_size: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatMessageItem {
    #[serde(rename = "type")]
    pub item_type: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_item: Option<WechatTextItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_item: Option<WechatImageItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub voice_item: Option<WechatVoiceItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_item: Option<WechatFileItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video_item: Option<WechatVideoItem>,
}

impl WechatMessageItem {
    pub fn text(text: impl Into<String>) -> Result<Self, String> {
        let text = text.into();
        if text.is_empty() {
            return Err("wechat_text_item_empty".to_string());
        }
        Ok(Self {
            item_type: MESSAGE_ITEM_TEXT,
            text_item: Some(WechatTextItem { text }),
            image_item: None,
            voice_item: None,
            file_item: None,
            video_item: None,
        })
    }

    pub fn image(media: WechatCdnMedia, ciphertext_size: usize) -> Result<Self, String> {
        Ok(Self {
            item_type: MESSAGE_ITEM_IMAGE,
            text_item: None,
            image_item: Some(WechatImageItem {
                media,
                mid_size: checked_size(ciphertext_size)?,
            }),
            voice_item: None,
            file_item: None,
            video_item: None,
        })
    }

    pub fn voice_silk(
        media: WechatCdnMedia,
        sample_rate: i64,
        playtime_ms: i64,
    ) -> Result<Self, String> {
        if sample_rate <= 0 || playtime_ms <= 0 {
            return Err("wechat_voice_metadata_invalid".to_string());
        }
        Ok(Self {
            item_type: MESSAGE_ITEM_VOICE,
            text_item: None,
            image_item: None,
            voice_item: Some(WechatVoiceItem {
                media,
                encode_type: 6,
                sample_rate,
                playtime: playtime_ms,
            }),
            file_item: None,
            video_item: None,
        })
    }

    pub fn file(
        media: WechatCdnMedia,
        file_name: impl Into<String>,
        plaintext_size: usize,
    ) -> Result<Self, String> {
        let file_name = file_name.into();
        if file_name.trim().is_empty() {
            return Err("wechat_file_name_missing".to_string());
        }
        Ok(Self {
            item_type: MESSAGE_ITEM_FILE,
            text_item: None,
            image_item: None,
            voice_item: None,
            file_item: Some(WechatFileItem {
                media,
                file_name,
                len: plaintext_size.to_string(),
            }),
            video_item: None,
        })
    }

    pub fn video(media: WechatCdnMedia, ciphertext_size: usize) -> Result<Self, String> {
        Ok(Self {
            item_type: MESSAGE_ITEM_VIDEO,
            text_item: None,
            image_item: None,
            voice_item: None,
            file_item: None,
            video_item: Some(WechatVideoItem {
                media,
                video_size: checked_size(ciphertext_size)?,
            }),
        })
    }
}

fn checked_size(size: usize) -> Result<i64, String> {
    i64::try_from(size).map_err(|_| "wechat_media_size_overflow".to_string())
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatOutboundMessage {
    pub from_user_id: String,
    pub to_user_id: String,
    pub client_id: String,
    pub message_type: i64,
    pub message_state: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_list: Option<Vec<WechatMessageItem>>,
    pub context_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WechatSendMessageRequest {
    pub msg: WechatOutboundMessage,
    pub base_info: BaseInfo,
}

impl WechatSendMessageRequest {
    pub fn generating(
        to_user_id: &str,
        context_token: &str,
        client_id: impl Into<String>,
        run_id: impl Into<String>,
        channel_version: &str,
    ) -> Result<Self, String> {
        Self::build(
            to_user_id,
            context_token,
            client_id.into(),
            Some(run_id.into()),
            MESSAGE_STATE_GENERATING,
            None,
            channel_version,
        )
    }

    pub fn generating_with_item(
        to_user_id: &str,
        context_token: &str,
        client_id: impl Into<String>,
        run_id: impl Into<String>,
        item: WechatMessageItem,
        channel_version: &str,
    ) -> Result<Self, String> {
        Self::build(
            to_user_id,
            context_token,
            client_id.into(),
            Some(run_id.into()),
            MESSAGE_STATE_GENERATING,
            Some(vec![item]),
            channel_version,
        )
    }

    pub fn finish(
        to_user_id: &str,
        context_token: &str,
        client_id: impl Into<String>,
        run_id: Option<String>,
        item: WechatMessageItem,
        channel_version: &str,
    ) -> Result<Self, String> {
        Self::build(
            to_user_id,
            context_token,
            client_id.into(),
            run_id,
            MESSAGE_STATE_FINISH,
            Some(vec![item]),
            channel_version,
        )
    }

    fn build(
        to_user_id: &str,
        context_token: &str,
        client_id: String,
        run_id: Option<String>,
        message_state: i64,
        item_list: Option<Vec<WechatMessageItem>>,
        channel_version: &str,
    ) -> Result<Self, String> {
        let to_user_id = required_wire_value("to_user_id", to_user_id)?;
        let context_token = required_wire_value("context_token", context_token)?;
        let client_id = required_wire_value("client_id", &client_id)?;
        let run_id = run_id
            .map(|value| required_wire_value("run_id", &value))
            .transpose()?;
        Ok(Self {
            msg: WechatOutboundMessage {
                from_user_id: String::new(),
                to_user_id,
                client_id,
                message_type: MESSAGE_TYPE_BOT,
                message_state,
                item_list,
                context_token,
                run_id,
            },
            base_info: BaseInfo {
                channel_version: channel_version.to_string(),
            },
        })
    }
}

fn required_wire_value(name: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(format!("wechat_{name}_missing"));
    }
    Ok(value.to_string())
}

pub fn new_wechat_client_id(label: &str) -> String {
    let suffix = hex::encode(rand::thread_rng().gen::<[u8; 8]>());
    format!("wechat-ilink-{}-{suffix}", label.trim().replace(' ', "-"))
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
