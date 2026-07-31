use serde::{Deserialize, Serialize};

use crate::types::ChannelKind;

pub const CHANNEL_INGRESS_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelReplyTargetKind {
    Chat,
    User,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelReplyTarget {
    pub kind: ChannelReplyTargetKind,
    pub external_id: String,
}

impl ChannelReplyTarget {
    pub fn chat(external_id: impl Into<String>) -> Self {
        Self {
            kind: ChannelReplyTargetKind::Chat,
            external_id: external_id.into(),
        }
    }

    pub fn user(external_id: impl Into<String>) -> Self {
        Self {
            kind: ChannelReplyTargetKind::User,
            external_id: external_id.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelIngressAttachment {
    pub kind: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelIngressEnvelope {
    pub schema_version: u16,
    pub channel: ChannelKind,
    pub adapter: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bound_user_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation_chat_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_user_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_chat_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_target: Option<ChannelReplyTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ChannelIngressAttachment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_token: Option<String>,
}

impl ChannelIngressEnvelope {
    pub fn new(channel: ChannelKind, adapter: impl Into<String>) -> Self {
        Self {
            schema_version: CHANNEL_INGRESS_SCHEMA_VERSION,
            channel,
            adapter: adapter.into(),
            bound_user_id: None,
            conversation_chat_id: None,
            external_user_id: None,
            external_chat_id: None,
            message_id: None,
            reply_target: None,
            locale: None,
            attachments: Vec::new(),
            context_token: None,
        }
    }

    pub fn with_external_ids(
        mut self,
        external_user_id: impl Into<String>,
        external_chat_id: impl Into<String>,
    ) -> Self {
        self.external_user_id = Some(external_user_id.into());
        self.external_chat_id = Some(external_chat_id.into());
        self
    }

    pub fn with_message_id(mut self, message_id: impl Into<String>) -> Self {
        self.message_id = Some(message_id.into());
        self
    }

    pub fn with_reply_target(mut self, reply_target: ChannelReplyTarget) -> Self {
        self.reply_target = Some(reply_target);
        self
    }

    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.locale = Some(locale.into());
        self
    }

    pub fn with_context_token(mut self, context_token: impl Into<String>) -> Self {
        self.context_token = Some(context_token.into());
        self
    }
}

pub fn default_adapter_for_channel(channel: ChannelKind) -> &'static str {
    match channel {
        ChannelKind::Telegram => "telegram_bot",
        ChannelKind::Whatsapp => "whatsapp_cloud",
        ChannelKind::Ui => "web_ui",
        ChannelKind::Wechat => "wechat_ilink",
        ChannelKind::Feishu => "feishu_open_platform",
        ChannelKind::Lark => "lark_open_platform",
    }
}

pub fn default_reply_target(
    channel: ChannelKind,
    external_user_id: Option<&str>,
    external_chat_id: Option<&str>,
) -> Option<ChannelReplyTarget> {
    match channel {
        ChannelKind::Whatsapp | ChannelKind::Wechat => external_user_id
            .or(external_chat_id)
            .map(ChannelReplyTarget::user),
        ChannelKind::Telegram | ChannelKind::Ui | ChannelKind::Feishu | ChannelKind::Lark => {
            external_chat_id
                .or(external_user_id)
                .map(ChannelReplyTarget::chat)
        }
    }
}

#[cfg(test)]
#[path = "channel_ingress_tests.rs"]
mod tests;
